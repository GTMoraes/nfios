//! O contrato HTTP que a página do jogo espera (o do servidor oficial, web/server-rs), testado com pedidos reais num
//! servidor de verdade: pastas falsas de pacote e de jogo, porta qualquer de 127.0.0.1.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// os testes mexem no servidor global (um por processo): um de cada vez
static SERIAL: Mutex<()> = Mutex::new(());

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn h(&self, k: &str) -> Option<&str> {
        let k = k.to_ascii_lowercase();
        self.headers.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str())
    }
}

/// lê uma resposta inteira (Content-Length) da conexão; `head`: pedido HEAD (sem corpo)
fn read_resp(s: &mut TcpStream, head: bool) -> Option<Resp> {
    let mut buf = Vec::new();
    let mut one = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match s.read(&mut one) {
            Ok(0) => return None,
            Ok(_) => buf.push(one[0]),
            Err(_) => return None,
        }
    }
    let text = String::from_utf8(buf).unwrap();
    let mut lines = text.split("\r\n");
    let status: u16 = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.to_ascii_lowercase(), v.trim().to_string())
        })
        .collect();
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").map(|(_, v)| v.parse().unwrap()).unwrap_or(0);
    let mut body = vec![0u8; if head { 0 } else { len }];
    s.read_exact(&mut body).ok()?;
    Some(Resp { status, headers, body })
}

fn conn(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s
}

fn req_on(s: &mut TcpStream, port: u16, method: &str, target: &str, extra: &[(&str, &str)]) -> Option<Resp> {
    let mut r = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in extra {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    r.push_str("\r\n");
    s.write_all(r.as_bytes()).ok()?;
    read_resp(s, method == "HEAD")
}

fn get(port: u16, target: &str, extra: &[(&str, &str)]) -> Resp {
    req_on(&mut conn(port), port, "GET", target, extra).expect("resposta")
}

fn raw(port: u16, bytes: &[u8]) -> Option<Resp> {
    let mut s = conn(port);
    s.write_all(bytes).unwrap();
    read_resp(&mut s, false)
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

fn set_mtime(p: &Path, secs: i64, nsec: i64) {
    let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    let ts = [libc::timespec { tv_sec: secs, tv_nsec: nsec }, libc::timespec { tv_sec: secs, tv_nsec: nsec }];
    assert_eq!(unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), ts.as_ptr(), 0) }, 0);
}

