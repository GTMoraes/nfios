//! HTTP/1.1 minimo: le pedidos (sem corpo) de uma conexao com keep-alive e escreve respostas.
//!
//! So o que a pagina do jogo e o WebKit usam: GET e HEAD, cabecalhos, Range.  Um pedido com corpo recebe 413 e a
//! conexao fecha (nada aqui recebe corpo).  Pedidos em sequencia na mesma conexao (keep-alive, inclusive os que
//! chegam juntos no mesmo pacote) sao atendidos um por vez, na ordem.

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// o maior cabecalho aceito (linha do pedido + cabecalhos)
pub const MAX_HEAD: usize = 32 << 10;

pub struct Request {
    pub method: String,
    /// o alvo como veio ("/data/CARS/x.bin?b=1")
    pub target: String,
    /// o caminho decodificado ("/data/CARS/x.bin"); vazio quando o alvo nao era um caminho valido
    pub path: String,
    /// a query sem o "?" (sem decodificar)
    pub query: String,
    pub http11: bool,
    /// nomes em minusculas, na ordem em que vieram
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// keep-alive: HTTP/1.1 sem "Connection: close", ou HTTP/1.0 com "Connection: keep-alive"
    pub fn keep_alive(&self) -> bool {
        let c = self.header("connection").unwrap_or("").to_ascii_lowercase();
        let has = |w: &str| c.split(',').any(|t| t.trim() == w);
        if self.http11 {
            !has("close")
        } else {
            has("keep-alive")
        }
    }

    /// um parametro da query, decodificado ("b" em "b=3&n=8")
    pub fn param(&self, name: &str) -> Option<String> {
        for part in self.query.split('&') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            if percent_decode(k).as_deref() == Some(name) {
                return percent_decode(&v.replace('+', " "));
            }
        }
        None
    }

    pub fn has_param(&self, name: &str) -> bool {
        self.query.split('&').any(|p| p.split('=').next().and_then(percent_decode).as_deref() == Some(name))
    }
}

#[derive(Debug)]
pub enum ReadError {
    /// a conexao caiu ou o tempo acabou
    Io(io::Error),
    /// o pedido nao e HTTP que este servidor le (400)
    Bad,
    /// cabecalho grande demais (431)
    TooLarge,
}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        ReadError::Io(e)
    }
}

pub struct Conn {
    pub stream: TcpStream,
    buf: Vec<u8>,
    start: usize,
    end: usize,
}

impl Conn {
    pub fn new(stream: TcpStream) -> Conn {
        Conn { stream, buf: vec![0u8; MAX_HEAD + 4096], start: 0, end: 0 }
    }

    /// O proximo pedido.  Ok(None): o cliente fechou a conexao entre dois pedidos.
    pub fn read_request(&mut self) -> Result<Option<Request>, ReadError> {
        loop {
            if let Some(pos) = find_head_end(&self.buf[self.start..self.end]) {
                let head_end = self.start + pos; // inicio de "\r\n\r\n"
                let req = parse_head(&self.buf[self.start..head_end]);
                self.start = head_end + 4;
                if self.start == self.end {
                    self.start = 0;
                    self.end = 0;
                }
                return req.map(Some);
            }
            if self.end - self.start >= MAX_HEAD {
                return Err(ReadError::TooLarge);
            }
            // espaco no fim do buffer: o que sobrou vai para o inicio
            if self.end == self.buf.len() {
                self.buf.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            let n = match self.stream.read(&mut self.buf[self.end..]) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(ReadError::Io(e)),
            };
            if n == 0 {
                if self.start == self.end {
                    return Ok(None);
                }
                return Err(ReadError::Io(io::ErrorKind::UnexpectedEof.into()));
            }
            self.end += n;
        }
    }

