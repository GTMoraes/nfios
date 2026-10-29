//! A ponte de multiplayer: a página continua vindo do iPhone (modo offline), e só o multiplayer sai para um servidor
//! de verdade (o nfsu2.xyz ou um servidor próprio), escolhido nos Ajustes do app.
//!
//!   /lan      WebSocket: a página conecta em ws://127.0.0.1:<porta>/lan e a ponte abre <remoto>/lan (wss em https),
//!             repete o Sec-WebSocket-Key da página (então o Sec-WebSocket-Accept do remoto é o que a página espera),
//!             devolve o 101 e daí em diante só copia bytes nos dois sentidos: os quadros da página já vêm mascarados
//!             (como um cliente tem que mandar) e os do servidor sem máscara (como a página espera receber).
//!   /players  GET e HEAD repassados ao remoto (a contagem de jogadores, as crews).
//!
//! A conexão ao remoto não manda Origin: o servidor oficial aceita clientes sem Origin ("tools send no Origin",
//! web/server-rs/src/app/mod.rs own_page) e recusa Origin de outro site, que é o que um navegador mandaria.
//! TLS com rustls + ring e as raízes da Mozilla (webpki-roots): não depende de nada do sistema.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::http::{Conn, Request};

/// o servidor de multiplayer escolhido: http(s)://host[:porta][/caminho]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remoto {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// prefixo do caminho, sem a barra do fim ("" para a raiz)
    pub base: String,
}

impl Remoto {
    /// o Host que vai no pedido (a porta só quando não é a padrão)
    fn host_header(&self) -> String {
        let padrao = if self.tls { 443 } else { 80 };
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == padrao { h } else { format!("{h}:{}", self.port) }
    }

    pub fn url(&self) -> String {
        format!("{}://{}{}", if self.tls { "https" } else { "http" }, self.host_header(), self.base)
    }
}

pub fn parse_remoto(url: &str) -> Result<Remoto, String> {
    let u = url.trim();
    let (tls, resto) = if let Some(r) = u.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = u.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("endereço sem http:// ou https://: {u}"));
    };
    let (autoridade, caminho) = match resto.find('/') {
        Some(i) => (&resto[..i], &resto[i..]),
        None => (resto, ""),
    };
    if autoridade.is_empty() || autoridade.contains('@') || caminho.contains('?') || caminho.contains('#') {
        return Err(format!("endereço inválido: {u}"));
    }
    let (host, port) = if let Some(r) = autoridade.strip_prefix('[') {
        let (h, depois) = r.split_once(']').ok_or_else(|| format!("endereço inválido: {u}"))?;
        let p = match depois.strip_prefix(':') {
            Some(p) => p.parse::<u16>().map_err(|_| format!("porta inválida: {u}"))?,
            None if depois.is_empty() => if tls { 443 } else { 80 },
            None => return Err(format!("endereço inválido: {u}")),
        };
        (h.to_string(), p)
    } else {
        match autoridade.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| format!("porta inválida: {u}"))?),
            None => (autoridade.to_string(), if tls { 443 } else { 80 }),
        }
    };
    if host.is_empty() || port == 0 || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) {
        return Err(format!("endereço inválido: {u}"));
    }
    Ok(Remoto { tls, host: host.to_ascii_lowercase(), port, base: caminho.trim_end_matches('/').to_string() })
}

static REMOTO: Mutex<Option<Remoto>> = Mutex::new(None);
/// só para testes: uma raiz a mais (o certificado do servidor TLS falso)
static RAIZ_EXTRA: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Escolhe o servidor de multiplayer ("" ou None: nenhum; /lan e /players respondem 503).
pub fn definir(url: Option<&str>) -> Result<(), String> {
    let r = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(parse_remoto(u)?),
        None => None,
    };
    *REMOTO.lock().unwrap_or_else(|e| e.into_inner()) = r;
    Ok(())
}

pub fn atual() -> Option<Remoto> {
    REMOTO.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[doc(hidden)]
pub fn testar_raiz_extra(der: Option<Vec<u8>>) {
    *RAIZ_EXTRA.lock().unwrap_or_else(|e| e.into_inner()) = der;
}

// ------------------------------------------------------------------------------------------- conexão ao remoto --

enum Lado {
    Puro(TcpStream),
    Tls(Box<rustls::ClientConnection>, TcpStream),
}

impl Lado {
    fn sock(&self) -> &TcpStream {
        match self {
            Lado::Puro(s) => s,
            Lado::Tls(_, s) => s,
        }
    }
}

impl Read for Lado {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Lado::Puro(s) => s.read(b),
            Lado::Tls(c, s) => match rustls::Stream::new(c.as_mut(), s).read(b) {
                // o servidor fechou o TCP sem close_notify: para HTTP com Connection: close, é o fim
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                r => r,
            },
        }
    }
}