struct Fixture {
    root: PathBuf,
    pack: PathBuf,
    game: PathBuf,
    wasm: Vec<u8>,
    big: Vec<u8>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// o pacote e a pasta do jogo falsos (a pasta segue a do teste do manifesto do oficial, mais um arquivo grande)
fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("nfsu2srv-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let pack = root.join("Pacote");
    let game = root.join("Jogo");
    std::fs::create_dir_all(pack.join("web/sub")).unwrap();
    std::fs::create_dir_all(pack.join("web/lang")).unwrap();
    std::fs::write(pack.join("web/index.html"), "<!doctype html><title>pagina</title>").unwrap();
    std::fs::write(pack.join("web/main.js"), "console.log(1)").unwrap();
    std::fs::write(pack.join("web/main.css"), "body{}").unwrap();
    std::fs::write(pack.join("web/sub/index.html"), "sub").unwrap();
    std::fs::write(pack.join("web/lang/pt.json"), "{}").unwrap();
    std::fs::write(pack.join("web/.escondido"), "SECRET").unwrap();
    let wasm = pattern(200_000, 7);
    std::fs::write(pack.join("game.wasm"), &wasm).unwrap();
    std::fs::write(
        pack.join("game.json"),
        "{\"pages\":16384,\"maxPages\":65536,\"guestEnd\":671088640,\"rev\":\"abc1234\",\"size\":200000,\"opt\":\"-O3\",\"lift\":\"FLAGABI\"}\n",
    )
    .unwrap();
    let p = |r: &str| game.join(r);
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(p("SPEED2.EXE"), "MZ fake").unwrap();
    for dir in ["GLOBAL", "TRACKS/sub", "SAVE", "scripts", "Movies"] {
        std::fs::create_dir_all(p(dir)).unwrap();
    }
    std::fs::write(p("GLOBAL/Game.txt"), "inside").unwrap();
    std::fs::write(p("GLOBAL/b.bin"), [0u8; 1000]).unwrap();
    std::fs::write(p("GLOBAL/B.bin"), [0u8; 10]).unwrap();
    std::fs::write(p("GLOBAL/_x"), "x").unwrap();
    std::fs::write(p("GLOBAL/\u{e9}t\u{e9}"), "\u{e9}").unwrap();
    std::fs::write(p("GLOBAL/a\"q.txt"), "q").unwrap();
    std::fs::write(p("GLOBAL/.hidden"), "SECRET").unwrap();
    std::fs::write(p(".hidden"), "SECRET").unwrap();
    std::fs::write(p("TRACKS/sub/L4RA.BUN"), pattern(3000, 3)).unwrap();
    std::fs::write(p("Movies/INTRO.VP6"), [2u8; 50]).unwrap();
    std::fs::write(p("unins000.dat"), "SECRET").unwrap();
    std::fs::write(p("SAVE/profile"), "SECRET").unwrap();
    std::fs::write(p("scripts/mod.ini"), "SECRET").unwrap();
    std::os::unix::fs::symlink(p("GLOBAL/Game.txt"), p("GLOBAL/link.txt")).unwrap();
    std::os::unix::fs::symlink(p("SAVE"), p("GLOBAL/dir-link")).unwrap();
    let big = pattern(3 * (1 << 20) + 512 * 1024, 11);
    std::fs::write(p("TRACKS/STREAML4RA.BUN"), &big).unwrap();
    set_mtime(&p("SPEED2.EXE"), 1_700_000_000, 999_999_999);
    set_mtime(&p("GLOBAL/Game.txt"), 1_760_000_123, 456_789_012);
    Fixture { root, pack, game, wasm, big }
}

fn start(f: &Fixture) -> u16 {
    nfsu2srv::stop();
    nfsu2srv::start(&f.pack, &f.game, 0).expect("servidor ligado")
}

const ISOLATION: [(&str, &str); 3] = [
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-embedder-policy", "require-corp"),
    ("cross-origin-resource-policy", "same-origin"),
];

fn isolated(r: &Resp) {
    for (k, v) in ISOLATION {
        assert_eq!(r.h(k), Some(v), "{k}");
    }
    assert_eq!(r.h("cache-control"), Some("no-store"));
}

#[test]
fn the_page_and_its_files() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("page");
    let port = start(&f);
    let r = get(port, "/", &[]);
    assert_eq!(r.status, 200);
    assert_eq!(r.h("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.body, b"<!doctype html><title>pagina</title>");
    isolated(&r);
    let r = get(port, "/main.js", &[]);
    assert_eq!((r.status, r.h("content-type")), (200, Some("text/javascript; charset=utf-8")));
    isolated(&r);
    assert_eq!(get(port, "/main.css", &[]).h("content-type"), Some("text/css; charset=utf-8"));
    assert_eq!(get(port, "/lang/pt.json", &[]).h("content-type"), Some("application/json"));
    assert_eq!(get(port, "/sub/", &[]).body, b"sub");
    assert_eq!(get(port, "/.escondido", &[]).status, 404);
    assert_eq!(get(port, "/nada.js", &[]).status, 404);
    // o módulo, e o game.json com o "module" que a página usa no endereço do módulo
    let r = get(port, "/game.wasm", &[]);
    assert_eq!((r.status, r.h("content-type")), (200, Some("application/wasm")));
    assert_eq!(r.body, f.wasm);
    isolated(&r);
    let r = get(port, "/game.json", &[]);
    assert_eq!(r.status, 200);
    let j = String::from_utf8(r.body).unwrap();
    assert!(j.starts_with("{\"module\":\""), "{j}");
    assert!(j.contains("\"rev\":\"abc1234\"") && j.contains("\"pages\":16384") && j.ends_with('}'), "{j}");
    let id = &j[11..27];
    assert!(id.bytes().all(|c| c.is_ascii_hexdigit()), "{id}");
    assert_eq!(get(port, &format!("/game.wasm?v={id}"), &[]).body, f.wasm);
    assert_eq!(get(port, "/game.json?latest", &[]).status, 200);
    // o que o modo local não tem: 404 (sem service worker, sem cofre de saves, sem nomes)
    for p in ["/sw.js", "/vault", "/names"] {
        assert_eq!(get(port, p, &[]).status, 404, "{p}");
    }
    // sem servidor de multiplayer escolhido (ponte.rs): 503, o "servidor não responde" que põe a página no modo
    // offline dela, sem tentar o /lan
    for p in ["/players", "/players?crews=1,2", "/lan"] {
        assert_eq!(get(port, p, &[]).status, 503, "{p}");
    }
    let mut s = conn(port);
    let r = req_on(&mut s, port, "HEAD", "/players", &[]).unwrap();
    assert!(r.status >= 500 && r.body.is_empty());
    // HEAD: os cabeçalhos de um GET, sem corpo; a conexão continua
    let r = req_on(&mut s, port, "HEAD", "/game.wasm", &[]).unwrap();
    assert_eq!((r.status, r.h("content-length")), (200, Some("200000")));
    let r = req_on(&mut s, port, "GET", "/main.js", &[]).unwrap();
    assert_eq!(r.body, b"console.log(1)");
    // o diagnóstico embutido e o estado
    let r = get(port, "/__diag/", &[]);
    assert_eq!(r.status, 200);
    assert!(String::from_utf8_lossy(&r.body).contains("/__diag/diag.js"));
    isolated(&r);
    for p in ["/__diag/diag.js", "/__diag/opfs-worker.js", "/__diag/worklet.js"] {
        let r = get(port, p, &[]);
        assert_eq!((r.status, r.h("content-type")), (200, Some("text/javascript; charset=utf-8")), "{p}");
    }
    assert_eq!(get(port, "/__diag", &[]).status, 302);
    let s = String::from_utf8(get(port, "/__status", &[]).body).unwrap();
    assert!(s.contains("\"page\":true,\"module\":true,\"info\":true"), "{s}");
    assert!(s.contains("\"exe\":true"), "{s}");
    nfsu2srv::stop();
}

/// a mesma lista que o teste do manifesto do oficial espera, e os mesmos bytes que o walk de referência em Node
#[test]
fn the_manifest_is_the_official_ones() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("manifest");
    let port = start(&f);
    let r = get(port, "/data-manifest", &[]);
    assert_eq!((r.status, r.h("content-type")), (200, Some("application/json")));
    isolated(&r);
    let body = String::from_utf8(r.body).unwrap();
    // os caminhos, na ordem
    let ps: Vec<String> = body.split("{\"p\":\"").skip(1).map(|x| x[..x.find("\",").unwrap()].replace("\\\"", "\"")).collect();
    let want = vec![
        "GLOBAL",
        "GLOBAL/B.bin",
        "GLOBAL/Game.txt",
        "GLOBAL/_x",
        "GLOBAL/a\"q.txt",
        "GLOBAL/b.bin",
        "GLOBAL/\u{e9}t\u{e9}",
        "Movies",
        "Movies/INTRO.VP6",
        "SPEED2.EXE",
        "TRACKS",
        "TRACKS/STREAML4RA.BUN",
        "TRACKS/sub",
        "TRACKS/sub/L4RA.BUN",
    ];
    assert_eq!(ps, want);
    assert!(body.contains("{\"p\":\"SPEED2.EXE\",\"s\":7,\"m\":1700000001}"), "{body}");
    assert!(body.contains("{\"p\":\"GLOBAL/Game.txt\",\"s\":6,\"m\":1760000123}"), "{body}");
    assert!(body.ends_with("],\"blocks\":{\"url\":\"/data-blocks/v2/\",\"size\":1048576,\"max\":8}}"), "{body}");
    assert!(!body.contains("SECRET") && !body.contains("SAVE") && !body.contains("unins") && !body.contains("link"));
    // o walk de referência (o buildManifest do servidor em Node, que o oficial também usa para conferir o seu)
    let script = r#"
import fsp from 'node:fs/promises'; import path from 'node:path';
const GAME = new Set(['speed2.exe','server.dll','server.cfg','cars','credits','frontend','global','langpacks','languages',
  'memcard','movies','nis','pfdata','sound','speech','subtitles','texpacks','tracks']);
const files = [];
async function walk(dir, rel) {
  for (const e of await fsp.readdir(dir, { withFileTypes: true })) {
    if (e.name.startsWith('.') || (!rel && !GAME.has(e.name.toLowerCase()))) continue;
    const abs = path.join(dir, e.name), r = rel ? rel + '/' + e.name : e.name;
    if (e.isDirectory()) { files.push({ p: r, d: 1 }); await walk(abs, r); }
    else if (e.isFile()) { const st = await fsp.stat(abs); files.push({ p: r, s: st.size, m: Math.floor(st.mtimeMs / 1000) }); }
  }
}
await walk(process.env.DATA, '');
process.stdout.write(JSON.stringify({ v: 1, files, blocks: { url: '/data-blocks/v2/', size: 1 << 20, max: 8 } }));
"#;
    match std::process::Command::new("node").args(["--input-type=module", "-e", script]).env("DATA", &f.game).output() {
        Ok(o) if o.status.success() => assert_eq!(String::from_utf8_lossy(&o.stdout), body),
        _ => panic!("sem node para a conferência de referência"),
    }
    // ?refresh refaz (um arquivo novo aparece)
    std::fs::write(f.game.join("TRACKS/NOVO.BIN"), "n").unwrap();
    assert!(!String::from_utf8(get(port, "/data-manifest", &[]).body).unwrap().contains("NOVO"));
    assert!(String::from_utf8(get(port, "/data-manifest?refresh", &[]).body).unwrap().contains("TRACKS/NOVO.BIN"));
    nfsu2srv::stop();
}

#[test]
fn game_files_and_ranges() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("ranges");
    let port = start(&f);
    let small = std::fs::read(f.game.join("TRACKS/sub/L4RA.BUN")).unwrap();
    let r = get(port, "/data/TRACKS/sub/L4RA.BUN", &[]);
    assert_eq!((r.status, r.h("content-type"), r.h("accept-ranges")), (200, Some("application/octet-stream"), Some("bytes")));
    assert_eq!(r.body, small);
    isolated(&r);
    let r = get(port, "/data/TRACKS/sub/L4RA.BUN", &[("Range", "bytes=10-19")]);
    assert_eq!((r.status, r.h("content-range")), (206, Some("bytes 10-19/3000")));
    assert_eq!(r.body, &small[10..20]);
    let r = get(port, "/data/TRACKS/sub/L4RA.BUN", &[("Range", "bytes=2990-")]);
    assert_eq!((r.status, r.body.as_slice()), (206, &small[2990..]));
    let r = get(port, "/data/TRACKS/sub/L4RA.BUN", &[("Range", "bytes=-5")]);
    assert_eq!((r.status, r.h("content-range")), (206, Some("bytes 2995-2999/3000")));
    let r = get(port, "/data/TRACKS/sub/L4RA.BUN", &[("Range", "bytes=3000-3010")]);
    assert_eq!((r.status, r.h("content-range"), r.body.len()), (416, Some("bytes */3000"), 0));
    // o io-worker pede 1 MiB de cada vez de um arquivo grande
    let r = get(port, "/data/TRACKS/STREAML4RA.BUN", &[("Range", "bytes=1048576-2097151")]);
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &f.big[1 << 20..2 << 20]);
    // nomes com acento e aspas, codificados como a página codifica (encodeURIComponent por segmento)
    assert_eq!(get(port, "/data/GLOBAL/%C3%A9t%C3%A9", &[]).body, "\u{e9}".as_bytes());
    assert_eq!(get(port, "/data/GLOBAL/a%22q.txt", &[]).body, b"q");
    // nada fora do que é do jogo
    for p in [
        "/data/SAVE/profile",
        "/data/unins000.dat",
        "/data/scripts/mod.ini",
        "/data/.hidden",
        "/data/GLOBAL/.hidden",
        "/data/GLOBAL/link.txt",
        "/data/GLOBAL/dir-link/profile",
        "/data/TRACKS",
        "/data/nada",
    ] {
        assert_eq!(get(port, p, &[]).status, 404, "{p}");
    }
    for p in ["/data/GLOBAL/..%2f..%2fx", "/data/GLOBAL/../SAVE/profile", "/data/GLOBAL/%2e%2e/SAVE/profile", "/data//GLOBAL/_x"] {
        assert_eq!(get(port, p, &[]).status, 400, "{p}");
    }
    nfsu2srv::stop();
}

