//! nfsu2srv: o servidor HTTP local do app NFSU2 para iPhone.
//!
//! Escuta só em 127.0.0.1 (nada sai do aparelho), numa porta fixa: a origem http://127.0.0.1:<porta> é onde a
//! página guarda os saves (OPFS), então a porta não pode mudar entre uma abertura e outra.  Uma thread aceita as
//! conexões (poll de 500 ms, para poder parar) e cada conexão tem a sua thread, com keep-alive.
//!
//! O iOS pode tirar o socket de escuta de um app suspenso.  Quando isso acontece o laço de aceitar termina e
//! `alive()` passa a dizer false; o app chama `start()` de novo ao voltar para a frente, que reabre a mesma porta.
//!
//! API C (include/nfsu2srv.h): nfsu2srv_start, nfsu2srv_stop, nfsu2srv_alive, nfsu2srv_port, nfsu2srv_last_error,
//! nfsu2srv_set_remote (o servidor de multiplayer da ponte, src/ponte.rs).

// a API C recebe ponteiros do Swift (strings terminadas em NUL que ele garante); marcar unsafe não muda nada lá
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod http;
pub mod manifest;
pub mod ponte;
pub mod routes;

use std::ffi::{c_char, CStr};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub use routes::Config;

/// conexões abertas ao mesmo tempo, no máximo (o WebKit abre poucas por origem; isto só limita um descontrole)
const MAX_CONNS: usize = 256;

struct Running {
    alive: Arc<AtomicBool>,
    port: u16,
    cfg: Arc<Config>,
    handle: Option<thread::JoinHandle<()>>,
    /// o descritor do socket de escuta (só para o teste que simula o iOS fechando o socket)
    fd: i32,
}

static STATE: Mutex<Option<Running>> = Mutex::new(None);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());
static CONNS: AtomicUsize = AtomicUsize::new(0);

fn set_error(s: String) {
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

pub fn last_error() -> String {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Liga o servidor em 127.0.0.1:`port` (0: uma porta livre qualquer, para testes) servindo `pack` e `game`.
/// Já ligado, vivo, atendendo, na mesma porta e com as mesmas pastas: não faz nada.  Devolve a porta.
///
/// "Atendendo" é conferido de verdade, com uma conexão a 127.0.0.1:porta: ao voltar da suspensão, o socket que o
/// iOS fechou recusa a conexão antes de o laço de aceitar perceber e marcar o servidor como parado.
pub fn start(pack: &Path, game: &Path, port: u16) -> Result<u16, String> {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = st.as_ref() {
        if r.alive.load(Ordering::SeqCst) && (port == 0 || r.port == port) && r.cfg.same_dirs(pack, game) && responde(r.port) {
            return Ok(r.port);
        }
    }
    if let Some(r) = st.take() {
        stop_running(r);
    }
    // a mesma porta logo depois de fechar: SO_REUSEADDR (o TcpListener do Rust já liga no Unix); e algumas tentativas
    // se o sistema ainda segura a porta por um instante
    let mut tries = 0;
    let listener = loop {
        match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) {
            Ok(l) => break l,
            Err(e) if tries < 20 && e.kind() == io::ErrorKind::AddrInUse => {
                tries += 1;
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                let msg = format!("não consegui abrir 127.0.0.1:{port}: {e}");
                set_error(msg.clone());
                return Err(msg);
            }
        }
    };
    let real_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let fd = listener.as_raw_fd();
    if let Err(e) = listener.set_nonblocking(true) {
        let msg = format!("socket de escuta: {e}");
        set_error(msg.clone());
        return Err(msg);
    }
    let cfg = Arc::new(Config::new(pack.to_path_buf(), game.to_path_buf(), real_port));
    let alive = Arc::new(AtomicBool::new(true));
    let (a2, c2) = (alive.clone(), cfg.clone());
    let handle = thread::Builder::new()
        .name("nfsu2srv-accept".into())
        .spawn(move || accept_loop(listener, a2, c2))
        .map_err(|e| {
            let msg = format!("thread do servidor: {e}");
            set_error(msg.clone());
            msg
        })?;
    *st = Some(Running { alive, port: real_port, cfg, handle: Some(handle), fd });
    set_error(String::new());
    Ok(real_port)
}

/// alguém atende em 127.0.0.1:porta (a conexão de teste chega ao servidor e fecha sem pedido)
fn responde(port: u16) -> bool {
    let addr = std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// Só para testes: derruba o socket de escuta como o iOS faz com um app suspenso (o laço de aceitar ainda não
/// percebeu; o próximo `start` tem que notar e religar).
#[doc(hidden)]
pub fn simular_socket_fechado_pelo_sistema() {
    if let Some(r) = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        unsafe {
            libc::shutdown(r.fd, libc::SHUT_RDWR);
        }
    }
}

fn stop_running(mut r: Running) {
    r.cfg.retired.store(true, Ordering::SeqCst);
    if let Some(h) = r.handle.take() {
        let _ = h.join();
    }
}

pub fn stop() {
    let r = STATE.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(r) = r {
        stop_running(r);
    }
}

pub fn alive() -> bool {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|r| r.alive.load(Ordering::SeqCst))
}

pub fn port() -> u16 {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(0, |r| r.port)
}

