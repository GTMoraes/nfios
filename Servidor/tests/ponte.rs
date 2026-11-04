//! A ponte de multiplayer (src/ponte.rs) contra servidores remotos falsos: WebSocket em ws:// e wss:// (TLS com um
//! certificado de teste), o /players repassado, e as falhas (nenhum servidor, servidor fora do ar, recusa).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// o servidor local e o remoto escolhido são globais: um teste de cada vez
static SERIAL: Mutex<()> = Mutex::new(());

fn ligar_local() -> u16 {
    let base = std::env::temp_dir().join(format!("nfsu2-ponte-{}", std::process::id()));
    let (pack, game) = (base.join("pack"), base.join("game"));
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::create_dir_all(&game).unwrap();
    nfsu2srv::start(&pack, &game, 0).unwrap()
}

/// lê até o fim do cabeçalho (byte a byte: nada do que vem depois é consumido)
fn ler_cabecalho<R: Read>(s: &mut R) -> String {
    let mut b = Vec::new();
    let mut um = [0u8; 1];
    while !b.ends_with(b"\r\n\r\n") {
        if s.read(&mut um).unwrap_or(0) == 0 {
            break;
        }
        b.push(um[0]);
    }
    String::from_utf8(b).unwrap()
}

fn cab<'a>(texto: &'a str, k: &str) -> Option<&'a str> {
    texto.split("\r\n").filter_map(|l| l.split_once(':')).find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.trim())
}

fn status(texto: &str) -> u16 {
    texto.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// o que o servidor remoto viu do pedido de upgrade
#[derive(Default, Clone)]
struct Visto {
    pedido: String,
}

/// atende um WebSocket "remoto": confere o pedido, responde 101 com um Accept inventado (a ponte só repassa) e devolve
/// cada byte recebido com +1 (para provar que passou pelo remoto)
fn atender_ws<S: Read + Write>(mut s: S, visto: &Arc<Mutex<Visto>>, recusar: Option<u16>) {
    let pedido = ler_cabecalho(&mut s);
    visto.lock().unwrap().pedido = pedido.clone();
    if let Some(st) = recusar {
        let _ = s.write_all(format!("HTTP/1.1 {st} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        return;
    }
    let chave = cab(&pedido, "sec-websocket-key").unwrap_or("").to_string();
    let r = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: aceite-de-{chave}\r\n\r\n");
    s.write_all(r.as_bytes()).unwrap();
    // um quadro que o servidor manda logo depois do 101 (chega junto com o cabeçalho na ponte)
    s.write_all(b"\x81\x02oi").unwrap();
    s.flush().unwrap();
    let mut buf = vec![0u8; 32 << 10];
    loop {
        let n = match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for b in &mut buf[..n] {
            *b = b.wrapping_add(1);
        }
        if s.write_all(&buf[..n]).is_err() || s.flush().is_err() {
            break;
        }
    }
}

fn remoto_ws(recusar: Option<u16>) -> (u16, Arc<Mutex<Visto>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = l.local_addr().unwrap().port();
    let visto = Arc::new(Mutex::new(Visto::default()));
    let v2 = visto.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let v = v2.clone();
            thread::spawn(move || atender_ws(s, &v, recusar));
        }
    });
    (porta, visto)
}

fn remoto_wss() -> (u16, Arc<Mutex<Visto>>) {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    nfsu2srv::ponte::testar_raiz_extra(Some(ck.cert.der().to_vec()));
    let chave = rustls::pki_types::PrivateKeyDer::Pkcs8(ck.signing_key.serialize_der().into());
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![ck.cert.der().clone()], chave)
        .unwrap();
    let cfg = Arc::new(cfg);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = l.local_addr().unwrap().port();
    let visto = Arc::new(Mutex::new(Visto::default()));
    let v2 = visto.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (v, cfg) = (v2.clone(), cfg.clone());
            thread::spawn(move || {
                let c = rustls::ServerConnection::new(cfg).unwrap();
                atender_ws(rustls::StreamOwned::new(c, s), &v, None);
            });
        }
    });
    (porta, visto)
}

