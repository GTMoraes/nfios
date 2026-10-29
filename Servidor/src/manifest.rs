//! O manifesto da pasta do jogo (/data-manifest), no formato do servidor oficial (web/server-rs/src/app/manifest.rs):
//!
//!   {"v":1,"files":[{"p":"CARS","d":1},{"p":"CARS/x.bin","s":123,"m":1700000000},...],
//!    "blocks":{"url":"/data-blocks/v2/","size":1048576,"max":8}}
//!
//! Só as entradas do jogo no primeiro nível (SPEED2.EXE, server.dll, server.cfg e as pastas de dados, comparadas sem
//! diferenciar maiúsculas), na ordem dos bytes do nome (a do readdir do Node, que o oficial reproduz), sem arquivos
//! que começam com ponto e sem links simbólicos.  "m" é o mtime em segundos inteiros, "s" o tamanho.

use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// as entradas do primeiro nível que são do jogo (as mesmas do oficial)
pub const GAME_ENTRIES: [&str; 18] = [
    "speed2.exe",
    "server.dll",
    "server.cfg",
    "cars",
    "credits",
    "frontend",
    "global",
    "langpacks",
    "languages",
    "memcard",
    "movies",
    "nis",
    "pfdata",
    "sound",
    "speech",
    "subtitles",
    "texpacks",
    "tracks",
];

pub const BLOCK: u64 = 1 << 20;
pub const BLOCKS_MAX: u64 = 8;

pub fn game_entry(name: &str) -> bool {
    let n = name.to_lowercase();
    GAME_ENTRIES.contains(&n.as_str())
}

pub struct Manifest {
    pub body: Vec<u8>,
    /// arquivos (sem as pastas)
    pub files: usize,
    /// soma dos tamanhos
    pub bytes: u64,
    /// se a pasta tem SPEED2.EXE no primeiro nível
    pub has_exe: bool,
}

fn entries(dir: &Path) -> io::Result<Vec<(Vec<u8>, String, fs::Metadata)>> {
    let mut v = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        // nome que não é UTF-8 não tem como ser pedido pela página: fica de fora
        let Some(s) = name.to_str() else { continue };
        // symlink_metadata: um link simbólico nunca é seguido
        let md = fs::symlink_metadata(e.path())?;
        v.push((name.as_bytes().to_vec(), s.to_string(), md));
    }
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(v)
}

fn walk(dir: &Path, rel: &str, out: &mut String, count: &mut usize, m: &mut Manifest) -> io::Result<()> {
    for (_, name, md) in entries(dir)? {
        if name.starts_with('.') || (rel.is_empty() && !game_entry(&name)) {
            continue;
        }
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let ft = md.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if *count > 0 {
                out.push(',');
            }
            out.push_str("{\"p\":");
            write_str(out, &r);
            out.push_str(",\"d\":1}");
            *count += 1;
            walk(&dir.join(&name), &r, out, count, m)?;
        } else if ft.is_file() {
            if *count > 0 {
                out.push(',');
            }
            out.push_str("{\"p\":");
            write_str(out, &r);
            out.push_str(",\"s\":");
            out.push_str(&md.size().to_string());
            out.push_str(",\"m\":");
            out.push_str(&mtime_s(&md).to_string());
            out.push('}');
            *count += 1;
            m.files += 1;
            m.bytes += md.size();
            if rel.is_empty() && name.eq_ignore_ascii_case("speed2.exe") {
                m.has_exe = true;
            }
        }
    }
    Ok(())
}

/// mtime em segundos como o oficial (e o Node antes dele) calcula: floor(mtimeMs / 1000), com mtimeMs =
/// seg * 1e3 + nseg / 1e6 em double.  Um horário logo abaixo de um segundo inteiro vira o segundo seguinte (o
/// double arredonda para cima): 1700000000.999999999 dá 1700000001, como no oficial.
pub fn mtime_s(md: &fs::Metadata) -> i64 {
    let ms = md.mtime() as f64 * 1000.0 + md.mtime_nsec() as f64 / 1e6;
    (ms / 1000.0).floor() as i64
}

pub fn build(data: &Path) -> io::Result<Manifest> {
    let mut m = Manifest { body: Vec::new(), files: 0, bytes: 0, has_exe: false };
    let mut body = String::from("{\"v\":1,\"files\":[");
    let mut count = 0usize;
    walk(data, "", &mut body, &mut count, &mut m)?;
    body.push_str(&format!("],\"blocks\":{{\"url\":\"/data-blocks/v2/\",\"size\":{BLOCK},\"max\":{BLOCKS_MAX}}}}}"));
    m.body = body.into_bytes();
    Ok(m)
}

/// Uma string JSON como JSON.stringify a escreve: aspas, barra invertida e os controles escapados; o resto como é.
pub fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings() {
        let mut s = String::new();
        write_str(&mut s, "a\"b\\c\n\u{1}é");
        assert_eq!(s, "\"a\\\"b\\\\c\\n\\u0001é\"");
    }
}
