//! As rotas: o que a página do jogo pede ao servidor oficial, servido do iPhone.
//!
//!   /                     pacote/web/index.html (a página, do mesmo build do módulo)
//!   /<arquivo>            pacote/web/<arquivo> (scripts, workers, estilos, ícones, idiomas)
//!   /game.json            pacote/game.json (o nfsu2.json do build) com "module"
//!   /game.wasm            pacote/game.wasm
//!   /data-manifest        o manifesto da pasta do jogo (manifest.rs)
//!   /data/<caminho>       um arquivo do jogo, com Range
//!   /data-blocks/v2/<c>   blocos de 1 MiB de um arquivo do jogo (?b=primeiro&n=quantos, até 8), sem compressão
//!   /players, /lan        a ponte de multiplayer (ponte.rs): repassados ao servidor escolhido nos Ajustes; 503 sem
//!                         nenhum (a página entra no modo offline dela)
//!   /sw.js                404: no modo local não há service worker (o servidor está sempre aqui)
//!   /__diag/              a página de diagnóstico do app (WebGPU, isolamento, OPFS, ...), embutida
//!   /__status             JSON com o estado do servidor (para o app)
//!
//! Tudo com COOP/COEP/CORP (isolamento entre origens: SharedArrayBuffer e as threads do jogo) e sem cache (os
//! arquivos estão no próprio aparelho; um cache só guardaria cópias velhas depois de trocar o pacote).

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::http::{parse_range, Conn, Range, Request};
use crate::manifest::{self, BLOCK, BLOCKS_MAX};