/// abre o /lan no servidor local como a página faria; devolve a resposta e o socket
fn abrir_lan(local: u16, query: &str) -> (String, TcpStream) {
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let p = format!(
        "GET /lan{query} HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nOrigin: http://127.0.0.1:{local}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    s.write_all(p.as_bytes()).unwrap();
    let r = ler_cabecalho(&mut s);
    (r, s)
}

fn padrao(n: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// manda `dados` e confere que tudo volta com +1, lendo e escrevendo ao mesmo tempo
fn ida_e_volta(s: &TcpStream, dados: &[u8]) {
    let mut w = s.try_clone().unwrap();
    let envio = dados.to_vec();
    let t = thread::spawn(move || {
        for parte in envio.chunks(7919) {
            w.write_all(parte).unwrap();
        }
    });
    let mut volta = vec![0u8; dados.len()];
    let mut r = s;
    r.read_exact(&mut volta).unwrap();
    t.join().unwrap();
    assert!(volta.iter().zip(dados).all(|(a, b)| *a == b.wrapping_add(1)), "os bytes voltaram diferentes");
}

fn ws_completo(local: u16, remota: u16, tls: bool, visto: &Arc<Mutex<Visto>>) {
    let url = format!("{}://localhost:{remota}", if tls { "https" } else { "http" });
    nfsu2srv::ponte::definir(Some(&url)).unwrap();
    let (r, mut s) = abrir_lan(local, "?room=7");
    assert_eq!(status(&r), 101, "{r}");
    assert_eq!(cab(&r, "sec-websocket-accept"), Some("aceite-de-dGhlIHNhbXBsZSBub25jZQ=="), "{r}");
    // o pedido que chegou ao remoto: mesmo caminho, mesma chave, Host do remoto, sem Origin
    let pedido = visto.lock().unwrap().pedido.clone();
    assert!(pedido.starts_with("GET /lan?room=7 HTTP/1.1\r\n"), "{pedido}");
    assert_eq!(cab(&pedido, "host"), Some(format!("localhost:{remota}").as_str()));
    assert_eq!(cab(&pedido, "sec-websocket-key"), Some("dGhlIHNhbXBsZSBub25jZQ=="));
    assert!(cab(&pedido, "origin").is_none(), "a ponte não pode mandar Origin: {pedido}");
    // o quadro que veio grudado no 101
    let mut oi = [0u8; 4];
    s.read_exact(&mut oi).unwrap();
    assert_eq!(&oi, b"\x81\x02oi");
    // pouco, e depois muito (mais que o limite de fila da ponte), nos dois sentidos ao mesmo tempo
    ida_e_volta(&s, b"\x82\x83abcabc");
    ida_e_volta(&s, &padrao(9 << 20));
    // a página fecha: a ponte fecha o remoto (e a próxima conexão funciona)
    drop(s);
    let (r, s2) = abrir_lan(local, "");
    assert_eq!(status(&r), 101);
    let mut oi = [0u8; 4];
    (&s2).read_exact(&mut oi).unwrap();
    ida_e_volta(&s2, b"de novo");
}

#[test]
fn websocket_sem_tls() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    let (remota, visto) = remoto_ws(None);
    ws_completo(local, remota, false, &visto);
}

#[test]
fn websocket_com_tls() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    let (remota, visto) = remoto_wss();
    ws_completo(local, remota, true, &visto);
    // um certificado que não é de uma raiz conhecida: a ponte recusa (502), não aceita qualquer coisa
    nfsu2srv::ponte::testar_raiz_extra(None);
    let (r, _s) = abrir_lan(local, "");
    assert_eq!(status(&r), 502, "{r}");
}

#[test]
fn o_remoto_fecha() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    // remoto que aceita, manda um quadro e fecha
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let remota = l.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        ler_cabecalho(&mut s);
        s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: x\r\n\r\n\x88\x00").unwrap();
    });
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{remota}"))).unwrap();
    let (r, mut s) = abrir_lan(local, "");
    assert_eq!(status(&r), 101);
    let mut tudo = Vec::new();
    s.read_to_end(&mut tudo).unwrap();
    assert_eq!(tudo, b"\x88\x00", "o quadro de fechar chega e a conexão termina");
}

