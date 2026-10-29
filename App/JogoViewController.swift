import UIKit
import WebKit

/// De onde vem a página.
enum ModoJogo: Equatable {
    /// http://127.0.0.1:48021: o servidor local do app (sua cópia do jogo, sem internet)
    case local
    /// https://nfsu2.xyz: o site do projeto (outros jogadores; offline depois do "Preload the game data")
    case online
    /// a página de diagnóstico do servidor local
    case diagnostico

    var endereco: URL {
        switch self {
        case .local: return ServidorLocal.base
        case .online: return URL(string: "https://nfsu2.xyz/")!
        case .diagnostico: return ServidorLocal.diagnostico
        }
    }

    var usaServidorLocal: Bool { self != .online }
}

/// Repassa as mensagens da página ("nfsu2") sem segurar o controlador (o WKUserContentController retém o handler).
@MainActor
final class PonteMensagens: NSObject, WKScriptMessageHandler {
    weak var alvo: JogoViewController?

    init(_ alvo: JogoViewController) {
        self.alvo = alvo
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        alvo?.receber(message.body)
    }
}

/// A página do jogo em tela cheia: sem barras, sem zoom, gestos das bordas adiados (o primeiro deslize só mostra o
/// indicador, o Início não abre sem querer), tela sempre ligada, vibração nas batidas, downloads para Arquivos.
final class JogoViewController: UIViewController, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    private let modo: ModoJogo
    private var webView: WKWebView!
    private var botaoMenu: UIButton!
    private var aviso: UIButton!
    private var falha: UIView?
    private var vibrador: Vibrador?
    /// a fase da página (body[data-phase]): "playing" durante o jogo
    private var fase = ""
    private var destinos: [ObjectIdentifier: URL] = [:]
    private var ultimoDownload: URL?
    private var avisoTimer: Timer?
    /// confere o servidor local a cada 10 s (modo offline): se o iOS fechou o socket, religa na mesma porta
    private var vigia: Timer?

    private static let corFundo = UIColor(red: 0.051, green: 0.051, blue: 0.051, alpha: 1)

    init(modo: ModoJogo) {
        self.modo = modo
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .fullScreen
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) não é usado")
    }

    // MARK: - tela cheia de verdade

    override var prefersStatusBarHidden: Bool { true }
    override var prefersHomeIndicatorAutoHidden: Bool { true }
    override var preferredScreenEdgesDeferringSystemGestures: UIRectEdge { .all }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        Ajustes.soPaisagem ? .landscape : .allButUpsideDown
    }

    override func loadView() {
        let v = UIView()
        v.backgroundColor = Self.corFundo
        view = v
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        if Ajustes.vibracao { vibrador = Vibrador() }
        montarWebView()
        montarBotaoMenu()
        montarAviso()
        NotificationCenter.default.addObserver(self, selector: #selector(voltouParaFrente),
                                               name: UIApplication.didBecomeActiveNotification, object: nil)
        if modo.usaServidorLocal {
            vigia = Timer.scheduledTimer(timeInterval: 10, target: self, selector: #selector(conferirServidor),
                                         userInfo: nil, repeats: true)
        }
        carregar()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        UIApplication.shared.isIdleTimerDisabled = true
        setNeedsStatusBarAppearanceUpdate()
        setNeedsUpdateOfHomeIndicatorAutoHidden()
        setNeedsUpdateOfScreenEdgesDeferringSystemGestures()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        UIApplication.shared.isIdleTimerDisabled = false
    }

    // MARK: - montagem

    /// O WebKit segura o requestAnimationFrame perto de 60 por segundo mesmo numa tela de 120 Hz (a preferência
    /// PreferPageRenderingUpdatesNear60FPSEnabled, ligada por padrão; é o "Prefer Page Rendering Updates near 60fps"
    /// do Safari › Avançado › Recursos do WebKit).  Desligada aqui pela interface interna do WebKit (_features e
    /// _setEnabled:forFeature:, o que o Safari usa para essa tela): se um iOS futuro não tiver uma das duas, nada
    /// acontece e o jogo fica em 60.  Junto com CADisableMinimumFrameDurationOnPhone no Info.plist, sem o qual o
    /// iPhone não passa de 60 Hz para app nenhum.  No Modo Pouca Energia o iOS volta a 60 de qualquer jeito.
    private static func liberar120Hz(_ prefs: WKPreferences) {
        let classe: AnyObject = WKPreferences.self
        let listar = NSSelectorFromString("_features")
        let ligar = NSSelectorFromString("_setEnabled:forFeature:")
        guard classe.responds(to: listar), prefs.responds(to: ligar),
              let lista = classe.perform(listar)?.takeUnretainedValue() as? [NSObject],
              let metodo = class_getInstanceMethod(WKPreferences.self, ligar) else { return }
        typealias Ligar = @convention(c) (AnyObject, Selector, Bool, AnyObject) -> Void
        let f = unsafeBitCast(method_getImplementation(metodo), to: Ligar.self)
        for recurso in lista {
            guard let chave = recurso.value(forKey: "key") as? String, chave.contains("Near60FPS") else { continue }
            f(prefs, ligar, false, recurso)
        }
    }

    private func montarWebView() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []
        // nfsu2.xyz está em WKAppBoundDomains (Info.plist) e 127.0.0.1 conta como do app por ser local: com isto o
        // service worker do site funciona (o modo offline dele) e os scripts do app entram nas duas páginas
        config.limitsNavigationsToAppBoundDomains = true
        // o mesmo user agent do Safari do iPhone: a página reconhece o navegador como reconhece no Safari
        config.applicationNameForUserAgent = Self.nomeDoSafari()
        if Ajustes.fps120 { Self.liberar120Hz(config.preferences) }
        let ucc = config.userContentController
        ucc.add(PonteMensagens(self), name: "nfsu2")
        ucc.addUserScript(WKUserScript(source: Self.scriptDaPonte(vibrar: Ajustes.vibracao, som: Ajustes.som),
                                       injectionTime: .atDocumentStart, forMainFrameOnly: true))

        let wv = WKWebView(frame: view.bounds, configuration: config)
        wv.translatesAutoresizingMaskIntoConstraints = false
        wv.navigationDelegate = self
        wv.uiDelegate = self
        wv.allowsLinkPreview = false
        wv.allowsBackForwardNavigationGestures = false
        wv.isOpaque = false
        wv.backgroundColor = Self.corFundo
        wv.underPageBackgroundColor = Self.corFundo
        wv.scrollView.backgroundColor = Self.corFundo
        // a página cuida das áreas seguras (viewport-fit=cover); o WebKit não empurra o conteúdo
        wv.scrollView.contentInsetAdjustmentBehavior = .never
        wv.scrollView.bounces = false
        wv.scrollView.alwaysBounceVertical = false
        wv.scrollView.alwaysBounceHorizontal = false
        // depurável pelo Safari de um Mac (Desenvolvedor › iPhone), se um dia houver um Mac por perto
        wv.isInspectable = true
        view.addSubview(wv)
        NSLayoutConstraint.activate([
            wv.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            wv.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            wv.topAnchor.constraint(equalTo: view.topAnchor),
            wv.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        webView = wv
    }

    /// "Version/26.0 Mobile/15E148 Safari/604.1", com a versão deste iOS
    private static func nomeDoSafari() -> String {
        let v = UIDevice.current.systemVersion.split(separator: ".")
        let versao = v.prefix(2).joined(separator: ".")
        return "Version/\(versao.isEmpty ? "26.0" : versao) Mobile/15E148 Safari/604.1"
    }

    /// O script que o app põe na página antes de tudo (só no quadro principal):
    /// - navigator.standalone = true: a página se trata como app da Tela de Início (sem o aviso de "Adicionar")
    /// - navigator.vibrate → vibração do iPhone (o iOS não tem navigator.vibrate)
    /// - navigator.audioSession.type: tocar mesmo no silencioso, ou seguir a chave
    /// - sem seleção de texto, lupa e menu de "Copiar" ao segurar o dedo (menos nos campos de texto)
    /// - avisa o app quando a página entra e sai do jogo (body[data-phase] = "playing"), para o botão do menu
    static func scriptDaPonte(vibrar: Bool, som: String) -> String {
        let vibracao = vibrar
            ? "def('vibrate', { value: function (p) { post({ type: 'vibrate', pattern: Array.isArray(p) ? p.map(Number) : [Number(p) || 0] }); return true; }, writable: true });"
            : ""
        return """
        (function () {
          var h = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.nfsu2;
          var post = function (m) { try { if (h) h.postMessage(m); } catch (e) {} };
          var def = function (k, d) { try { d.configurable = true; Object.defineProperty(navigator, k, d); } catch (e) {} };
          def('standalone', { get: function () { return true; } });
          \(vibracao)
          try { if (navigator.audioSession) navigator.audioSession.type = '\(som)'; } catch (e) {}
          var fase = function () { post({ type: 'phase', phase: (document.body && document.body.dataset.phase) || '' }); };
          var css = 'html,body{-webkit-user-select:none;user-select:none;-webkit-touch-callout:none;-webkit-tap-highlight-color:transparent}' +
                    'input,textarea,select,[contenteditable]{-webkit-user-select:text;user-select:text}';
          var pronto = function () {
            var s = document.createElement('style');
            s.textContent = css;
            (document.head || document.documentElement).appendChild(s);
            if (document.body) {
              fase();
              new MutationObserver(fase).observe(document.body, { attributes: true, attributeFilter: ['data-phase'] });
            }
          };
          if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', pronto); else pronto();
        })();
        """
    }

    private func montarBotaoMenu() {
        var cfg = UIButton.Configuration.filled()
        cfg.image = UIImage(systemName: "square.grid.2x2")
        cfg.preferredSymbolConfigurationForImage = UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold)
        cfg.baseBackgroundColor = UIColor.black.withAlphaComponent(0.55)
        cfg.baseForegroundColor = .white
        cfg.cornerStyle = .capsule
        cfg.contentInsets = NSDirectionalEdgeInsets(top: 9, leading: 9, bottom: 9, trailing: 9)
        let b = UIButton(configuration: cfg)
        b.translatesAutoresizingMaskIntoConstraints = false
        b.accessibilityLabel = "Menu do app"
        b.showsMenuAsPrimaryAction = true
        b.menu = UIMenu(title: "", children: [
            UIAction(title: "Voltar ao início do app", image: UIImage(systemName: "chevron.backward")) { [weak self] _ in
                self?.fechar()
            },
            UIAction(title: "Recarregar a página", image: UIImage(systemName: "arrow.clockwise")) { [weak self] _ in
                self?.recarregar()
            },
        ])
        view.addSubview(b)
        NSLayoutConstraint.activate([
            b.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -10),
            b.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -10),
        ])
        botaoMenu = b
    }

    private func montarAviso() {
        var cfg = UIButton.Configuration.filled()
        cfg.baseBackgroundColor = UIColor(white: 0.12, alpha: 0.92)
        cfg.baseForegroundColor = .white
        cfg.cornerStyle = .capsule
        cfg.image = UIImage(systemName: "square.and.arrow.down")
        cfg.imagePadding = 8
        cfg.contentInsets = NSDirectionalEdgeInsets(top: 10, leading: 14, bottom: 10, trailing: 14)
        let b = UIButton(configuration: cfg, primaryAction: UIAction { [weak self] _ in
            guard let self, let u = self.ultimoDownload else { return }
            self.esconderAviso()
            self.compartilhar([u])
        })
        b.translatesAutoresizingMaskIntoConstraints = false
        b.alpha = 0
        b.isHidden = true
        view.addSubview(b)
        NSLayoutConstraint.activate([
            b.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            b.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            b.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -32),
        ])
        aviso = b
    }

    // MARK: - carregar, recarregar, sair

    private func carregar() {
        esconderFalha()
        if modo.usaServidorLocal && !ServidorLocal.garantir() {
            mostrarFalha("O servidor local não abriu.\n\(ServidorLocal.erro)")
            return
        }
        webView.load(URLRequest(url: modo.endereco))
    }

    private func recarregar() {
        esconderFalha()
        if modo.usaServidorLocal { ServidorLocal.garantir() }
        if webView.url == nil { carregar() } else { webView.reload() }
    }

    private func fechar() {
        avisoTimer?.invalidate()
        avisoTimer = nil
        vigia?.invalidate()
        vigia = nil
        vibrador?.parar()
        webView.configuration.userContentController.removeScriptMessageHandler(forName: "nfsu2")
        dismiss(animated: true)
    }

    /// de volta do segundo plano: o iOS pode ter fechado o socket do servidor local com o app suspenso
    @objc private func voltouParaFrente() {
        conferirServidor()
    }

    /// fora da thread principal: religar pode levar um instante se a porta ainda estiver presa
    @objc private func conferirServidor() {
        guard modo.usaServidorLocal else { return }
        DispatchQueue.global(qos: .utility).async { ServidorLocal.garantir() }
    }

    // MARK: - mensagens da página

    func receber(_ corpo: Any) {
        guard let d = corpo as? [String: Any], let tipo = d["type"] as? String else { return }
        switch tipo {
        case "vibrate":
            let padrao = (d["pattern"] as? [Any] ?? []).compactMap { ($0 as? NSNumber)?.doubleValue }
            vibrador?.vibrar(padrao)
        case "phase":
            fase = d["phase"] as? String ?? ""
            atualizarBotaoMenu()
        case "share":
            if let t = d["text"] as? String { compartilhar([t]) }
        default:
            break
        }
    }

    private func atualizarBotaoMenu() {
        // durante a corrida o botão some (os controles de toque são da página); no menu da página ele volta
        let mostrar = fase != "playing"
        UIView.animate(withDuration: 0.2) {
            self.botaoMenu.alpha = mostrar ? 1 : 0
        }
        botaoMenu.isUserInteractionEnabled = mostrar
    }

    // MARK: - navegação

    private func permitido(_ url: URL) -> Bool {
        let esquema = url.scheme?.lowercased() ?? ""
        if esquema == "about" || esquema == "blob" || esquema == "data" { return true }
        guard esquema == "http" || esquema == "https", let host = url.host()?.lowercased() else { return false }
        switch modo {
        case .online:
            return host == "nfsu2.xyz" || host.hasSuffix(".nfsu2.xyz")
        case .local, .diagnostico:
            return host == "127.0.0.1" && url.port == Int(ServidorLocal.porta)
        }
    }

    private func abrirFora(_ url: URL) {
        let esquema = url.scheme?.lowercased() ?? ""
        guard ["http", "https", "mailto", "tel"].contains(esquema) else { return }
        UIApplication.shared.open(url)
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, preferences: WKWebpagePreferences,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy, WKWebpagePreferences) -> Void) {
        if navigationAction.shouldPerformDownload {
            decisionHandler(.download, preferences)
            return
        }
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel, preferences)
            return
        }
        guard let quadro = navigationAction.targetFrame else {
            // nova janela (target=_blank): links para fora (Discord, Ko-fi, GitHub) abrem no Safari
            if !permitido(url) { abrirFora(url) }
            decisionHandler(.cancel, preferences)
            return
        }
        if quadro.isMainFrame && !permitido(url) {
            abrirFora(url)
            decisionHandler(.cancel, preferences)
            return
        }
        decisionHandler(.allow, preferences)
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationResponsePolicy) -> Void) {
        if !navigationResponse.canShowMIMEType {
            decisionHandler(.download)
            return
        }
        if let r = navigationResponse.response as? HTTPURLResponse,
           let cd = r.value(forHTTPHeaderField: "Content-Disposition"), cd.lowercased().hasPrefix("attachment") {
            decisionHandler(.download)
            return
        }
        decisionHandler(.allow)
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        esconderFalha()
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
        tratarFalha(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) {
        tratarFalha(error)
    }

    private func tratarFalha(_ error: Error) {
        let e = error as NSError
        // cancelada (outra navegação, ou virou download): não é falha
        if e.domain == NSURLErrorDomain && e.code == NSURLErrorCancelled { return }
        if e.domain == "WebKitErrorDomain" && e.code == 102 { return }
        let texto: String
        switch modo {
        case .online:
            texto = "Não consegui abrir o nfsu2.xyz.\n\(e.localizedDescription)\n\nSem internet, o site só abre depois de ter carregado uma vez aqui (e o \"Preload the game data\" deixa os dados no iPhone). Para jogar sem depender do site, use o modo offline com a sua cópia."
        case .local, .diagnostico:
            texto = "O servidor local não respondeu.\n\(e.localizedDescription)"
        }
        mostrarFalha(texto)
    }

    /// O iOS fechou o processo da página (falta de memória, geralmente).  A página recarregada mostra o que houve e
    /// oferece o log desse jogo.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        if modo.usaServidorLocal { ServidorLocal.garantir() }
        webView.reload()
    }

    // MARK: - janelas, diálogos, permissões

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction,
                 windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let u = navigationAction.request.url, !permitido(u) { abrirFora(u) }
        return nil
    }

    private func podeMostrarDialogo() -> Bool {
        view.window != nil && presentedViewController == nil
    }

    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo,
                 completionHandler: @escaping @MainActor @Sendable () -> Void) {
        guard podeMostrarDialogo() else { completionHandler(); return }
        let a = UIAlertController(title: nil, message: message, preferredStyle: .alert)
        a.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler() })
        present(a, animated: true)
    }

    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo,
                 completionHandler: @escaping @MainActor @Sendable (Bool) -> Void) {
        guard podeMostrarDialogo() else { completionHandler(false); return }
        let a = UIAlertController(title: nil, message: message, preferredStyle: .alert)
        a.addAction(UIAlertAction(title: "Cancelar", style: .cancel) { _ in completionHandler(false) })
        a.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler(true) })
        present(a, animated: true)
    }

    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping @MainActor @Sendable (String?) -> Void) {
        guard podeMostrarDialogo() else { completionHandler(nil); return }
        let a = UIAlertController(title: nil, message: prompt, preferredStyle: .alert)
        a.addTextField { campo in campo.text = defaultText }
        a.addAction(UIAlertAction(title: "Cancelar", style: .cancel) { _ in completionHandler(nil) })
        a.addAction(UIAlertAction(title: "OK", style: .default) { [weak a] _ in completionHandler(a?.textFields?.first?.text ?? "") })
        present(a, animated: true)
    }

    /// direção por inclinação (o "tilt" dos controles de toque): sem perguntar, a página é do próprio app
    func webView(_ webView: WKWebView, requestDeviceOrientationAndMotionPermissionFor origin: WKSecurityOrigin,
                 initiatedByFrame frame: WKFrameInfo, decisionHandler: @escaping @MainActor @Sendable (WKPermissionDecision) -> Void) {
        decisionHandler(.grant)
    }

    /// câmera e microfone: o jogo não usa
    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin, initiatedByFrame frame: WKFrameInfo,
                 type: WKMediaCaptureType, decisionHandler: @escaping @MainActor @Sendable (WKPermissionDecision) -> Void) {
        decisionHandler(.deny)
    }

    // MARK: - downloads (log, capturas, backups de saves) → Documentos/Downloads

    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String,
                  completionHandler: @escaping @MainActor @Sendable (URL?) -> Void) {
        let fm = FileManager.default
        try? fm.createDirectory(at: Pastas.downloads, withIntermediateDirectories: true)
        let limpo = suggestedFilename.replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: ":", with: "_")
        let nome = limpo.isEmpty ? "download" : limpo
        var destino = Pastas.downloads.appendingPathComponent(nome)
        let base = (nome as NSString).deletingPathExtension
        let ext = (nome as NSString).pathExtension
        var n = 2
        while fm.fileExists(atPath: destino.path) {
            destino = Pastas.downloads.appendingPathComponent(ext.isEmpty ? "\(base) (\(n))" : "\(base) (\(n)).\(ext)")
            n += 1
        }
        destinos[ObjectIdentifier(download)] = destino
        completionHandler(destino)
    }

    func downloadDidFinish(_ download: WKDownload) {
        guard let u = destinos.removeValue(forKey: ObjectIdentifier(download)) else { return }
        ultimoDownload = u
        mostrarAviso("Salvo em Arquivos › NFSU2 › Downloads: \(u.lastPathComponent). Toque para compartilhar.")
    }

    func download(_ download: WKDownload, didFailWithError error: any Error, resumeData: Data?) {
        destinos.removeValue(forKey: ObjectIdentifier(download))
        ultimoDownload = nil
        mostrarAviso("O download falhou: \(error.localizedDescription)")
    }

    // MARK: - avisos, falhas, compartilhar

    private func mostrarAviso(_ texto: String) {
        avisoTimer?.invalidate()
        var cfg = aviso.configuration
        cfg?.title = texto
        cfg?.titleLineBreakMode = .byTruncatingMiddle
        aviso.configuration = cfg
        aviso.isHidden = false
        view.bringSubviewToFront(aviso)
        UIView.animate(withDuration: 0.25) { self.aviso.alpha = 1 }
        avisoTimer = Timer.scheduledTimer(timeInterval: 5, target: self, selector: #selector(avisoExpirou), userInfo: nil, repeats: false)
    }

    @objc private func avisoExpirou() {
        esconderAviso()
    }

    private func esconderAviso() {
        avisoTimer?.invalidate()
        avisoTimer = nil
        UIView.animate(withDuration: 0.25, animations: { self.aviso.alpha = 0 }, completion: { _ in self.aviso.isHidden = true })
    }

    private func compartilhar(_ itens: [Any]) {
        guard view.window != nil, presentedViewController == nil else { return }
        let vc = UIActivityViewController(activityItems: itens, applicationActivities: nil)
        vc.popoverPresentationController?.sourceView = botaoMenu
        present(vc, animated: true)
    }

    private func mostrarFalha(_ texto: String) {
        esconderFalha()
        let caixa = UIView()
        caixa.translatesAutoresizingMaskIntoConstraints = false
        caixa.backgroundColor = Self.corFundo

        let rotulo = UILabel()
        rotulo.text = texto
        rotulo.textColor = UIColor(white: 0.85, alpha: 1)
        rotulo.font = .preferredFont(forTextStyle: .body)
        rotulo.numberOfLines = 0
        rotulo.textAlignment = .center

        var c1 = UIButton.Configuration.filled()
        c1.title = "Tentar de novo"
        c1.baseBackgroundColor = UIColor(red: 0.784, green: 0.941, blue: 0.533, alpha: 1)
        c1.baseForegroundColor = .black
        c1.cornerStyle = .capsule
        let tentar = UIButton(configuration: c1, primaryAction: UIAction { [weak self] _ in self?.carregar() })

        var c2 = UIButton.Configuration.gray()
        c2.title = "Voltar ao início do app"
        c2.cornerStyle = .capsule
        let voltar = UIButton(configuration: c2, primaryAction: UIAction { [weak self] _ in self?.fechar() })

        let pilha = UIStackView(arrangedSubviews: [rotulo, tentar, voltar])
        pilha.axis = .vertical
        pilha.spacing = 16
        pilha.alignment = .center
        pilha.translatesAutoresizingMaskIntoConstraints = false
        caixa.addSubview(pilha)
        view.addSubview(caixa)
        NSLayoutConstraint.activate([
            caixa.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            caixa.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            caixa.topAnchor.constraint(equalTo: view.topAnchor),
            caixa.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            pilha.centerYAnchor.constraint(equalTo: caixa.centerYAnchor),
            pilha.leadingAnchor.constraint(equalTo: caixa.safeAreaLayoutGuide.leadingAnchor, constant: 28),
            pilha.trailingAnchor.constraint(equalTo: caixa.safeAreaLayoutGuide.trailingAnchor, constant: -28),
        ])
        falha = caixa
        view.bringSubviewToFront(botaoMenu)
    }

    private func esconderFalha() {
        falha?.removeFromSuperview()
        falha = nil
    }
}
