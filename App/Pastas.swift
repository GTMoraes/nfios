import Foundation
import CryptoKit

/// Onde ficam as coisas do modo offline.  Tudo em Documentos, que aparece no app Arquivos (No Meu iPhone › NFSU2)
/// e no app Dispositivos Apple do Windows (Compartilhamento de Arquivos):
///
///   Documentos/Jogo       a sua pasta do NFSU2 (SPEED2.EXE, CARS, TRACKS, ...), fora do backup do iCloud
///   Documentos/Pacote     a página + o módulo feitos do seu SPEED2.EXE (web/, game.wasm, game.json)
///   Documentos/Downloads  o que a página baixa (log, capturas de tela, backups de saves)
enum Pastas {
    static var documentos: URL { FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0] }
    static var jogo: URL { documentos.appendingPathComponent("Jogo", isDirectory: true) }
    static var pacote: URL { documentos.appendingPathComponent("Pacote", isDirectory: true) }
    static var downloads: URL { documentos.appendingPathComponent("Downloads", isDirectory: true) }
    /// onde uma importação monta a pasta antes de trocar a antiga (o ponto a esconde do app Arquivos)
    static var obra: URL { documentos.appendingPathComponent(".importando", isDirectory: true) }

    /// O SPEED2.EXE da v1.2 americana, o único com que o projeto funciona (README do nfsu2-web).
    static let exeTamanho: Int64 = 4_800_512
    static let exeSHA256 = "f9dd86c054878ce6276beb07c1fd61874f7a1e4bf1f241b084c65b73e24168a7"

    static func preparar() {
        let fm = FileManager.default
        try? fm.createDirectory(at: downloads, withIntermediateDirectories: true)
        // o que sobrou de uma importação interrompida (app fechado no meio)
        try? fm.removeItem(at: obra)
        excluirDoBackup(jogo)
        excluirDoBackup(pacote)
    }

    /// os ~2,5 GB do jogo não vão para o backup do iCloud (a pasta original continua no seu PC)
    static func excluirDoBackup(_ u: URL) {
        guard FileManager.default.fileExists(atPath: u.path) else { return }
        var url = u
        var v = URLResourceValues()
        v.isExcludedFromBackup = true
        try? url.setResourceValues(v)
    }

    /// Tamanho total e quantidade de arquivos de uma pasta (sem seguir links).
    static func medir(_ pasta: URL) -> (bytes: Int64, arquivos: Int) {
        let fm = FileManager.default
        guard let e = fm.enumerator(at: pasta, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey], options: []) else {
            return (0, 0)
        }
        var bytes: Int64 = 0
        var n = 0
        for case let u as URL in e {
            let v = try? u.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
            if v?.isRegularFile == true {
                bytes += Int64(v?.fileSize ?? 0)
                n += 1
            }
        }
        return (bytes, n)
    }

    /// SHA-256 de um arquivo, em hexadecimal (lido em blocos: não carrega o arquivo inteiro na memória)
    static func sha256(_ u: URL) -> String? {
        guard let h = try? FileHandle(forReadingFrom: u) else { return nil }
        defer { try? h.close() }
        var hasher = SHA256()
        while true {
            guard let bloco = try? h.read(upToCount: 1 << 20), !bloco.isEmpty else { break }
            hasher.update(data: bloco)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// o SPEED2.EXE da pasta do jogo (o nome em qualquer caixa)
    static func exe(em pasta: URL) -> URL? {
        guard let nomes = try? FileManager.default.contentsOfDirectory(atPath: pasta.path) else { return nil }
        return nomes.first { $0.lowercased() == "speed2.exe" }.map { pasta.appendingPathComponent($0) }
    }
}

/// O que o modo offline tem agora: pacote (página + módulo) e pasta do jogo.
struct EstadoOffline: Equatable {
    var pacotePronto = false
    var pacoteRev: String?
    /// o SHA-256 do SPEED2.EXE de que o pacote foi feito (pacote.json, escrito pelo montar-pacote.sh)
    var pacoteExe: String?
    var pacoteBytes: Int64 = 0
    var jogoPronto = false
    var jogoBytes: Int64 = 0
    var jogoArquivos = 0
    var exeSHA: String?

    var exeOficial: Bool { exeSHA == Pastas.exeSHA256 }
    /// o pacote foi feito de outro SPEED2.EXE (só dá para saber quando o pacote diz de qual)
    var exeDiferenteDoPacote: Bool {
        guard let a = pacoteExe, let b = exeSHA else { return false }
        return a.lowercased() != b.lowercased()
    }
    var podeJogar: Bool { pacotePronto && jogoPronto }

    /// Lê o estado das pastas (alguns milissegundos; o hash do SPEED2.EXE fica guardado enquanto o arquivo não muda).
    static func ler() -> EstadoOffline {
        var e = EstadoOffline()
        let fm = FileManager.default
        let p = Pastas.pacote
        e.pacotePronto = fm.fileExists(atPath: p.appendingPathComponent("web/index.html").path)
            && fm.fileExists(atPath: p.appendingPathComponent("game.wasm").path)
            && fm.fileExists(atPath: p.appendingPathComponent("game.json").path)
        if let d = try? Data(contentsOf: p.appendingPathComponent("game.json")),
           let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any] {
            e.pacoteRev = j["rev"] as? String
        }
        if let d = try? Data(contentsOf: p.appendingPathComponent("pacote.json")),
           let j = try? JSONSerialization.jsonObject(with: d) as? [String: Any] {
            e.pacoteExe = j["exe_sha256"] as? String
            if let r = j["rev"] as? String { e.pacoteRev = r }
        }
        if e.pacotePronto { e.pacoteBytes = Pastas.medir(p).bytes }
        if let exe = Pastas.exe(em: Pastas.jogo) {
            e.jogoPronto = true
            let m = Pastas.medir(Pastas.jogo)
            e.jogoBytes = m.bytes
            e.jogoArquivos = m.arquivos
            e.exeSHA = hashGuardado(exe)
        }
        return e
    }

    /// o SHA-256 do exe, guardado com o tamanho e a data dele (refeito só quando o arquivo muda)
    private static func hashGuardado(_ exe: URL) -> String? {
        let v = try? exe.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey])
        let chave = "\(v?.fileSize ?? -1)-\(v?.contentModificationDate?.timeIntervalSince1970 ?? 0)"
        let ud = UserDefaults.standard
        if ud.string(forKey: "exe.chave") == chave, let h = ud.string(forKey: "exe.sha256") { return h }
        guard let h = Pastas.sha256(exe) else { return nil }
        ud.set(chave, forKey: "exe.chave")
        ud.set(h, forKey: "exe.sha256")
        return h
    }
}