#[test]
fn falhas_do_lan() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    // nenhum servidor escolhido: 503
    nfsu2srv::ponte::definir(None).unwrap();
    assert_eq!(status(&abrir_lan(local, "").0), 503);
    // fora do ar: 502
    let morta = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{morta}"))).unwrap();
    assert_eq!(status(&abrir_lan(local, "").0), 502);
    // o remoto recusa (cheio, banido): o mesmo status
    let (remota, _) = remoto_ws(Some(429));
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{remota}"))).unwrap();
    assert_eq!(status(&abrir_lan(local, "").0), 429);
    // outra origem: 403, e sem upgrade: 400
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(format!("GET /lan HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nOrigin: https://evil.example\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: a\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes()).unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 403);
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(format!("GET /lan HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\n\r\n").as_bytes()).unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 400);
    // endereço inválido não muda o que estava escolhido
    assert!(nfsu2srv::ponte::definir(Some("nfsu2.xyz")).is_err());
    assert_eq!(nfsu2srv::ponte::atual().unwrap().port, remota);
    // pela API C (a que o Swift usa)
    let ruim = std::ffi::CString::new("ftp://x").unwrap();
    assert_eq!(nfsu2srv::nfsu2srv_set_remote(ruim.as_ptr()), -1);
    assert!(nfsu2srv::last_error().contains("ftp://x"), "{}", nfsu2srv::last_error());
    let bom = std::ffi::CString::new("https://nfsu2.xyz").unwrap();
    assert_eq!(nfsu2srv::nfsu2srv_set_remote(bom.as_ptr()), 0);
    assert_eq!(nfsu2srv::ponte::atual().unwrap().url(), "https://nfsu2.xyz");
    assert_eq!(nfsu2srv::nfsu2srv_set_remote(std::ptr::null()), 0);
    assert!(nfsu2srv::ponte::atual().is_none());
}