/// os cabeçalhos de toda resposta
pub const COMMON_HEADERS: &[(&str, &str)] = &[
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
    ("Cache-Control", "no-store"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
];

const DIAG_HTML: &str = include_str!("diag/index.html");
const DIAG_JS: &str = include_str!("diag/diag.js");
const DIAG_WORKER: &str = include_str!("diag/opfs-worker.js");
const DIAG_WORKLET: &str = include_str!("diag/worklet.js");

pub struct Config {
    /// a pasta do pacote (web/, game.wasm, game.json); vazia: sem pacote
    pub pack: PathBuf,
    /// a pasta do jogo (SPEED2.EXE e as pastas de dados); vazia: sem jogo
    pub game: PathBuf,
    /// a porta (o Host aceito é 127.0.0.1:porta ou localhost:porta)
    pub port: u16,
    /// o manifesto feito por último (feito no primeiro pedido)
    manifest: Mutex<Option<std::sync::Arc<manifest::Manifest>>>,
    /// a identidade do módulo (para /game.json "module"), feita no primeiro pedido
    module_id: Mutex<Option<String>>,
    /// o servidor desta configuração foi desligado: as conexões que ainda estão abertas fecham antes do próximo
    /// pedido (o WebKit repete o pedido numa conexão nova, que já é do servidor religado)
    pub retired: std::sync::atomic::AtomicBool,
}

impl Config {
    pub fn new(pack: PathBuf, game: PathBuf, port: u16) -> Config {
        Config {
            pack,
            game,
            port,
            manifest: Mutex::new(None),
            module_id: Mutex::new(None),
            retired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn same_dirs(&self, pack: &Path, game: &Path) -> bool {
        self.pack == pack && self.game == game
    }

    fn manifest(&self, refresh: bool) -> io::Result<std::sync::Arc<manifest::Manifest>> {
        let mut m = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        if refresh || m.is_none() {
            *m = Some(std::sync::Arc::new(manifest::build(&self.game)?));
        }
        Ok(m.as_ref().unwrap().clone())
    }

    fn module_id(&self) -> Option<String> {
        let mut id = self.module_id.lock().unwrap_or_else(|e| e.into_inner());
        if id.is_none() {
            *id = module_identity(&self.pack.join("game.wasm"));
        }
        id.clone()
    }
}

/// Um id curto do módulo: FNV-1a de 64 bits sobre o tamanho, o mtime e os primeiros e últimos 64 KB.  Só serve de
/// nome (o módulo vai sem cache); um módulo trocado tem outro.
fn module_identity(p: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let f = File::open(p).ok()?;
    let md = f.metadata().ok()?;
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&md.size().to_le_bytes());
    eat(&md.mtime().to_le_bytes());
    eat(&md.mtime_nsec().to_le_bytes());
    let mut buf = vec![0u8; 64 << 10];
    let n = read_full_at(&f, &mut buf, 0);
    eat(&buf[..n]);
    if md.size() > buf.len() as u64 {
        let tail = md.size() - buf.len() as u64;
        let n = read_full_at(&f, &mut buf, tail);
        eat(&buf[..n]);
    }
    Some(format!("{h:016x}"))
}

fn read_full_at(f: &File, buf: &mut [u8], off: u64) -> usize {
    let mut got = 0;
    while got < buf.len() {
        match f.read_at(&mut buf[got..], off + got as u64) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    got
}

/// O tipo pela extensão (os da página e do diagnóstico)
fn content_type(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// `rel` ("a/b/c") dentro de `root`, ou None se algum segmento é vazio, ".", "..", começa com ponto ou tem um
/// separador; e o caminho real (links resolvidos) tem que continuar dentro do caminho real da raiz.
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    if root.as_os_str().is_empty() {
        return None;
    }
    let mut p = root.to_path_buf();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.') || seg.contains('\\') || seg.contains('\0') {
            return None;
        }
        p.push(seg);
    }
    // o caminho real tem que ser exatamente a raiz real + rel: nenhum link simbólico no meio do caminho, nem para
    // dentro da pasta (o oficial abre só caminhos sem links; o manifesto nunca lista um)
    let real = p.canonicalize().ok()?;
    let mut want = root.canonicalize().ok()?;
    for seg in rel.split('/') {
        want.push(seg);
    }
    (real == want).then_some(real)
}

/// Atende um pedido.  Devolve se a conexão continua (keep-alive).
pub fn serve(cfg: &Config, q: &Request, c: &mut Conn) -> io::Result<bool> {
    let keep = q.keep_alive();
    // HTTP/1.1 precisa de Host; e só os nomes deste servidor (rebinding de DNS: uma página de outro site que
    // apontasse um nome seu para 127.0.0.1 não chega aos arquivos)
    let host = q.header("host").unwrap_or("");
    if q.http11 && host.is_empty() {
        return plain(c, q, 400, "Bad request", false);
    }
    if !host.is_empty() && !own_host(host, cfg.port) {
        return plain(c, q, 421, "Misdirected request", false);
    }
    if q.path.is_empty() {
        return plain(c, q, 400, "Bad request", false);
    }
    if q.method != "GET" && q.method != "HEAD" {
        let h = [("Allow", "GET, HEAD".to_string())];
        c.write_head(405, &h, Some(0), false)?;
        return Ok(false);
    }
    // nada aqui recebe corpo
    let has_body = q.header("content-length").is_some_and(|v| v.trim() != "0") || q.header("transfer-encoding").is_some();
    if has_body {
        return plain(c, q, 413, "Too large", false);
    }
    let p = q.path.as_str();
    match p {
        "/__status" => return status(cfg, q, c, keep),
        "/__diag" => return redirect(c, q, "/__diag/", keep),
        "/__diag/" | "/__diag/index.html" => return text(c, q, 200, "text/html; charset=utf-8", DIAG_HTML, keep),
        "/__diag/diag.js" => return text(c, q, 200, "text/javascript; charset=utf-8", DIAG_JS, keep),
        "/__diag/opfs-worker.js" => return text(c, q, 200, "text/javascript; charset=utf-8", DIAG_WORKER, keep),
        "/__diag/worklet.js" => return text(c, q, 200, "text/javascript; charset=utf-8", DIAG_WORKLET, keep),
        "/game.json" => return game_json(cfg, q, c, keep),
        "/game.wasm" => {
            let f = cfg.pack.join("game.wasm");
            return file(c, q, &f, Some("application/wasm"), false, keep);
        }
        "/data-manifest" => return data_manifest(cfg, q, c, keep),
        // o multiplayer: repassado ao servidor escolhido nos Ajustes (404 quando nenhum, e a página esconde)
        "/players" => return crate::ponte::players(q, c, keep),
        "/lan" => {
            // só a nossa página (um WebSocket de outra origem nunca chega aqui no app, mas não custa)
            if q.header("origin").is_some_and(|o| !own_origin(o, cfg.port)) {
                return plain(c, q, 403, "Forbidden", false);
            }
            return crate::ponte::lan(q, c, &cfg.retired);
        }
        "/sw.js" | "/vault" | "/names" | "/robots.txt" => return plain(c, q, 404, "Not found", keep),
        _ => {}
    }
    if let Some(rest) = p.strip_prefix("/data/") {
        return match game_file(cfg, rest) {
            Some(f) => file(c, q, &f, Some("application/octet-stream"), true, keep),
            None => plain(c, q, 404, "Not found", keep),
        };
    }
    if let Some(rest) = p.strip_prefix("/data-blocks/") {
        // (os endereços sem o v2/ também: os mesmos arquivos, como no oficial)
        let rest = rest.strip_prefix("v2/").unwrap_or(rest);
        return blocks(cfg, q, c, rest, keep);
    }
    // a página: pacote/web
    let web = cfg.pack.join("web");
    let rel = if p == "/" { "index.html".to_string() } else if p.ends_with('/') { format!("{}index.html", &p[1..]) } else { p[1..].to_string() };
    match safe_join(&web, &rel) {
        Some(f) if f.is_file() => file(c, q, &f, None, false, keep),
        _ if p == "/" => no_pack(c, q, keep),
        _ => plain(c, q, 404, "Not found", keep),
    }
}

/// "127.0.0.1:porta", "localhost:porta" (e [::1]:porta)
fn own_host(host: &str, port: u16) -> bool {
    let h = host.to_ascii_lowercase();
    let want = |name: &str| h == format!("{name}:{port}");
    want("127.0.0.1") || want("localhost") || want("[::1]")
}

fn own_origin(origin: &str, port: u16) -> bool {
    origin.to_ascii_lowercase().strip_prefix("http://").is_some_and(|h| own_host(h, port))
}

fn game_file(cfg: &Config, rest: &str) -> Option<PathBuf> {
    let first = rest.split('/').next()?;
    if !manifest::game_entry(first) {
        return None;
    }
    safe_join(&cfg.game, rest).filter(|f| f.is_file())
}

fn plain(c: &mut Conn, q: &Request, status: u16, body: &str, keep: bool) -> io::Result<bool> {
    text(c, q, status, "text/plain; charset=utf-8", body, keep)
}

fn text(c: &mut Conn, q: &Request, status: u16, ty: &str, body: &str, keep: bool) -> io::Result<bool> {
    bytes(c, q, status, ty, body.as_bytes(), keep)
}

fn bytes(c: &mut Conn, q: &Request, status: u16, ty: &str, body: &[u8], keep: bool) -> io::Result<bool> {
    let h = [("Content-Type", ty.to_string())];
    c.write_head(status, &h, Some(body.len() as u64), keep)?;
    if q.method != "HEAD" {
        c.write_body(body)?;
    }
    Ok(keep)
}

fn redirect(c: &mut Conn, q: &Request, to: &str, keep: bool) -> io::Result<bool> {
    let h = [("Location", to.to_string()), ("Content-Type", "text/plain; charset=utf-8".to_string())];
    c.write_head(302, &h, Some(0), keep)?;
    let _ = q;
    Ok(keep)
}

/// a página que aparece sem pacote instalado (o app normalmente nem abre o modo local assim)
fn no_pack(c: &mut Conn, q: &Request, keep: bool) -> io::Result<bool> {
    let body = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
        <title>NFSU2</title><body style='background:#0d0d0d;color:#ddd;font:16px -apple-system,sans-serif;padding:24px'>\
        <h2>Pacote do jogo não instalado</h2><p>Importe o pacote (página + módulo) e a pasta do jogo pelo app.</p>\
        <p><a style='color:#c8f088' href='/__diag/'>Diagnóstico</a></p>";
    text(c, q, 404, "text/html; charset=utf-8", body, keep)
}

fn game_json(cfg: &Config, q: &Request, c: &mut Conn, keep: bool) -> io::Result<bool> {
    let Ok(raw) = std::fs::read_to_string(cfg.pack.join("game.json")) else {
        return plain(c, q, 404, "Not found", keep);
    };
    let t = raw.trim();
    let body = match (t.strip_prefix('{'), cfg.module_id()) {
        // {"pages":...}: o "module" entra primeiro (o oficial manda o hash do módulo nesse campo)
        (Some(rest), Some(id)) if !t.contains("\"module\"") => {
            let rest = rest.trim_start();
            if rest.starts_with('}') {
                format!("{{\"module\":\"{id}\"}}")
            } else {
                format!("{{\"module\":\"{id}\",{rest}")
            }
        }
        _ => t.to_string(),
    };
    bytes(c, q, 200, "application/json", body.as_bytes(), keep)
}

fn data_manifest(cfg: &Config, q: &Request, c: &mut Conn, keep: bool) -> io::Result<bool> {
    if cfg.game.as_os_str().is_empty() || !cfg.game.is_dir() {
        return plain(c, q, 404, "Not found", keep);
    }
    match cfg.manifest(q.has_param("refresh")) {
        Ok(m) => bytes(c, q, 200, "application/json", &m.body, keep),
        Err(e) => plain(c, q, 500, &format!("manifest: {e}"), keep),
    }
}

fn status(cfg: &Config, q: &Request, c: &mut Conn, keep: bool) -> io::Result<bool> {
    let web = cfg.pack.join("web").join("index.html").is_file();
    let wasm = cfg.pack.join("game.wasm").is_file();
    let gj = cfg.pack.join("game.json").is_file();
    let (files, total, exe) = if cfg.game.is_dir() {
        match cfg.manifest(false) {
            Ok(m) => (m.files, m.bytes, m.has_exe),
            Err(_) => (0, 0, false),
        }
    } else {
        (0, 0, false)
    };
    let mut s = String::from("{");
    s.push_str(&format!("\"ok\":true,\"port\":{},", cfg.port));
    s.push_str(&format!("\"pack\":{{\"page\":{web},\"module\":{wasm},\"info\":{gj}}},"));
    s.push_str(&format!("\"game\":{{\"files\":{files},\"bytes\":{total},\"exe\":{exe}}}"));
    s.push('}');
    bytes(c, q, 200, "application/json", s.as_bytes(), keep)
}

/// Um arquivo inteiro (ou o intervalo do Range, quando `ranges`).
fn file(c: &mut Conn, q: &Request, path: &Path, ty: Option<&str>, ranges: bool, keep: bool) -> io::Result<bool> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return plain(c, q, 404, "Not found", keep),
    };
    let md = f.metadata()?;
    if !md.is_file() {
        return plain(c, q, 404, "Not found", keep);
    }
    let size = md.len();
    let ty = ty.unwrap_or_else(|| content_type(path)).to_string();
    let range = if ranges { parse_range(q.header("range"), size) } else { Range::Full };
    let mut h: Vec<(&str, String)> = vec![("Content-Type", ty)];
    if ranges {
        h.push(("Accept-Ranges", "bytes".to_string()));
    }
    let (status, start, end) = match range {
        Range::Full => (200, 0, size),
        Range::Part(a, b) => {
            h.push(("Content-Range", format!("bytes {}-{}/{}", a, b - 1, size)));
            (206, a, b)
        }
        Range::Unsatisfiable => {
            h.push(("Content-Range", format!("bytes */{size}")));
            c.write_head(416, &h, Some(0), keep)?;
            return Ok(keep);
        }
    };
    c.write_head(status, &h, Some(end - start), keep)?;
    if q.method != "HEAD" {
        send_range(c, &f, start, end)?;
    }
    Ok(keep)
}

