//! Para testar no computador: cargo run --example serve -- <pasta do pacote> <pasta do jogo> [porta]
use std::path::PathBuf;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("uso: serve <pacote> <jogo> [porta]");
        std::process::exit(2);
    }
    let port: u16 = a.get(3).and_then(|p| p.parse().ok()).unwrap_or(8123);
    match nfsu2srv::start(&PathBuf::from(&a[1]), &PathBuf::from(&a[2]), port) {
        Ok(p) => eprintln!("servindo em http://127.0.0.1:{p}/ (Ctrl-C para sair)"),
        Err(e) => {
            eprintln!("erro: {e}");
            std::process::exit(1);
        }
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if !nfsu2srv::alive() {
            eprintln!("o servidor parou: {}", nfsu2srv::last_error());
            std::process::exit(1);
        }
    }
}