#[test]
fn players_repassado() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let remota = l.local_addr().unwrap().port();
    let pedidos = Arc::new(Mutex::new(Vec::<String>::new()));
    let p2 = pedidos.clone();
    thread::spawn(move || {
        for (i, s) in l.incoming().flatten().enumerate() {
            let mut s = s;
            let pedido = ler_cabecalho(&mut s);
            p2.lock().unwrap().push(pedido.clone());
            let r: &[u8] = if i % 2 == 0 {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"n\":3,\r\nb\r\n\"crews\":[]}\r\n0\r\n\r\n"
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\nConnection: close\r\n\r\n{\"n\":5}"
            };
            if pedido.starts_with("HEAD") {
                let fim = r.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                s.write_all(&r[..fim]).unwrap();
            } else {
                s.write_all(r).unwrap();
            }
        }
    });
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{remota}/"))).unwrap();
    let pedir = |metodo: &str, alvo: &str| -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(format!("{metodo} {alvo} HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let c = ler_cabecalho(&mut s);
        let mut corpo = Vec::new();
        s.read_to_end(&mut corpo).unwrap();
        (c, corpo)
    };
    let (c, corpo) = pedir("GET", "/players?crews=1");
    assert_eq!(status(&c), 200, "{c}");
    assert_eq!(corpo, br#"{"n":3,"crews":[]}"#);
    assert_eq!(cab(&c, "content-type"), Some("application/json"));
    assert_eq!(cab(&c, "cross-origin-resource-policy"), Some("same-origin"), "os cabeçalhos de isolamento continuam");
    let (_, corpo) = pedir("GET", "/players");
    assert_eq!(corpo, br#"{"n":5}"#);
    let (c, corpo) = pedir("HEAD", "/players");
    assert_eq!(status(&c), 200);
    assert!(corpo.is_empty());
    let vistos = pedidos.lock().unwrap().clone();
    assert!(vistos[0].starts_with("GET /players?crews=1 HTTP/1.1\r\n"), "{}", vistos[0]);
    assert!(vistos.iter().all(|p| cab(p, "origin").is_none()));
    // nenhum servidor: 503 (a página entra no modo offline dela; com 404 acharia que o servidor está no ar)
    nfsu2srv::ponte::definir(None).unwrap();
    assert_eq!(status(&pedir("GET", "/players").0), 503);
    assert_eq!(status(&pedir("HEAD", "/players").0), 503);
}

#[test]
fn desligar_corta_a_ponte() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    let (remota, _) = remoto_ws(None);
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{remota}"))).unwrap();
    let (r, mut s) = abrir_lan(local, "");
    assert_eq!(status(&r), 101);
    let mut oi = [0u8; 4];
    s.read_exact(&mut oi).unwrap();
    nfsu2srv::stop();
    // a ponte percebe em até ~1 s e fecha a conexão da página
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut resto = Vec::new();
    assert!(s.read_to_end(&mut resto).is_ok(), "a conexão terminou (não ficou presa)");
}

// ----------------------------------------------------------------- /vault, /names, /friends, /seen (repassar) --

/// um pedido como o servidor HTTP falso o viu: a primeira linha, o cabeçalho inteiro e o corpo
#[derive(Clone, Debug)]
struct PedidoVisto {
    linha: String,
    cabecalho: String,
    corpo: Vec<u8>,
}

/// atende pedidos HTTP como o servidor oficial faria (o bastante para a ponte): guarda o que viu e responde conforme
/// o caminho
fn atender_http<S: Read + Write>(mut s: S, vistos: &Arc<Mutex<Vec<PedidoVisto>>>) {
    let cabecalho = ler_cabecalho(&mut s);
    let linha = cabecalho.split("\r\n").next().unwrap_or("").to_string();
    let n: usize = cab(&cabecalho, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut corpo = vec![0u8; n];
    s.read_exact(&mut corpo).unwrap();
    vistos.lock().unwrap().push(PedidoVisto { linha: linha.clone(), cabecalho: cabecalho.clone(), corpo: corpo.clone() });
    let auth = cab(&cabecalho, "authorization").is_some();
    let resp: Vec<u8> = if linha.starts_with("GET /vault ") && !auth {
        b"HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope".to_vec()
    } else if linha.starts_with("GET /vault ") && cab(&cabecalho, "if-none-match") == Some("\"0123456789abcdef\"") {
        b"HTTP/1.1 304 Not Modified\r\nETag: \"0123456789abcdef\"\r\nConnection: close\r\n\r\n".to_vec()
    } else if linha.starts_with("GET /vault ") {
        let mut r = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nETag: \"0123456789abcdef\"\r\nx-vault-saved: 1793800000000\r\nSet-Cookie: x=1\r\nContent-Length: 200000\r\nConnection: close\r\n\r\n".to_vec();
        r.extend(padrao(200_000));
        r
    } else if linha.starts_with("PUT /vault ") {
        let ok = cab(&cabecalho, "if-match") == Some("\"0123456789abcdef\"");
        if ok {
            format!("HTTP/1.1 200 OK\r\nETag: \"fedcba9876543210\"\r\nx-vault-saved: 1793800001000\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
        } else {
            b"HTTP/1.1 412 Precondition Failed\r\nETag: \"0123456789abcdef\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    } else if linha.starts_with("DELETE /vault ") {
        b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec()
    } else if linha.starts_with("POST /names ") {
        // o corpo de volta, num JSON (prova que chegou inteiro)
        let j = format!("{{\"mine\":{{\"name\":\"GTM-\"}},\"got\":{}}}", corpo.len());
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{j}\r\n0\r\n\r\n", j.len()).into_bytes()
    } else if linha.starts_with("GET /names") {
        let j = br#"{"name":"GTM-","state":"yours"}"#;
        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", j.len()).into_bytes();
        r.extend_from_slice(j);
        r
    } else if linha.starts_with("HEAD ") {
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 99\r\nConnection: close\r\n\r\n".to_vec()
    } else {
        b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow".to_vec()
    };
    let _ = s.write_all(&resp);
    let _ = s.flush();
}

fn remoto_http(tls: bool) -> (u16, Arc<Mutex<Vec<PedidoVisto>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = l.local_addr().unwrap().port();
    let vistos = Arc::new(Mutex::new(Vec::new()));
    let v2 = vistos.clone();
    let cfg = tls.then(|| {
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        nfsu2srv::ponte::testar_raiz_extra(Some(ck.cert.der().to_vec()));
        let chave = rustls::pki_types::PrivateKeyDer::Pkcs8(ck.signing_key.serialize_der().into());
        Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![ck.cert.der().clone()], chave)
                .unwrap(),
        )
    });
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (v, cfg) = (v2.clone(), cfg.clone());
            thread::spawn(move || match cfg {
                Some(cfg) => {
                    let c = rustls::ServerConnection::new(cfg).unwrap();
                    atender_http(rustls::StreamOwned::new(c, s), &v);
                }
                None => atender_http(s, &v),
            });
        }
    });
    (porta, vistos)
}

