import Foundation
import Observation

/// Copia para Documentos a pasta do jogo e/ou a pasta do pacote escolhida no app Arquivos (iCloud Drive, No Meu
/// iPhone, um pendrive USB-C, um compartilhamento do PC pelo "Conectar ao Servidor"...).
///
/// A pasta escolhida pode ser a do jogo (tem SPEED2.EXE), a do pacote (tem game.wasm e web/) ou uma pasta que tenha
/// as duas dentro.  Da pasta do jogo só vem o que o jogo lê (as mesmas entradas que o servidor oficial serve): nunca
/// saves, mods, desinstaladores.  A cópia é montada em Documentos/.importando e só troca a antiga no fim.
@Observable
final class Importador: @unchecked Sendable {   // o que a tela lê muda só na thread principal; o cancelar tem trava
    enum Tipo: String { case jogo = "a pasta do jogo", pacote = "o pacote" }

    private(set) var ativo = false
    private(set) var titulo = ""
    private(set) var arquivoAtual = ""
    private(set) var feitos: Int64 = 0
    private(set) var total: Int64 = 0
    var erro: String?
    var resultado: String?

    private let trava = NSLock()
    @ObservationIgnored private var pediuCancelar = false
    private var cancelado: Bool {
        trava.lock(); defer { trava.unlock() }
        return pediuCancelar
    }

    /// as entradas do primeiro nível que são do jogo (Servidor/src/manifest.rs, GAME_ENTRIES)
    static let entradasDoJogo: Set<String> = [
        "speed2.exe", "server.dll", "server.cfg", "cars", "credits", "frontend", "global", "langpacks", "languages",
        "memcard", "movies", "nis", "pfdata", "sound", "speech", "subtitles", "texpacks", "tracks",
    ]
    /// do pacote: a página e o módulo
    static let entradasDoPacote: Set<String> = ["web", "game.wasm", "game.json", "pacote.json"]

    enum Falha: LocalizedError {
        case nadaReconhecido
        case soNaNuvem(String)
        case semEspaco(precisa: Int64, livre: Int64)
        case leitura(String)
        case escrita(String)
        case cancelado
        case semServidor
        case servidor(String)
        case listaInvalida
        case corrompido(String)

        var errorDescription: String? {
            switch self {
            case .nadaReconhecido:
                return "Essa pasta não tem SPEED2.EXE (pasta do jogo) nem game.wasm + web/ (pacote), nem dentro de uma subpasta."
            case .soNaNuvem(let n):
                return "\"\(n)\" ainda está só na nuvem. No app Arquivos, toque e segure a pasta › Baixar Agora, espere terminar e importe de novo."
            case let .semEspaco(precisa, livre):
                return "Falta espaço: precisa de \(tamanhoLegivel(precisa)) e há \(tamanhoLegivel(livre)) livres."
            case .leitura(let n): return "Não consegui ler \"\(n)\"."
            case .escrita(let n): return "Não consegui gravar \"\(n)\"."
            case .cancelado: return "Cancelado."
            case .semServidor:
                return "Ponha o endereço do seu servidor em Ajustes › Meu servidor (ex.: http://100.91.24.1:9091)."
            case .servidor(let m): return "O servidor não respondeu (\(m)). O Tailscale está ligado no iPhone?"
            case .listaInvalida: return "A lista de arquivos do servidor (manifesto.json) veio inválida. Rode o preparar.sh de novo no servidor."
            case .corrompido(let n): return "\"\(n)\" chegou diferente do que o servidor diz. Tente de novo."
            }
        }
    }

    @ObservationIgnored private var baixador: Baixador?

    func cancelar() {
        trava.lock(); pediuCancelar = true; let b = baixador; trava.unlock()
        b?.cancelarAtual()
    }

    /// Importa o que houver na pasta escolhida.  `ativo` volta a false no fim (a tela observa).
    func importar(_ origem: URL) {
        guard !ativo else { return }
        trava.lock(); pediuCancelar = false; trava.unlock()
        ativo = true
        erro = nil
        resultado = nil
        titulo = "Lendo a pasta…"
        arquivoAtual = ""
        feitos = 0
        total = 0
        DispatchQueue.global(qos: .userInitiated).async {
            let acesso = origem.startAccessingSecurityScopedResource()
            defer { if acesso { origem.stopAccessingSecurityScopedResource() } }
            var feitas: [Tipo] = []
            var falha: Error?
            do {
                let planos = try Self.planejar(origem)
                for (tipo, raiz) in planos {
                    try self.copiar(tipo, de: raiz)
                    feitas.append(tipo)
                }
            } catch {
                falha = error
            }
            try? FileManager.default.removeItem(at: Pastas.obra)
            Pastas.excluirDoBackup(Pastas.jogo)
            Pastas.excluirDoBackup(Pastas.pacote)
            // o servidor lê o manifesto e o módulo de novo
            ServidorLocal.reiniciar()
            let mensagemDeErro = falha.map { ($0 as? LocalizedError)?.errorDescription ?? $0.localizedDescription }
            let importadas = feitas.map(\.rawValue).joined(separator: " e ")
            DispatchQueue.main.async {
                self.ativo = false
                if var m = mensagemDeErro {
                    if !importadas.isEmpty { m += " (já importado: \(importadas))" }
                    self.erro = m
                } else {
                    self.resultado = "Importado: \(importadas)."
                }
            }
        }
    }