impl Write for Lado {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Lado::Puro(s) => s.write(b),
            Lado::Tls(c, s) => rustls::Stream::new(c.as_mut(), s).write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Lado::Puro(s) => s.flush(),
            Lado::Tls(c, s) => rustls::Stream::new(c.as_mut(), s).flush(),
        }
    }
}

fn config_tls() -> Arc<rustls::ClientConfig> {
    let mut raizes = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(der) = RAIZ_EXTRA.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        let _ = raizes.add(rustls::pki_types::CertificateDer::from(der));
    }
    let provedor = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provedor)
        .with_safe_default_protocol_versions()
        .expect("versões de TLS do ring")
        .with_root_certificates(raizes)
        .with_no_client_auth();
    Arc::new(cfg)
}

fn conectar(r: &Remoto) -> io::Result<Lado> {
    let mut ultimo = io::Error::new(io::ErrorKind::NotFound, format!("{} não resolveu", r.host));
    let mut sock = None;
    for a in (r.host.as_str(), r.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, Duration::from_secs(8)) {
            Ok(s) => {
                sock = Some(s);
                break;
            }
            Err(e) => ultimo = e,
        }
    }
    let s = sock.ok_or(ultimo)?;
    crate::no_sigpipe(&s);
    let _ = s.set_nodelay(true);
    s.set_read_timeout(Some(Duration::from_secs(15)))?;
    s.set_write_timeout(Some(Duration::from_secs(15)))?;
    if !r.tls {
        return Ok(Lado::Puro(s));
    }
    let nome = rustls::pki_types::ServerName::try_from(r.host.clone())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nome de servidor inválido para TLS"))?;
    let mut c = rustls::ClientConnection::new(config_tls(), nome).map_err(io::Error::other)?;
    c.set_buffer_limit(None);
    Ok(Lado::Tls(Box::new(c), s))
}

/// lê a resposta até o fim do cabeçalho; devolve (status, cabeçalhos em minúsculas, bytes que vieram depois)
fn ler_cabecalho(l: &mut Lado) -> io::Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    let mut b = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    loop {
        if let Some(i) = b.windows(4).position(|w| w == b"\r\n\r\n") {
            let cab = String::from_utf8_lossy(&b[..i]).into_owned();
            let resto = b[i + 4..].to_vec();
            let mut linhas = cab.split("\r\n");
            let status = linhas
                .next()
                .and_then(|l| l.split(' ').nth(1))
                .and_then(|s| s.parse::<u16>().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "resposta HTTP inválida do servidor"))?;
            let h = linhas.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
            return Ok((status, h, resto));
        }
        if b.len() > 64 << 10 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "cabeçalho grande demais"));
        }
        let n = l.read(&mut tmp)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "o servidor fechou a conexão"));
        }
        b.extend_from_slice(&tmp[..n]);
    }
}

fn cab<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
    h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

/// corpo com Transfer-Encoding: chunked
fn des_chunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(i) = b.windows(2).position(|w| w == b"\r\n") else { break };
        let tam = std::str::from_utf8(&b[..i]).ok().and_then(|s| usize::from_str_radix(s.split(';').next()?.trim(), 16).ok());
        let Some(tam) = tam else { break };
        if tam == 0 || b.len() < i + 2 + tam {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + tam]);
        b = &b[(i + 4 + tam).min(b.len())..];
    }
    out
}

// --------------------------------------------------------------------------------------------------- /players --

/// GET/HEAD /players?... repassado ao remoto, com o status, o tipo e o corpo dele
pub fn players(q: &Request, c: &mut Conn, keep: bool) -> io::Result<bool> {
    // sem servidor: 503, que a página lê como "o servidor não responde" e entra no modo offline dela (sem tentar o
    // /lan); um 404 ela leria como servidor no ar (web/src/app/players.ts, serverAnswers: status < 500)
    let Some(r) = atual() else { return simples(c, q, 503, "nenhum servidor de multiplayer escolhido", keep) };
    let alvo = format!("{}/players{}", r.base, if q.query.is_empty() { String::new() } else { format!("?{}", q.query) });
    let res = (|| -> io::Result<(u16, Option<String>, Vec<u8>)> {
        let mut l = conectar(&r)?;
        let pedido = format!(
            "{} {alvo} HTTP/1.1\r\nHost: {}\r\nUser-Agent: nfsu2-ios\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
            q.method,
            r.host_header()
        );
        l.write_all(pedido.as_bytes())?;
        l.flush()?;
        let (status, h, mut corpo) = ler_cabecalho(&mut l)?;
        if q.method != "HEAD" {
            let limite: usize = cab(&h, "content-length").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
            let mut tmp = [0u8; 8192];
            while corpo.len() < limite && corpo.len() < 1 << 20 {
                let n = l.read(&mut tmp)?;
                if n == 0 {
                    break;
                }
                corpo.extend_from_slice(&tmp[..n]);
            }
            if cab(&h, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
                corpo = des_chunk(&corpo);
            }
            corpo.truncate(limite.min(corpo.len()));
        } else {
            corpo.clear();
        }
        Ok((status, cab(&h, "content-type").map(str::to_string), corpo))
    })();
    match res {
        Ok((status, tipo, corpo)) => {
            let h = [("Content-Type", tipo.unwrap_or_else(|| "application/json".into()))];
            c.write_head(status, &h, Some(corpo.len() as u64), keep)?;
            if q.method != "HEAD" {
                c.write_body(&corpo)?;
            }
            Ok(keep)
        }
        // sem resposta do servidor: 502 (a página trata como "servidor não respondeu")
        Err(e) => simples(c, q, 502, &format!("servidor de multiplayer: {e}"), keep),
    }
}