#[test]
fn data_blocks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("blocks");
    let port = start(&f);
    const MB: usize = 1 << 20;
    let u = |q: &str| format!("/data-blocks/v2/TRACKS/STREAML4RA.BUN?{q}");
    let r = get(port, &u("b=1&n=2&m=1&s=3670016"), &[]);
    assert_eq!((r.status, r.h("content-type")), (200, Some("application/octet-stream")));
    assert_eq!(r.body, &f.big[MB..3 * MB]);
    isolated(&r);
    // o último bloco é menor; pedir além do fim é o fim do arquivo
    let r = get(port, &u("b=3&n=8"), &[]);
    assert_eq!(r.body, &f.big[3 * MB..]);
    assert_eq!(get(port, &u("b=0&n=1"), &[]).body, &f.big[..MB]);
    assert_eq!(get(port, &u("b=4&n=1"), &[]).status, 416);
    for q in ["b=0&n=0", "b=0&n=9", "b=x&n=1", "n=1", "b=0", "b=12345678&n=1", "b=0&n=001"] {
        assert_eq!(get(port, &u(q), &[]).status, 400, "{q}");
    }
    // o endereço sem v2/ (de antes) também
    assert_eq!(get(port, "/data-blocks/TRACKS/STREAML4RA.BUN?b=2&n=1", &[]).body, &f.big[2 * MB..3 * MB]);
    assert_eq!(get(port, "/data-blocks/v2/SAVE/profile?b=0&n=1", &[]).status, 404);
    nfsu2srv::stop();
}