    // MARK: - baixar do servidor próprio

    /// a lista que o preparar.sh do servidor monta (servidor-gt-srv/preparar.sh)
    private struct Manifesto: Decodable {
        struct Item: Decodable { let p: String; let s: Int64; let h: String }
        let v: Int
        let rev: String?
        let arquivos: [Item]
    }

    /// onde o download monta as pastas: fica entre uma tentativa e outra (o que já chegou e conferiu não baixa de
    /// novo) e só troca as pastas antigas no fim
    static var baixando: URL { Pastas.documentos.appendingPathComponent(".baixando", isDirectory: true) }

    /// Baixa o jogo e o pacote do seu servidor (http://<ip>:<porta>/arquivos/), conferindo o tamanho e o SHA-256 de
    /// cada arquivo.  `ativo` volta a false no fim, como na importação.
    func baixar(de base: URL) {
        guard !ativo else { return }
        trava.lock(); pediuCancelar = false; trava.unlock()
        ativo = true
        erro = nil
        resultado = nil
        titulo = "Pedindo a lista ao servidor…"
        arquivoAtual = ""
        feitos = 0
        total = 0
        Task.detached(priority: .userInitiated) {
            var falha: Error?
            var feitas: [Tipo] = []
            do {
                feitas = try await self.baixarTudo(base)
            } catch {
                falha = error
            }
            self.trava.lock(); self.baixador = nil; self.trava.unlock()
            Pastas.excluirDoBackup(Pastas.jogo)
            Pastas.excluirDoBackup(Pastas.pacote)
            ServidorLocal.reiniciar()
            let mensagemDeErro = falha.map { ($0 as? LocalizedError)?.errorDescription ?? $0.localizedDescription }
            let baixadas = feitas.map(\.rawValue).joined(separator: " e ")
            DispatchQueue.main.async {
                self.ativo = false
                if let m = mensagemDeErro {
                    self.erro = m + " O que já chegou fica guardado: tocar de novo continua de onde parou."
                } else {
                    self.resultado = "Baixado: \(baixadas)."
                }
            }
        }
    }

