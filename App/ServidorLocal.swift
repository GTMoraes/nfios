import Foundation

/// O servidor HTTP local (Servidor/, em Rust: libnfsu2srv.a), que serve a página, o módulo e a pasta do jogo em
/// http://127.0.0.1:48021.  A porta é fixa de propósito: a origem (endereço + porta) é onde a página guarda os
/// saves do modo offline, e uma porta diferente seria outra origem, sem os saves.
enum ServidorLocal {
    static let porta: UInt16 = 48021

    static var base: URL { URL(string: "http://127.0.0.1:\(porta)/")! }
    static var diagnostico: URL { URL(string: "http://127.0.0.1:\(porta)/__diag/")! }

    /// Liga o servidor se ele não está ligado (ou se o iOS fechou o socket com o app suspenso).  Barato quando já
    /// está no ar: pode ser chamado a cada volta para a frente.
    @discardableResult
    static func garantir() -> Bool {
        aplicarMultiplayer()
        return nfsu2srv_start(Pastas.pacote.path, Pastas.jogo.path, porta) > 0
    }

    /// Passa à ponte o servidor de multiplayer dos Ajustes (vale para as próximas conexões da página).  nil quando
    /// deu certo; a mensagem de erro quando o endereço não serve (aí a ponte fica sem servidor).
    @discardableResult
    static func aplicarMultiplayer() -> String? {
        guard let url = Ajustes.enderecoMultiplayer else {
            nfsu2srv_set_remote(nil)
            return nil
        }
        if nfsu2srv_set_remote(url) == 0 { return nil }
        let e = erro
        nfsu2srv_set_remote(nil)
        return e
    }

    enum Resposta: Equatable {
        case respondeu(jogadores: Int?)
        case semServidor
        case naoRespondeu(String)
    }

    /// Pergunta /players pela ponte (o mesmo caminho que a página usa): confere o servidor escolhido de ponta a ponta.
    static func testarMultiplayer() async -> Resposta {
        if Ajustes.enderecoMultiplayer == nil { return .semServidor }
        if let e = aplicarMultiplayer() { return .naoRespondeu(e) }
        garantir()
        var pedido = URLRequest(url: base.appendingPathComponent("players"), cachePolicy: .reloadIgnoringLocalCacheData,
                                timeoutInterval: 20)
        pedido.setValue("application/json", forHTTPHeaderField: "Accept")
        do {
            let (dados, resp) = try await URLSession.shared.data(for: pedido)
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            switch status {
            case 200:
                let json = try? JSONSerialization.jsonObject(with: dados) as? [String: Any]
                return .respondeu(jogadores: Self.contar(json))
            default:
                let texto = String(data: dados.prefix(300), encoding: .utf8) ?? ""
                return .naoRespondeu(texto.isEmpty ? "HTTP \(status)" : texto)
            }
        } catch {
            return .naoRespondeu(error.localizedDescription)
        }
    }

    /// o total de jogadores, se a resposta tiver um número com cara disso
    private static func contar(_ json: [String: Any]?) -> Int? {
        guard let json else { return nil }
        for k in ["players", "count", "online", "total", "n"] {
            if let n = json[k] as? Int { return n }
        }
        return nil
    }

    /// Desliga e liga de novo (depois de importar: o manifesto e o módulo são lidos de novo).
    @discardableResult
    static func reiniciar() -> Bool {
        nfsu2srv_stop()
        return garantir()
    }

    static var noAr: Bool { nfsu2srv_alive() == 1 }

    /// a última mensagem de erro do servidor ("" se nenhuma)
    static var erro: String {
        var buf = [CChar](repeating: 0, count: 512)
        _ = nfsu2srv_last_error(&buf, Int32(buf.count))
        return buf.withUnsafeBufferPointer { p in String(cString: p.baseAddress!) }
    }
}