/// uma resposta lida de uma conexão com keep-alive: o cabeçalho e o corpo (pelo Content-Length)
fn ler_resposta(s: &mut TcpStream, head: bool) -> (String, Vec<u8>) {
    let c = ler_cabecalho(s);
    let n: usize = if head || status(&c) == 304 { 0 } else { cab(&c, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0) };
    let mut corpo = vec![0u8; n];
    s.read_exact(&mut corpo).unwrap();
    (c, corpo)
}

/// manda um pedido como a página (Origin e Sec-Fetch-Site da própria página), em pedaços, numa conexão aberta
fn mandar(s: &mut TcpStream, local: u16, metodo: &str, alvo: &str, extra: &str, corpo: &[u8]) {
    let mut p = format!(
        "{metodo} {alvo} HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nOrigin: http://127.0.0.1:{local}\r\nSec-Fetch-Site: same-origin\r\nCookie: c=1\r\n{extra}"
    );
    if !corpo.is_empty() || metodo == "POST" || metodo == "PUT" {
        p.push_str(&format!("Content-Length: {}\r\n", corpo.len()));
    }
    p.push_str("\r\n");
    let mut tudo = p.into_bytes();
    tudo.extend_from_slice(corpo);
    for parte in tudo.chunks(7001) {
        s.write_all(parte).unwrap();
        s.flush().unwrap();
    }
}