    private func baixarTudo(_ base: URL) async throws -> [Tipo] {
        let fm = FileManager.default
        let b = Baixador()
        trava.lock(); baixador = b; trava.unlock()
        defer { b.encerrar() }
        let raiz = base.appendingPathComponent("arquivos", isDirectory: true)

        // a lista
        let listaTemp = fm.temporaryDirectory.appendingPathComponent("manifesto-\(UUID().uuidString).json")
        do {
            try await b.baixar(raiz.appendingPathComponent("manifesto.json"), para: listaTemp)
        } catch let e as Falha {
            throw e
        } catch {
            throw Falha.servidor(error.localizedDescription)
        }
        defer { try? fm.removeItem(at: listaTemp) }
        guard let dados = try? Data(contentsOf: listaTemp),
              let m = try? JSONDecoder().decode(Manifesto.self, from: dados), m.v == 1, !m.arquivos.isEmpty,
              m.arquivos.allSatisfy({ Self.caminhoValido($0.p) && $0.s >= 0 && $0.h.count == 64 }) else {
            throw Falha.listaInvalida
        }

        // espaço: o que falta baixar, mais 300 MB de folga (as pastas antigas só saem no fim)
        let obra = Self.baixando
        try? fm.createDirectory(at: obra, withIntermediateDirectories: true)
        Pastas.excluirDoBackup(obra)
        func pronto(_ i: Manifesto.Item) -> Bool {
            let v = try? obra.appendingPathComponent(i.p).resourceValues(forKeys: [.fileSizeKey])
            return v?.fileSize.map(Int64.init) == i.s
        }
        let soma = m.arquivos.reduce(Int64(0)) { $0 + $1.s }
        let falta = m.arquivos.filter { !pronto($0) }.reduce(Int64(0)) { $0 + $1.s }
        let livre = (try? Pastas.documentos.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]))?
            .volumeAvailableCapacityForImportantUsage ?? Int64.max
        if falta + 300_000_000 > livre { throw Falha.semEspaco(precisa: falta + 300_000_000, livre: livre) }

        var feitos = soma - falta
        let f0 = feitos
        let titulo = m.rev.map { "Baixando do seu servidor (rev \($0))" } ?? "Baixando do seu servidor"
        DispatchQueue.main.async {
            self.titulo = titulo
            self.total = soma
            self.feitos = f0
        }
        for item in m.arquivos {
            if cancelado { throw Falha.cancelado }
            if pronto(item) { continue }
            let nome = item.p
            DispatchQueue.main.async { self.arquivoAtual = nome }
            let destino = obra.appendingPathComponent(item.p)
            let temp = obra.appendingPathComponent(".parcial", isDirectory: false)
            try? fm.removeItem(at: temp)
            let antes = feitos
            b.progresso = { escritos in
                let f = antes + escritos
                DispatchQueue.main.async { self.feitos = f }
            }
            do {
                try await b.baixar(Self.endereco(raiz, item.p), para: temp)
            } catch let e as Falha {
                throw e
            } catch {
                if cancelado { throw Falha.cancelado }
                throw Falha.servidor(error.localizedDescription)
            }
            feitos = antes + item.s
            let tam = (try? temp.resourceValues(forKeys: [.fileSizeKey]))?.fileSize.map(Int64.init)
            guard tam == item.s, Pastas.sha256(temp)?.lowercased() == item.h.lowercased() else {
                try? fm.removeItem(at: temp)
                throw Falha.corrompido(item.p)
            }
            do {
                try fm.createDirectory(at: destino.deletingLastPathComponent(), withIntermediateDirectories: true)
                try? fm.removeItem(at: destino)
                try fm.moveItem(at: temp, to: destino)
            } catch {
                throw Falha.escrita(item.p)
            }
        }
        let fTotal = feitos
        DispatchQueue.main.async { self.feitos = fTotal; self.titulo = "Concluindo…"; self.arquivoAtual = "" }

        // troca as pastas (mesmo volume: um rename cada)
        var feitas: [Tipo] = []
        for (tipo, nome, alvo) in [(Tipo.pacote, "Pacote", Pastas.pacote), (Tipo.jogo, "Jogo", Pastas.jogo)] {
            let pasta = obra.appendingPathComponent(nome, isDirectory: true)
            guard fm.fileExists(atPath: pasta.path) else { continue }
            try? fm.removeItem(at: alvo)
            do { try fm.moveItem(at: pasta, to: alvo) } catch { throw Falha.escrita(nome) }
            feitas.append(tipo)
        }
        try? fm.removeItem(at: obra)
        return feitas
    }

    /// "Jogo/..." ou "Pacote/...", sem "..", sem barra invertida, e a primeira pasta tem que ser do jogo ou do pacote
    static func caminhoValido(_ p: String) -> Bool {
        let partes = p.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        guard partes.count >= 2, !p.contains("\\"), !p.hasPrefix("/"),
              partes.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." && !$0.hasPrefix(".") }) else { return false }
        switch partes[0] {
        case "Jogo": return entradasDoJogo.contains(partes[1].lowercased())
        case "Pacote": return entradasDoPacote.contains(partes[1].lowercased())
        default: return false
        }
    }

    /// o endereço de um arquivo, cada parte do caminho codificada (espaços e acentos nos nomes do jogo)
    static func endereco(_ raiz: URL, _ p: String) -> URL {
        var u = raiz
        for parte in p.split(separator: "/") { u.appendPathComponent(String(parte), isDirectory: false) }
        return u
    }

    // MARK: - o que tem na pasta

    private static func nomes(_ pasta: URL) -> [String] {
        (try? FileManager.default.contentsOfDirectory(atPath: pasta.path)) ?? []
    }

    static func tipo(de pasta: URL) -> Tipo? {
        let n = Set(nomes(pasta).map { $0.lowercased() })
        if n.contains("speed2.exe") { return .jogo }
        if n.contains("game.wasm") && n.contains("web") { return .pacote }
        return nil
    }

    /// a pasta escolhida e as subpastas dela (um nível) que são jogo ou pacote: a pasta do jogo com o pacote-nfsu2
    /// dentro (como o montar-pacote.sh deixa) traz as duas coisas numa importação só
    static func planejar(_ origem: URL) throws -> [(Tipo, URL)] {
        var achados: [Tipo: URL] = [:]
        if let t = tipo(de: origem) { achados[t] = origem }
        for nome in nomes(origem).sorted() where !nome.hasPrefix(".") {
            let u = origem.appendingPathComponent(nome, isDirectory: true)
            var dir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: u.path, isDirectory: &dir), dir.boolValue else { continue }
            if let t = tipo(de: u), achados[t] == nil { achados[t] = u }
        }
        // o pacote primeiro (pequeno): se o jogo falhar no meio, o pacote já ficou
        let ordem: [Tipo] = [.pacote, .jogo]
        let planos = ordem.compactMap { t in achados[t].map { (t, $0) } }
        if planos.isEmpty { throw Falha.nadaReconhecido }
        return planos
    }

    // MARK: - cópia

    private struct Item { let origem: URL; let relativo: String; let tamanho: Int64; let pasta: Bool }

    /// a lista do que copiar (só as entradas de primeiro nível do tipo; sem arquivos com ponto e sem links)
    private static func listar(_ tipo: Tipo, raiz: URL) throws -> [Item] {
        let permitidas = tipo == .jogo ? entradasDoJogo : entradasDoPacote
        var itens: [Item] = []
        let chaves: [URLResourceKey] = [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]
        func visitar(_ u: URL, _ rel: String) throws {
            let v = try? u.resourceValues(forKeys: Set(chaves))
            if v?.isSymbolicLink == true { return }
            if v?.isDirectory == true {
                itens.append(Item(origem: u, relativo: rel, tamanho: 0, pasta: true))
                for nome in nomes(u).sorted() {
                    if nome.hasPrefix(".") {
                        // um arquivo do iCloud que não está no aparelho aparece como ".nome.icloud"
                        if nome.hasSuffix(".icloud") { throw Falha.soNaNuvem(String(nome.dropFirst().dropLast(7))) }
                        continue
                    }
                    try visitar(u.appendingPathComponent(nome), rel + "/" + nome)
                }
            } else if v?.isRegularFile == true {
                itens.append(Item(origem: u, relativo: rel, tamanho: Int64(v?.fileSize ?? 0), pasta: false))
            }
        }
        for nome in nomes(raiz).sorted() {
            if nome.hasPrefix(".") {
                if nome.hasSuffix(".icloud") && permitidas.contains(String(nome.dropFirst().dropLast(7)).lowercased()) {
                    throw Falha.soNaNuvem(String(nome.dropFirst().dropLast(7)))
                }
                continue
            }
            guard permitidas.contains(nome.lowercased()) else { continue }
            try visitar(raiz.appendingPathComponent(nome), nome)
        }
        return itens
    }

    private func copiar(_ tipo: Tipo, de raiz: URL) throws {
        let fm = FileManager.default
        DispatchQueue.main.async { self.titulo = "Verificando \(tipo.rawValue)…" }
        let itens = try Self.listar(tipo, raiz: raiz)
        let soma = itens.reduce(Int64(0)) { $0 + $1.tamanho }
        // espaço: o que entra, menos o que sai (a pasta antiga só sai no fim), com folga de 300 MB
        let livre = (try? Pastas.documentos.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]))?
            .volumeAvailableCapacityForImportantUsage ?? Int64.max
        if soma + 300_000_000 > livre { throw Falha.semEspaco(precisa: soma + 300_000_000, livre: livre) }

        let obra = Pastas.obra.appendingPathComponent(tipo == .jogo ? "Jogo" : "Pacote", isDirectory: true)
        try? fm.removeItem(at: obra)
        do { try fm.createDirectory(at: obra, withIntermediateDirectories: true) } catch { throw Falha.escrita(obra.lastPathComponent) }
        DispatchQueue.main.async {
            self.titulo = tipo == .jogo ? "Copiando a pasta do jogo" : "Copiando o pacote"
            self.feitos = 0
            self.total = soma
        }
        var feitos: Int64 = 0
        var ultimoAviso = Date.distantPast
        for item in itens {
            if cancelado { throw Falha.cancelado }
            let destino = obra.appendingPathComponent(item.relativo)
            if item.pasta {
                do { try fm.createDirectory(at: destino, withIntermediateDirectories: true) } catch { throw Falha.escrita(item.relativo) }
                continue
            }
            let nome = item.relativo
            DispatchQueue.main.async { self.arquivoAtual = nome }
            try copiarArquivo(item.origem, destino, nome: item.relativo) { n in
                feitos += n
                if Date().timeIntervalSince(ultimoAviso) > 0.1 {
                    ultimoAviso = Date()
                    let f = feitos
                    DispatchQueue.main.async { self.feitos = f }
                }
            }
        }
        let copiados = feitos
        DispatchQueue.main.async { self.feitos = copiados; self.titulo = "Concluindo…" }
        // troca a pasta antiga pela nova (mesmo volume: um rename)
        let alvo = tipo == .jogo ? Pastas.jogo : Pastas.pacote
        try? fm.removeItem(at: alvo)
        do { try fm.moveItem(at: obra, to: alvo) } catch { throw Falha.escrita(alvo.lastPathComponent) }
    }

    /// Copia um arquivo em blocos de 4 MB (progresso dentro dos arquivos grandes, como os vídeos), mantendo a data.
    private func copiarArquivo(_ origem: URL, _ destino: URL, nome: String, progresso: (Int64) -> Void) throws {
        let fm = FileManager.default
        guard let entrada = try? FileHandle(forReadingFrom: origem) else { throw Falha.leitura(nome) }
        defer { try? entrada.close() }
        guard fm.createFile(atPath: destino.path, contents: nil), let saida = try? FileHandle(forWritingTo: destino) else {
            throw Falha.escrita(nome)
        }
        defer { try? saida.close() }
        while true {
            if cancelado { throw Falha.cancelado }
            let fim: Bool = try autoreleasepool {
                let bloco: Data
                do { bloco = try entrada.read(upToCount: 4 << 20) ?? Data() } catch { throw Falha.leitura(nome) }
                if bloco.isEmpty { return true }
                do { try saida.write(contentsOf: bloco) } catch { throw Falha.escrita(nome) }
                progresso(Int64(bloco.count))
                return false
            }
            if fim { break }
        }
        if let data = (try? origem.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate {
            try? fm.setAttributes([.modificationDate: data], ofItemAtPath: destino.path)
        }
    }
}

