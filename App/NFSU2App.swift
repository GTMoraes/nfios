import SwiftUI
import UIKit

@main
struct NFSU2App: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup {
            InicioView()
                .preferredColorScheme(.dark)
                .tint(Tema.acento)
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // escrever num socket que o WebKit já fechou dá erro, nunca SIGPIPE (que fecharia o app); o servidor local
        // também liga SO_NOSIGPIPE em cada conexão
        signal(SIGPIPE, SIG_IGN)
        Pastas.preparar()
        ServidorLocal.garantir()
        return true
    }
}

/// Abre o jogo (ou o diagnóstico) por cima da tela inicial, em tela cheia (UIKit: é assim que a tela controla a
/// barra de status, o indicador de Início e os gestos das bordas).
@MainActor
enum Abrir {
    static func jogo(_ modo: ModoJogo) {
        guard let topo = controladorDoTopo() else { return }
        topo.present(JogoViewController(modo: modo), animated: true)
    }

    private static func controladorDoTopo() -> UIViewController? {
        let cenas = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let janelas = cenas.flatMap { $0.windows }
        var vc = (janelas.first { $0.isKeyWindow } ?? janelas.first)?.rootViewController
        while let p = vc?.presentedViewController { vc = p }
        return vc
    }
}