fn simples(c: &mut Conn, q: &Request, status: u16, texto: &str, keep: bool) -> io::Result<bool> {
    let h = [("Content-Type", "text/plain; charset=utf-8".to_string())];
    c.write_head(status, &h, Some(texto.len() as u64), keep)?;
    if q.method != "HEAD" {
        c.write_body(texto.as_bytes())?;
    }
    Ok(keep)
}

// -------------------------------------------------------------------------------------------------------- /lan --

/// O WebSocket do multiplayer: atende a página e liga ao remoto até um dos dois fechar (ou o servidor ser desligado).
/// Sempre fecha a conexão no fim.
pub fn lan(q: &Request, c: &mut Conn, retired: &AtomicBool) -> io::Result<bool> {
    let upgrade = q.header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let chave = q.header("sec-websocket-key").unwrap_or("").to_string();
    let Some(r) = atual() else { return simples(c, q, 503, "nenhum servidor de multiplayer escolhido", false) };
    if !upgrade || chave.is_empty() || q.header("sec-websocket-version") != Some("13") {
        return simples(c, q, 400, "Bad request", false);
    }
    let alvo = format!("{}/lan{}", r.base, if q.query.is_empty() { String::new() } else { format!("?{}", q.query) });
    let protocolo = q.header("sec-websocket-protocol").map(str::to_string);
    let ligado = (|| -> io::Result<(u16, Vec<(String, String)>, Vec<u8>, Lado)> {
        let mut l = conectar(&r)?;
        let mut pedido = format!(
            "GET {alvo} HTTP/1.1\r\nHost: {}\r\nUser-Agent: nfsu2-ios\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {chave}\r\nSec-WebSocket-Version: 13\r\n",
            r.host_header()
        );
        if let Some(p) = &protocolo {
            pedido.push_str(&format!("Sec-WebSocket-Protocol: {p}\r\n"));
        }
        pedido.push_str("\r\n");
        l.write_all(pedido.as_bytes())?;
        l.flush()?;
        let (status, h, resto) = ler_cabecalho(&mut l)?;
        Ok((status, h, resto, l))
    })();
    let (status, h, resto, remoto) = match ligado {
        Ok(x) => x,
        Err(e) => return simples(c, q, 502, &format!("servidor de multiplayer: {e}"), false),
    };
    if status != 101 {
        // a recusa do remoto (cheio, banido, muitas conexões): o mesmo status para a página
        return simples(c, q, status, "o servidor de multiplayer recusou a conexão", false);
    }
    let aceite = cab(&h, "sec-websocket-accept").unwrap_or("").to_string();
    let mut resposta = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {aceite}\r\n");
    if let Some(p) = cab(&h, "sec-websocket-protocol") {
        resposta.push_str(&format!("Sec-WebSocket-Protocol: {p}\r\n"));
    }
    resposta.push_str("\r\n");
    c.stream.write_all(resposta.as_bytes())?;
    bombear(&c.stream, remoto, resto, retired);
    Ok(false)
}

/// até ~4 MB esperando para um lado antes de parar de ler do outro (o WebSocket é confiável: nada se descarta)
const LIMITE: usize = 4 << 20;