#[test]
fn requests_it_refuses() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("refuse");
    let port = start(&f);
    // outro nome (rebinding de DNS) e HTTP/1.1 sem Host
    assert_eq!(raw(port, b"GET / HTTP/1.1\r\nHost: evil.example\r\n\r\n").unwrap().status, 421);
    assert_eq!(raw(port, format!("GET / HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n").as_bytes()).unwrap().status, 421);
    assert_eq!(raw(port, b"GET / HTTP/1.1\r\n\r\n").unwrap().status, 400);
    assert_eq!(raw(port, format!("GET / HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).unwrap().status, 200);
    assert_eq!(raw(port, b"GET / HTTP/1.0\r\n\r\n").unwrap().status, 200);
    // métodos e corpos
    let r = raw(port, format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n").as_bytes()).unwrap();
    assert_eq!((r.status, r.h("allow")), (405, Some("GET, HEAD")));
    let r = raw(port, format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 5\r\n\r\nhello").as_bytes()).unwrap();
    assert_eq!(r.status, 413);
    // lixo, e um cabeçalho grande demais
    assert_eq!(raw(port, b"HELLO\r\n\r\n").unwrap().status, 400);
    let mut big = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX: ").into_bytes();
    big.extend(std::iter::repeat_n(b'a', 40_000));
    big.extend(b"\r\n\r\n");
    assert_eq!(raw(port, &big).map(|r| r.status), Some(431));
    nfsu2srv::stop();
}

#[test]
fn keep_alive_and_pipelining() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("pipe");
    let port = start(&f);
    let mut s = conn(port);
    // dois pedidos no mesmo pacote: duas respostas, na ordem
    let two = format!(
        "GET /main.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\nGET /data/TRACKS/sub/L4RA.BUN HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=0-3\r\n\r\n"
    );
    s.write_all(two.as_bytes()).unwrap();
    let a = read_resp(&mut s, false).unwrap();
    let b = read_resp(&mut s, false).unwrap();
    assert_eq!((a.status, a.body.as_slice()), (200, b"console.log(1)".as_slice()));
    assert_eq!((b.status, b.body.len()), (206, 4));
    assert_eq!(a.h("connection"), Some("keep-alive"));
    // vinte na mesma conexão
    for i in 0..20 {
        let r = req_on(&mut s, port, "GET", "/data/TRACKS/STREAML4RA.BUN", &[("Range", &format!("bytes={}-{}", i * 1000, i * 1000 + 99))]).unwrap();
        assert_eq!(r.body, &f.big[i * 1000..i * 1000 + 100]);
    }
    // Connection: close fecha depois da resposta
    let r = req_on(&mut s, port, "GET", "/", &[("Connection", "close")]).unwrap();
    assert_eq!(r.h("connection"), Some("close"));
    let mut rest = Vec::new();
    assert_eq!(s.read_to_end(&mut rest).unwrap_or(0), 0);
    nfsu2srv::stop();
}

#[test]
fn many_at_once() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("many");
    let port = start(&f);
    let big = std::sync::Arc::new(f.big.clone());
    let threads: Vec<_> = (0..16)
        .map(|t| {
            let big = big.clone();
            std::thread::spawn(move || {
                let mut s = conn(port);
                let mut x = 12345u64 + t;
                for _ in 0..25 {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let b = (x >> 33) % 4;
                    let n = 1 + (x >> 40) % 8;
                    let r = req_on(&mut s, port, "GET", &format!("/data-blocks/v2/TRACKS/STREAML4RA.BUN?b={b}&n={n}"), &[]).unwrap();
                    let start = (b as usize) << 20;
                    let end = big.len().min(((b + n) as usize) << 20);
                    assert_eq!(r.body, &big[start..end]);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    nfsu2srv::stop();
}

#[test]
fn start_stop_and_the_same_port_again() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("restart");
    let port = start(&f);
    assert!(nfsu2srv::alive());
    assert_eq!(nfsu2srv::port(), port);
    // de novo com as mesmas pastas e porta: nada muda (a conexão aberta segue servindo)
    let mut old = conn(port);
    assert_eq!(req_on(&mut old, port, "GET", "/main.js", &[]).unwrap().status, 200);
    assert_eq!(nfsu2srv::start(&f.pack, &f.game, port).unwrap(), port);
    assert_eq!(req_on(&mut old, port, "GET", "/main.js", &[]).unwrap().status, 200);
    // desligado: a conexão antiga fecha sem responder ao próximo pedido; o novo servidor, na mesma porta, atende
    nfsu2srv::stop();
    assert!(!nfsu2srv::alive());
    assert_eq!(nfsu2srv::port(), 0);
    assert_eq!(nfsu2srv::start(&f.pack, &f.game, port).unwrap(), port);
    assert!(req_on(&mut old, port, "GET", "/main.js", &[]).is_none(), "a conexão do servidor antigo fecha");
    assert_eq!(get(port, "/main.js", &[]).status, 200);
    // o iOS fechou o socket de escuta com o app suspenso: o próximo start nota (a conexão de teste falha) e religa
    nfsu2srv::simular_socket_fechado_pelo_sistema();
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "o socket derrubado recusa conexões");
    assert_eq!(nfsu2srv::start(&f.pack, &f.game, port).unwrap(), port);
    assert_eq!(get(port, "/main.js", &[]).status, 200, "religado na mesma porta");
    assert!(nfsu2srv::alive());
    // outras pastas na mesma porta: religa com elas
    let g = fixture("restart2");
    std::fs::write(g.pack.join("web/index.html"), "outra").unwrap();
    assert_eq!(nfsu2srv::start(&g.pack, &g.game, port).unwrap(), port);
    assert_eq!(get(port, "/", &[]).body, b"outra");
    // sem pacote: a raiz explica, o diagnóstico continua
    nfsu2srv::stop();
    let port2 = nfsu2srv::start(Path::new(""), Path::new(""), 0).unwrap();
    let r = get(port2, "/", &[]);
    assert_eq!(r.status, 404);
    assert!(String::from_utf8_lossy(&r.body).contains("Pacote do jogo"));
    assert_eq!(get(port2, "/__diag/", &[]).status, 200);
    assert_eq!(get(port2, "/data-manifest", &[]).status, 404);
    assert_eq!(get(port2, "/game.json", &[]).status, 404);
    assert!(String::from_utf8(get(port2, "/__status", &[]).body).unwrap().contains("\"page\":false"));
    nfsu2srv::stop();
    // a porta ocupada por outro: erro com o motivo
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = other.local_addr().unwrap().port();
    let t0 = std::time::Instant::now();
    assert!(nfsu2srv::start(&f.pack, &f.game, busy).is_err());
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(nfsu2srv::last_error().contains(&busy.to_string()), "{}", nfsu2srv::last_error());
}

#[test]
fn the_c_api() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture("capi");
    nfsu2srv::stop();
    let pack = std::ffi::CString::new(f.pack.to_str().unwrap()).unwrap();
    let game = std::ffi::CString::new(f.game.to_str().unwrap()).unwrap();
    let port = nfsu2srv::nfsu2srv_start(pack.as_ptr(), game.as_ptr(), 0);
    assert!(port > 0);
    assert_eq!(nfsu2srv::nfsu2srv_alive(), 1);
    assert_eq!(nfsu2srv::nfsu2srv_port(), port);
    assert_eq!(get(port as u16, "/main.js", &[]).status, 200);
    nfsu2srv::nfsu2srv_stop();
    assert_eq!(nfsu2srv::nfsu2srv_alive(), 0);
    // o erro: porta ocupada
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = other.local_addr().unwrap().port();
    assert_eq!(nfsu2srv::nfsu2srv_start(pack.as_ptr(), game.as_ptr(), busy), -1);
    let mut buf = [0 as std::ffi::c_char; 8];
    let n = nfsu2srv::nfsu2srv_last_error(buf.as_mut_ptr(), buf.len() as i32);
    assert!(n > 8, "a mensagem inteira tem {n} bytes");
    assert_eq!(buf[7], 0, "cortada com o NUL no fim");
    let mut full = vec![0 as std::ffi::c_char; n as usize + 1];
    nfsu2srv::nfsu2srv_last_error(full.as_mut_ptr(), full.len() as i32);
    let s = unsafe { std::ffi::CStr::from_ptr(full.as_ptr()) }.to_string_lossy().into_owned();
    assert!(s.contains(&busy.to_string()), "{s}");
    // ponteiros nulos: sem pacote e sem jogo, mas liga
    let p = nfsu2srv::nfsu2srv_start(std::ptr::null(), std::ptr::null(), 0);
    assert!(p > 0);
    assert_eq!(get(p as u16, "/__diag/", &[]).status, 200);
    nfsu2srv::nfsu2srv_stop();
}
