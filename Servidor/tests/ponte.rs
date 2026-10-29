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