fn escrever_o_que_der(s: &TcpStream, fila: &mut Vec<u8>) -> io::Result<()> {
    let mut w = s;
    while !fila.is_empty() {
        match w.write(fila) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                fila.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Copia bytes nos dois sentidos, num laço só (poll nos dois sockets): o TLS do rustls não se divide em duas threads.
fn bombear(pagina: &TcpStream, mut remoto: Lado, mut para_pagina: Vec<u8>, retired: &AtomicBool) {
    let _ = pagina.set_nonblocking(true);
    let _ = remoto.sock().set_nonblocking(true);
    let mut para_remoto: Vec<u8> = Vec::new(); // (só no remoto sem TLS; no TLS o rustls guarda)
    let mut buf = vec![0u8; 64 << 10];
    let mut remoto_aberto = true;
    loop {
        if retired.load(Ordering::SeqCst) {
            break;
        }
        // o que o TLS já decifrou
        if let Lado::Tls(conn, _) = &mut remoto {
            loop {
                match conn.reader().read(&mut buf) {
                    Ok(0) => {
                        remoto_aberto = false;
                        break;
                    }
                    Ok(n) => para_pagina.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        remoto_aberto = false;
                        break;
                    }
                }
            }
        }
        if escrever_o_que_der(pagina, &mut para_pagina).is_err() {
            break;
        }
        let pendente_remoto = match &mut remoto {
            Lado::Puro(s) => {
                if escrever_o_que_der(s, &mut para_remoto).is_err() {
                    break;
                }
                !para_remoto.is_empty()
            }
            Lado::Tls(conn, s) => {
                let mut falhou = false;
                while conn.wants_write() {
                    match conn.write_tls(s) {
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            falhou = true;
                            break;
                        }
                    }
                }
                if falhou {
                    break;
                }
                conn.wants_write()
            }
        };
        if !remoto_aberto && para_pagina.is_empty() {
            break;
        }
        let espera_remoto = match &remoto {
            Lado::Puro(_) => para_remoto.len(),
            Lado::Tls(..) => 0,
        };
        let mut fds = [
            libc::pollfd {
                fd: pagina.as_raw_fd(),
                events: (if espera_remoto < LIMITE { libc::POLLIN } else { 0 }) | (if para_pagina.is_empty() { 0 } else { libc::POLLOUT }),
                revents: 0,
            },
            libc::pollfd {
                fd: remoto.sock().as_raw_fd(),
                events: (if remoto_aberto && para_pagina.len() < LIMITE { libc::POLLIN } else { 0 })
                    | (if pendente_remoto { libc::POLLOUT } else { 0 }),
                revents: 0,
            },
        ];
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 1000) };
        if n < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            continue;
        }
        let ruim = libc::POLLERR | libc::POLLNVAL;
        if fds[0].revents & ruim != 0 {
            break;
        }
        // da página para o remoto
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut p = pagina;
            match p.read(&mut buf) {
                Ok(0) => break,
                Ok(k) => match &mut remoto {
                    Lado::Puro(_) => para_remoto.extend_from_slice(&buf[..k]),
                    Lado::Tls(conn, _) => {
                        if conn.writer().write_all(&buf[..k]).is_err() {
                            break;
                        }
                    }
                },
                Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        // do remoto para a página
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            match &mut remoto {
                Lado::Puro(s) => match s.read(&mut buf) {
                    Ok(0) => remoto_aberto = false,
                    Ok(k) => para_pagina.extend_from_slice(&buf[..k]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => remoto_aberto = false,
                },
                Lado::Tls(conn, s) => match conn.read_tls(s) {
                    Ok(0) => remoto_aberto = false,
                    Ok(_) => {
                        if conn.process_new_packets().is_err() {
                            remoto_aberto = false;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => remoto_aberto = false,
                },
            }
        }
    }
    let _ = pagina.shutdown(std::net::Shutdown::Both);
    let _ = remoto.sock().shutdown(std::net::Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enderecos() {
        let r = parse_remoto("https://nfsu2.xyz").unwrap();
        assert_eq!((r.tls, r.host.as_str(), r.port, r.base.as_str()), (true, "nfsu2.xyz", 443, ""));
        assert_eq!(r.url(), "https://nfsu2.xyz");
        let r = parse_remoto("http://100.91.24.1:9091/").unwrap();
        assert_eq!((r.tls, r.host.as_str(), r.port, r.base.as_str()), (false, "100.91.24.1", 9091, ""));
        assert_eq!(r.host_header(), "100.91.24.1:9091");
        let r = parse_remoto(" HTTPS://Jogo.Exemplo.com:8443/nfsu2/ ".to_lowercase().as_str()).unwrap();
        assert_eq!((r.port, r.base.as_str()), (8443, "/nfsu2"));
        let r = parse_remoto("http://[::1]:8080").unwrap();
        assert_eq!((r.host.as_str(), r.port, r.host_header()), ("::1", 8080, "[::1]:8080".to_string()));
        for ruim in ["nfsu2.xyz", "ftp://x", "https://", "https://a:b", "https://u@x", "https://x/?a", "https://x y", "http://x:0"] {
            assert!(parse_remoto(ruim).is_err(), "{ruim}");
        }
    }

    #[test]
    fn chunked() {
        assert_eq!(des_chunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), b"Wikipedia");
        assert_eq!(des_chunk(b"3;x=1\r\nabc\r\n0\r\n\r\n"), b"abc");
    }
}