fn repasse_completo(tls: bool) {
    let local = ligar_local();
    let (remota, vistos) = remoto_http(tls);
    let url = format!("{}://localhost:{remota}", if tls { "https" } else { "http" });
    nfsu2srv::ponte::definir(Some(&url)).unwrap();
    let token = "Bearer AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    // tudo numa conexão só (keep-alive), como o WebKit faz: o corpo de um pedido não pode vazar para o seguinte
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

    // a página descobre que o servidor tem nuvem: GET /vault sem token -> 401 (com 404 ela esconderia a nuvem)
    mandar(&mut s, local, "GET", "/vault", "", &[]);
    let (c, corpo) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 401, "{c}");
    assert_eq!(corpo, b"nope");
    assert_eq!(cab(&c, "cross-origin-resource-policy"), Some("same-origin"), "os cabeçalhos de isolamento continuam");
    assert_eq!(cab(&c, "connection"), Some("keep-alive"));

    // os saves da nuvem: 200 KB, com ETag e x-vault-saved; o Set-Cookie do remoto não passa
    mandar(&mut s, local, "GET", "/vault", &format!("Authorization: {token}\r\n"), &[]);
    let (c, corpo) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 200, "{c}");
    assert_eq!(corpo, padrao(200_000));
    assert_eq!(cab(&c, "etag"), Some("\"0123456789abcdef\""));
    assert_eq!(cab(&c, "x-vault-saved"), Some("1793800000000"));
    assert_eq!(cab(&c, "content-type"), Some("application/octet-stream"));
    assert!(cab(&c, "set-cookie").is_none(), "{c}");

    // nada mudou na nuvem: 304, sem corpo
    mandar(&mut s, local, "GET", "/vault", &format!("Authorization: {token}\r\nIf-None-Match: \"0123456789abcdef\"\r\n"), &[]);
    let (c, _) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 304, "{c}");
    assert!(cab(&c, "content-length").is_none(), "{c}");

    // enviar os saves (250 KB, em pedaços) sobre a versão que a página tem
    let saves = padrao(250_000);
    mandar(&mut s, local, "PUT", "/vault", &format!("Authorization: {token}\r\nIf-Match: \"0123456789abcdef\"\r\nContent-Type: application/octet-stream\r\n"), &saves);
    let (c, _) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 200, "{c}");
    assert_eq!(cab(&c, "etag"), Some("\"fedcba9876543210\""));
    // outro aparelho mandou antes: 412 com a versão de lá (a página mostra o conflito)
    mandar(&mut s, local, "PUT", "/vault", &format!("Authorization: {token}\r\nIf-Match: \"1111111111111111\"\r\n"), b"x");
    let (c, _) = ler_resposta(&mut s, false);
    assert_eq!((status(&c), cab(&c, "etag")), (412, Some("\"0123456789abcdef\"")), "{c}");

    // o nome reservado: perguntar e reservar (resposta chunked do remoto vira Content-Length)
    mandar(&mut s, local, "GET", "/names?name=GTM-&alt=GTM-1234", &format!("Authorization: {token}\r\n"), &[]);
    let (c, corpo) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 200);
    assert_eq!(corpo, br#"{"name":"GTM-","state":"yours"}"#);
    let pedido_nome = br#"{"name":"GTM-","captcha":"x"}"#;
    mandar(&mut s, local, "POST", "/names", &format!("Authorization: {token}\r\nContent-Type: application/json\r\n"), pedido_nome);
    let (c, corpo) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 200, "{c}");
    assert_eq!(String::from_utf8(corpo).unwrap(), format!("{{\"mine\":{{\"name\":\"GTM-\"}},\"got\":{}}}", pedido_nome.len()));

    // HEAD, um erro do remoto com Retry-After, e DELETE
    mandar(&mut s, local, "HEAD", "/friends", &format!("Authorization: {token}\r\n"), &[]);
    let (c, corpo) = ler_resposta(&mut s, true);
    assert_eq!(status(&c), 200);
    assert!(corpo.is_empty());
    mandar(&mut s, local, "GET", "/seen?at=play&id=abc", "", &[]);
    let (c, corpo) = ler_resposta(&mut s, false);
    assert_eq!((status(&c), cab(&c, "retry-after"), corpo.as_slice()), (429, Some("60"), &b"slow"[..]), "{c}");
    mandar(&mut s, local, "DELETE", "/vault", &format!("Authorization: {token}\r\n"), &[]);
    let (c, _) = ler_resposta(&mut s, false);
    assert_eq!(status(&c), 204, "{c}");

    // o que chegou ao remoto
    let v = vistos.lock().unwrap().clone();
    let linhas: Vec<&str> = v.iter().map(|p| p.linha.as_str()).collect();
    assert_eq!(
        linhas,
        [
            "GET /vault HTTP/1.1",
            "GET /vault HTTP/1.1",
            "GET /vault HTTP/1.1",
            "PUT /vault HTTP/1.1",
            "PUT /vault HTTP/1.1",
            "GET /names?name=GTM-&alt=GTM-1234 HTTP/1.1",
            "POST /names HTTP/1.1",
            "HEAD /friends HTTP/1.1",
            "GET /seen?at=play&id=abc HTTP/1.1",
            "DELETE /vault HTTP/1.1",
        ]
    );
    for p in &v {
        // nem Origin, nem Sec-Fetch-Site (o oficial recusaria: "página de outro site"), nem o cookie da página
        for k in ["origin", "sec-fetch-site", "cookie"] {
            assert!(cab(&p.cabecalho, k).is_none(), "{k} foi ao remoto: {}", p.cabecalho);
        }
        assert_eq!(cab(&p.cabecalho, "host"), Some(format!("localhost:{remota}").as_str()));
    }
    assert!(cab(&v[0].cabecalho, "authorization").is_none());
    assert_eq!(cab(&v[1].cabecalho, "authorization"), Some(token));
    assert_eq!(cab(&v[2].cabecalho, "if-none-match"), Some("\"0123456789abcdef\""));
    assert_eq!(v[3].corpo, saves, "os saves chegaram inteiros");
    assert_eq!(cab(&v[3].cabecalho, "if-match"), Some("\"0123456789abcdef\""));
    assert_eq!(cab(&v[3].cabecalho, "content-type"), Some("application/octet-stream"));
    assert_eq!(v[6].corpo, pedido_nome);
}