/// Baixa um arquivo de cada vez, com o progresso byte a byte (para a barra andar dentro dos arquivos grandes, como
/// os vídeos) e o arquivo movido para onde pedido antes do URLSession apagar o temporário dele.
final class Baixador: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    private let trava = NSLock()
    private var continuacao: CheckedContinuation<Void, Error>?
    private var destino: URL?
    private var resultado: Error?
    private var tarefa: URLSessionDownloadTask?
    private var ultimoAviso = Date.distantPast
    /// chamado na fila do URLSession com quanto do arquivo atual já chegou (no máximo 10 vezes por segundo)
    var progresso: (Int64) -> Void {
        get { trava.lock(); defer { trava.unlock() }; return _progresso }
        set { trava.lock(); _progresso = newValue; trava.unlock() }
    }
    private var _progresso: (Int64) -> Void = { _ in }

    private lazy var sessao: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 60
        c.timeoutIntervalForResource = 3600
        c.waitsForConnectivity = false
        c.requestCachePolicy = .reloadIgnoringLocalCacheData
        c.httpMaximumConnectionsPerHost = 2
        return URLSession(configuration: c, delegate: self, delegateQueue: nil)
    }()

    func baixar(_ url: URL, para destino: URL) async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            let t = sessao.downloadTask(with: url)
            trava.lock()
            continuacao = c
            self.destino = destino
            resultado = nil
            tarefa = t
            trava.unlock()
            t.resume()
        }
    }

    func cancelarAtual() {
        trava.lock(); let t = tarefa; trava.unlock()
        t?.cancel()
    }

    func encerrar() {
        sessao.invalidateAndCancel()
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64,
                    totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        trava.lock()
        let agora = Date()
        let avisa = agora.timeIntervalSince(ultimoAviso) > 0.1
        if avisa { ultimoAviso = agora }
        let p = _progresso
        trava.unlock()
        if avisa { p(totalBytesWritten) }
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        let status = (downloadTask.response as? HTTPURLResponse)?.statusCode ?? 0
        trava.lock(); let d = destino; trava.unlock()
        var e: Error?
        if status != 200 {
            e = Importador.Falha.servidor("HTTP \(status) em \(downloadTask.originalRequest?.url?.path ?? "?")")
        } else if let d {
            do {
                try? FileManager.default.removeItem(at: d)
                try FileManager.default.moveItem(at: location, to: d)
            } catch {
                e = Importador.Falha.escrita(d.lastPathComponent)
            }
        }
        trava.lock(); resultado = e; trava.unlock()
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        trava.lock()
        let c = continuacao
        continuacao = nil
        tarefa = nil
        let r = error ?? resultado
        trava.unlock()
        if let r { c?.resume(throwing: r) } else { c?.resume() }
    }
}