/// Os bytes [start, end) do arquivo para a conexão.  Um arquivo que encolheu no meio (trocado durante o envio)
/// fecha a conexão: o Content-Length já prometido não seria cumprido.
fn send_range(c: &mut Conn, f: &File, start: u64, end: u64) -> io::Result<()> {
    let mut buf = vec![0u8; 256 << 10];
    let mut off = start;
    while off < end {
        let want = ((end - off) as usize).min(buf.len());
        let n = read_full_at(f, &mut buf[..want], off);
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        c.write_body(&buf[..n])?;
        off += n as u64;
    }
    Ok(())
}

/// /data-blocks/<caminho>?b=<primeiro>&n=<quantos>: os blocos b .. b+n-1 (1 MiB cada, o último menor) num 200
fn blocks(cfg: &Config, q: &Request, c: &mut Conn, rest: &str, keep: bool) -> io::Result<bool> {
    let Some(path) = game_file(cfg, rest) else { return plain(c, q, 404, "Not found", keep) };
    let digits = |x: &Option<String>, max: usize| x.as_ref().is_some_and(|x| !x.is_empty() && x.len() <= max && x.bytes().all(|c| c.is_ascii_digit()));
    let (b, n) = (q.param("b"), q.param("n"));
    if !digits(&b, 7) || !digits(&n, 2) {
        return plain(c, q, 400, "Bad request", keep);
    }
    let b: u64 = b.unwrap().parse().unwrap_or(0);
    let n: u64 = n.unwrap().parse().unwrap_or(0);
    if !(1..=BLOCKS_MAX).contains(&n) {
        return plain(c, q, 400, "Bad request", keep);
    }
    let f = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return plain(c, q, 404, "Not found", keep),
    };
    let size = f.metadata()?.len();
    let (start, end) = (b * BLOCK, size.min((b + n) * BLOCK));
    if start >= size {
        return plain(c, q, 416, "Range not satisfiable", keep);
    }
    let h = [("Content-Type", "application/octet-stream".to_string())];
    c.write_head(200, &h, Some(end - start), keep)?;
    if q.method != "HEAD" {
        send_range(c, &f, start, end)?;
    }
    Ok(keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(own_host("127.0.0.1:8123", 8123));
        assert!(own_host("LOCALHOST:8123", 8123));
        assert!(!own_host("127.0.0.1:8124", 8123));
        assert!(!own_host("evil.example:8123", 8123));
        assert!(!own_host("127.0.0.1", 8123));
    }

    #[test]
    fn joins() {
        let d = std::env::temp_dir().join(format!("nfsu2srv-join-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::write(d.join("a/b.txt"), "x").unwrap();
        assert!(safe_join(&d, "a/b.txt").is_some());
        assert!(safe_join(&d, "a/../a/b.txt").is_none());
        assert!(safe_join(&d, "a/.b").is_none());
        assert!(safe_join(&d, "").is_none());
        assert!(safe_join(&d, "a//b.txt").is_none());
        assert!(safe_join(Path::new(""), "a/b.txt").is_none());
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join(format!("nfsu2srv-out-{}", std::process::id()));
            std::fs::write(&outside, "secret").unwrap();
            let _ = std::os::unix::fs::symlink(&outside, d.join("a/link"));
            assert!(safe_join(&d, "a/link").is_none(), "um link para fora da raiz nunca é servido");
            let _ = std::os::unix::fs::symlink(d.join("a/b.txt"), d.join("a/dentro"));
            assert!(safe_join(&d, "a/dentro").is_none(), "nem um link para dentro dela");
            let _ = std::os::unix::fs::symlink(&d, std::env::temp_dir().join(format!("nfsu2srv-rootlink-{}", std::process::id())));
            let via = std::env::temp_dir().join(format!("nfsu2srv-rootlink-{}", std::process::id()));
            assert!(safe_join(&via, "a/b.txt").is_some(), "a raiz pode estar atrás de um link (/var -> /private/var no iOS)");
            let _ = std::fs::remove_file(&via);
            let _ = std::fs::remove_file(&outside);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