    /// Escreve a linha de status e os cabecalhos.  `len`: Content-Length (None: sem corpo e sem Content-Length).
    pub fn write_head(&mut self, status: u16, headers: &[(&str, String)], len: Option<u64>, keep_alive: bool) -> io::Result<()> {
        let mut h = String::with_capacity(512);
        h.push_str("HTTP/1.1 ");
        h.push_str(&status.to_string());
        h.push(' ');
        h.push_str(reason(status));
        h.push_str("\r\n");
        for (k, v) in crate::routes::COMMON_HEADERS {
            push_header(&mut h, k, v);
        }
        for (k, v) in headers {
            push_header(&mut h, k, v);
        }
        if let Some(n) = len {
            push_header(&mut h, "Content-Length", &n.to_string());
        }
        push_header(&mut h, "Connection", if keep_alive { "keep-alive" } else { "close" });
        h.push_str("\r\n");
        self.stream.write_all(h.as_bytes())
    }

    pub fn write_body(&mut self, b: &[u8]) -> io::Result<()> {
        self.stream.write_all(b)
    }
}

fn push_header(h: &mut String, k: &str, v: &str) {
    // um valor nunca quebra a linha (nomes e valores vem deste codigo, mas a defesa custa pouco)
    let clean: String = v.chars().filter(|&c| c != '\r' && c != '\n').collect();
    h.push_str(k);
    h.push_str(": ");
    h.push_str(&clean);
    h.push_str("\r\n");
}

fn find_head_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_head(head: &[u8]) -> Result<Request, ReadError> {
    let text = std::str::from_utf8(head).map_err(|_| ReadError::Bad)?;
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or(ReadError::Bad)?;
    let mut parts = first.split(' ');
    let method = parts.next().filter(|m| !m.is_empty() && m.bytes().all(|c| c.is_ascii_uppercase())).ok_or(ReadError::Bad)?;
    let target = parts.next().filter(|t| !t.is_empty()).ok_or(ReadError::Bad)?;
    let version = parts.next().ok_or(ReadError::Bad)?;
    if parts.next().is_some() {
        return Err(ReadError::Bad);
    }
    let http11 = match version {
        "HTTP/1.1" => true,
        "HTTP/1.0" => false,
        _ => return Err(ReadError::Bad),
    };
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once(':').ok_or(ReadError::Bad)?;
        if k.is_empty() || k.bytes().any(|c| c <= b' ' || c >= 0x7f) {
            return Err(ReadError::Bad);
        }
        headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
    }
    let (raw_path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q.to_string()),
        None => (target, String::new()),
    };
    // forma de origem ("/x") apenas; "*" e URLs absolutas nao sao servidas aqui
    let path = if raw_path.starts_with('/') { decode_path(raw_path).unwrap_or_default() } else { String::new() };
    Ok(Request { method: method.to_string(), target: target.to_string(), path, query, http11, headers })
}

/// O caminho decodificado, ou None se ele tenta algo que nenhuma rota aceita: separador codificado (%2f, %5c),
/// caractere de controle, barra invertida, "//", segmento "." ou "..".
pub fn decode_path(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("%2f") || lower.contains("%5c") {
        return None;
    }
    let p = percent_decode(raw)?;
    if p.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}' || c == '\\') || p.contains("//") {
        return None;
    }
    if p.split('/').any(|s| s == "." || s == "..") {
        return None;
    }
    Some(p)
}

/// %XX decodificado como UTF-8 (None: escape malformado ou bytes que nao sao UTF-8)
pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            // (um "%" sem os dois digitos depois: get devolve None)
            let h = hex(*b.get(i + 1)?)?;
            let l = hex(*b.get(i + 2)?)?;
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        416 => "Range Not Satisfiable",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// O intervalo pedido em "Range" para um arquivo de `size` bytes.
#[derive(Debug, PartialEq, Eq)]
pub enum Range {
    /// sem Range, ou um Range que se ignora (varios intervalos, outra unidade, sintaxe invalida): o arquivo inteiro
    Full,
    /// [inicio, fim) — fim exclusivo
    Part(u64, u64),
    /// fora do arquivo: 416
    Unsatisfiable,
}