#[test]
fn nuvem_nomes_e_amigos_sem_tls() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    repasse_completo(false);
}

#[test]
fn nuvem_nomes_e_amigos_com_tls() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    repasse_completo(true);
}

#[test]
fn falhas_do_repasse() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = ligar_local();
    let um = |metodo: &str, alvo: &str, extra: &str, corpo: &[u8]| -> String {
        let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        mandar(&mut s, local, metodo, alvo, extra, corpo);
        ler_cabecalho(&mut s)
    };
    // nenhum servidor escolhido: 404, o "servidor sem nuvem e sem nomes" de antes
    nfsu2srv::ponte::definir(None).unwrap();
    for p in ["/vault", "/names?name=x", "/friends", "/seen?at=play"] {
        assert_eq!(status(&um("GET", p, "", &[])), 404, "{p}");
    }
    assert_eq!(status(&um("PUT", "/vault", "", b"abc")), 404);
    // sem internet (o servidor não responde): 502, e a página segue com os saves do iPhone
    let morta = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{morta}"))).unwrap();
    assert_eq!(status(&um("GET", "/vault", "", &[])), 502);
    assert_eq!(status(&um("PUT", "/vault", "", b"abc")), 502);
    assert_eq!(status(&um("POST", "/names", "", b"{}")), 502);
    // nada chega ao remoto nestes: método que a rota não tem, corpo onde não cabe, corpo grande demais, outra origem
    let (remota, vistos) = remoto_http(false);
    nfsu2srv::ponte::definir(Some(&format!("http://127.0.0.1:{remota}"))).unwrap();
    let r = um("PATCH", "/vault", "", b"x");
    assert_eq!((status(&r), cab(&r, "allow")), (405, Some("GET, HEAD, PUT, DELETE")), "{r}");
    let r = um("POST", "/seen", "", b"x");
    assert_eq!((status(&r), cab(&r, "allow")), (405, Some("GET, HEAD")), "{r}");
    assert_eq!(status(&um("GET", "/names", "", b"corpo")), 413);
    // (grande demais: só o cabeçalho, com o tamanho; a recusa vem antes do corpo e fecha a conexão, e um corpo que
    // ainda estivesse indo levaria um RST que pode apagar a resposta)
    let grande = |metodo: &str, alvo: &str, n: usize| -> String {
        let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(format!("{metodo} {alvo} HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nContent-Length: {n}\r\n\r\n").as_bytes()).unwrap();
        ler_cabecalho(&mut s)
    };
    let r = grande("PUT", "/vault", 600 << 10);
    assert_eq!((status(&r), cab(&r, "connection")), (413, Some("close")), "{r}");
    assert_eq!(status(&grande("POST", "/names", 20 << 10)), 413);
    assert_eq!(status(&grande("POST", "/friends", 20 << 10)), 413);
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(format!("GET /vault HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nOrigin: https://evil.example\r\n\r\n").as_bytes()).unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 403);
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(format!("GET /vault HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nSec-Fetch-Site: cross-site\r\n\r\n").as_bytes()).unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 403);
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(format!("POST /names HTTP/1.1\r\nHost: 127.0.0.1:{local}\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n").as_bytes()).unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 411);
    // outro Host (rebinding de DNS) continua 421
    let mut s = TcpStream::connect(("127.0.0.1", local)).unwrap();
    s.write_all(b"GET /vault HTTP/1.1\r\nHost: evil.example\r\n\r\n").unwrap();
    assert_eq!(status(&ler_cabecalho(&mut s)), 421);
    thread::sleep(Duration::from_millis(200));
    assert!(vistos.lock().unwrap().is_empty(), "nada destes pode chegar ao remoto");
    // e a rota de antes continua recusando corpo
    let r = um("POST", "/game.json", "", b"x");
    assert_eq!(status(&r), 405);
}