fn accept_loop(listener: TcpListener, alive: Arc<AtomicBool>, cfg: Arc<Config>) {
    let fd = listener.as_raw_fd();
    loop {
        if cfg.retired.load(Ordering::SeqCst) {
            break;
        }
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, 500) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            set_error(format!("poll: {e}"));
            break;
        }
        if r == 0 {
            continue;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 && pfd.revents & libc::POLLIN == 0 {
            set_error("o sistema fechou o socket de escuta".into());
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if CONNS.load(Ordering::SeqCst) >= MAX_CONNS {
                    drop(stream);
                    continue;
                }
                CONNS.fetch_add(1, Ordering::SeqCst);
                let cfg = cfg.clone();
                let spawned = thread::Builder::new().name("nfsu2srv-conn".into()).stack_size(256 << 10).spawn(move || {
                    // (um panic aqui fica nesta thread; o contador volta de qualquer jeito)
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(stream, &cfg)));
                    CONNS.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    CONNS.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted => continue,
                _ => {
                    let raw = e.raw_os_error().unwrap_or(0);
                    if raw == libc::EMFILE || raw == libc::ENFILE || raw == libc::ENOBUFS || raw == libc::ENOMEM {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    set_error(format!("accept: {e}"));
                    break;
                }
            },
        }
    }
    alive.store(false, Ordering::SeqCst);
}

/// SO_NOSIGPIPE: escrever num socket que o outro lado fechou dá erro, nunca SIGPIPE (que mataria o app).  No Linux o
/// Rust já escreve com MSG_NOSIGNAL.
pub(crate) fn no_sigpipe(s: &TcpStream) {
    #[cfg(any(target_os = "ios", target_os = "macos", target_os = "tvos", target_os = "visionos"))]
    unsafe {
        let one: libc::c_int = 1;
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            &one as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "tvos", target_os = "visionos")))]
    let _ = s;
}

fn handle(stream: TcpStream, cfg: &Config) {
    // no iOS/macOS a conexão aceita herda o O_NONBLOCK do socket de escuta: volta a bloquear
    let _ = stream.set_nonblocking(false);
    no_sigpipe(&stream);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(75)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
    let mut conn = http::Conn::new(stream);
    loop {
        if cfg.retired.load(Ordering::SeqCst) {
            break;
        }
        let q = match conn.read_request() {
            Ok(Some(q)) => q,
            Ok(None) => break,
            Err(http::ReadError::Io(_)) => break,
            Err(http::ReadError::Bad) => {
                let _ = conn.write_head(400, &[("Content-Type", "text/plain; charset=utf-8".to_string())], Some(0), false);
                break;
            }
            Err(http::ReadError::TooLarge) => {
                let _ = conn.write_head(431, &[("Content-Type", "text/plain; charset=utf-8".to_string())], Some(0), false);
                break;
            }
        };
        // desligado enquanto esperava este pedido: fecha sem responder (o WebKit repete numa conexão nova)
        if cfg.retired.load(Ordering::SeqCst) {
            break;
        }
        match routes::serve(cfg, &q, &mut conn) {
            Ok(true) => continue,
            _ => break,
        }
    }
    let _ = conn.stream.shutdown(std::net::Shutdown::Both);
}

// ------------------------------------------------------------------------------------------------------- API C --

fn path_arg(p: *const c_char) -> PathBuf {
    if p.is_null() {
        return PathBuf::new();
    }
    let s = unsafe { CStr::from_ptr(p) };
    PathBuf::from(s.to_string_lossy().into_owned())
}

/// Liga (ou religa) o servidor.  Devolve a porta (> 0) ou -1 (nfsu2srv_last_error diz por quê).
#[no_mangle]
pub extern "C" fn nfsu2srv_start(pack_dir: *const c_char, game_dir: *const c_char, port: u16) -> i32 {
    let (pack, game) = (path_arg(pack_dir), path_arg(game_dir));
    match std::panic::catch_unwind(|| start(&pack, &game, port)) {
        Ok(Ok(p)) => p as i32,
        Ok(Err(_)) => -1,
        Err(_) => {
            set_error("panic ao ligar o servidor".into());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn nfsu2srv_stop() {
    let _ = std::panic::catch_unwind(stop);
}

/// 1 se o laço de aceitar está rodando
#[no_mangle]
pub extern "C" fn nfsu2srv_alive() -> i32 {
    std::panic::catch_unwind(alive).unwrap_or(false) as i32
}

#[no_mangle]
pub extern "C" fn nfsu2srv_port() -> i32 {
    std::panic::catch_unwind(port).unwrap_or(0) as i32
}

/// Copia a última mensagem de erro (UTF-8, terminada em NUL) para buf; devolve o tamanho dela (sem o NUL).
#[no_mangle]
pub extern "C" fn nfsu2srv_last_error(buf: *mut c_char, len: i32) -> i32 {
    let msg = last_error();
    let b = msg.as_bytes();
    if !buf.is_null() && len > 0 {
        let n = b.len().min(len as usize - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr() as *const c_char, buf, n);
            *buf.add(n) = 0;
        }
    }
    b.len() as i32
}

/// Escolhe o servidor de multiplayer da ponte (/lan e /players): "https://nfsu2.xyz", "http://100.91.24.1:9091"...
/// NULL ou "": nenhum (a página fica sem multiplayer).  Vale para as próximas conexões.  0 ou -1 (endereço inválido).
#[no_mangle]
pub extern "C" fn nfsu2srv_set_remote(url: *const c_char) -> i32 {
    let u = if url.is_null() { None } else { Some(unsafe { CStr::from_ptr(url) }.to_string_lossy().into_owned()) };
    match std::panic::catch_unwind(|| ponte::definir(u.as_deref())) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            set_error(e);
            -1
        }
        Err(_) => -1,
    }
}
