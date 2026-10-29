import Foundation

/// Os ajustes do app (UserDefaults).  As telas usam as mesmas chaves com @AppStorage.
enum Ajustes {
    enum Chave {
        static let vibracao = "vibracao"
        static let som = "som"
        static let orientacao = "orientacao"
        static let modoPadrao = "modoPadrao"
        static let fps120 = "fps120"
        static let multiplayer = "multiplayer"
        static let servidorProprio = "servidorProprio"
    }

    static let nfsu2xyz = "https://nfsu2.xyz"

    /// vibração do iPhone nas batidas e nos toques dos controles (o navigator.vibrate da página)
    static var vibracao: Bool {
        UserDefaults.standard.object(forKey: Chave.vibracao) as? Bool ?? true
    }

    /// "playback": o som toca mesmo com o iPhone no silencioso (e pausa a música de outros apps);
    /// "ambient": segue a chave do silencioso e mistura com outros apps (como no Safari)
    static var som: String {
        let s = UserDefaults.standard.string(forKey: Chave.som) ?? "playback"
        return s == "ambient" ? "ambient" : "playback"
    }

    /// "livre" (gira com o iPhone; em pé a página mostra os controles embaixo) ou "paisagem"
    static var soPaisagem: Bool {
        UserDefaults.standard.string(forKey: Chave.orientacao) == "paisagem"
    }

    /// a imagem do jogo a 120 Hz nas telas ProMotion (o requestAnimationFrame do WebKit fica em 60 sem isto)
    static var fps120: Bool {
        UserDefaults.standard.object(forKey: Chave.fps120) as? Bool ?? true
    }

    /// o servidor de multiplayer do modo offline: "nfsu2xyz" (o padrão), "proprio" ou "nenhum"
    static var multiplayer: String {
        let m = UserDefaults.standard.string(forKey: Chave.multiplayer) ?? "nfsu2xyz"
        return ["nfsu2xyz", "proprio", "nenhum"].contains(m) ? m : "nfsu2xyz"
    }

    /// o endereço do servidor próprio, como digitado (ex.: http://100.91.24.1:9091)
    static var servidorProprio: String {
        UserDefaults.standard.string(forKey: Chave.servidorProprio) ?? ""
    }

    /// o endereço do servidor próprio como URL (nil se vazio ou sem http:// / https:// e host)
    static var urlServidorProprio: URL? {
        let t = servidorProprio.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let u = URL(string: t), let esquema = u.scheme?.lowercased(), esquema == "http" || esquema == "https",
              let h = u.host, !h.isEmpty else { return nil }
        return u
    }

    /// o endereço que a ponte do servidor local usa para /lan e /players (nil: sem multiplayer)
    static var enderecoMultiplayer: String? {
        switch multiplayer {
        case "proprio":
            let u = servidorProprio.trimmingCharacters(in: .whitespacesAndNewlines)
            return u.isEmpty ? nil : u
        case "nenhum": return nil
        default: return nfsu2xyz
        }
    }
}