pub fn parse_range(h: Option<&str>, size: u64) -> Range {
    let Some(h) = h else { return Range::Full };
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return Range::Full };
    let spec = spec.trim();
    if spec.contains(',') {
        return Range::Full;
    }
    let Some((a, b)) = spec.split_once('-') else { return Range::Full };
    let (a, b) = (a.trim(), b.trim());
    let num = |x: &str| -> Option<u64> {
        if x.is_empty() || x.len() > 19 || !x.bytes().all(|c| c.is_ascii_digit()) {
            None
        } else {
            x.parse().ok()
        }
    };
    if a.is_empty() {
        // sufixo: os ultimos n bytes
        let Some(n) = num(b) else { return Range::Full };
        if n == 0 || size == 0 {
            return Range::Unsatisfiable;
        }
        return Range::Part(size - n.min(size), size);
    }
    let Some(start) = num(a) else { return Range::Full };
    if b.is_empty() {
        return if start < size { Range::Part(start, size) } else { Range::Unsatisfiable };
    }
    let Some(last) = num(b) else { return Range::Full };
    if last < start {
        return Range::Full;
    }
    if start >= size {
        return Range::Unsatisfiable;
    }
    Range::Part(start, last.min(size - 1) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=0-9"), 100), Range::Part(0, 10));
        assert_eq!(parse_range(Some("bytes=90-"), 100), Range::Part(90, 100));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Range::Part(90, 100));
        assert_eq!(parse_range(Some("bytes=-1000"), 100), Range::Part(0, 100));
        assert_eq!(parse_range(Some("bytes=50-1000"), 100), Range::Part(50, 100));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=100-200"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-0"), 0), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), Range::Full);
        assert_eq!(parse_range(Some("items=0-1"), 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=9-1"), 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=x-1"), 100), Range::Full);
    }

    #[test]
    fn paths() {
        assert_eq!(decode_path("/a%20b/c").as_deref(), Some("/a b/c"));
        assert_eq!(decode_path("/data/%C3%A9t%C3%A9").as_deref(), Some("/data/été"));
        assert_eq!(decode_path("/a%2fb"), None);
        assert_eq!(decode_path("/a%5Cb"), None);
        assert_eq!(decode_path("/a/../b"), None);
        assert_eq!(decode_path("/a/%2e%2e/b"), None);
        assert_eq!(decode_path("/a/./b"), None);
        assert_eq!(decode_path("/a//b"), None);
        assert_eq!(decode_path("/a%00b"), None);
        assert_eq!(decode_path("/a%zzb"), None);
        assert_eq!(decode_path("/a%e"), None);
        assert_eq!(decode_path("/a%"), None);
        assert_eq!(decode_path("/%ff"), None);
    }

    #[test]
    fn head_parsing() {
        let r = parse_head(b"GET /x?b=1&n=8 HTTP/1.1\r\nHost: 127.0.0.1:1\r\nRange: bytes=0-1").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/x");
        assert_eq!(r.param("b").as_deref(), Some("1"));
        assert_eq!(r.param("n").as_deref(), Some("8"));
        assert!(r.has_param("n"));
        assert!(!r.has_param("m"));
        assert_eq!(r.header("range"), Some("bytes=0-1"));
        assert!(r.keep_alive());
        let r = parse_head(b"GET / HTTP/1.0\r\nConnection: Keep-Alive").unwrap();
        assert!(r.keep_alive());
        let r = parse_head(b"GET / HTTP/1.1\r\nConnection: close").unwrap();
        assert!(!r.keep_alive());
        assert!(parse_head(b"get / HTTP/1.1").is_err());
        assert!(parse_head(b"GET / HTTP/2").is_err());
        assert!(parse_head(b"GET  / HTTP/1.1").is_err());
        assert!(parse_head(b"GET / HTTP/1.1\r\nBad Header: x").is_err());
        let r = parse_head(b"GET http://x/ HTTP/1.1").unwrap();
        assert_eq!(r.path, "");
    }
}
