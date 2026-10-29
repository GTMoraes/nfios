import SwiftUI

struct AjustesView: View {
    @Environment(\.dismiss) private var fechar
    @AppStorage(Ajustes.Chave.vibracao) private var vibracao = true
    @AppStorage(Ajustes.Chave.som) private var som = "playback"
    @AppStorage(Ajustes.Chave.orientacao) private var orientacao = "livre"
    @AppStorage(Ajustes.Chave.fps120) private var fps120 = true
    @AppStorage(Ajustes.Chave.multiplayer) private var multiplayer = "nfsu2xyz"
    @AppStorage(Ajustes.Chave.servidorProprio) private var servidorProprio = ""
    @State private var erroEndereco: String?
    @State private var teste: String?
    @State private var testando = false
    @State private var estado = EstadoOffline()
    @State private var downloads: Int64 = 0
    @State private var confirmar: Confirmacao?

    private var versao: String {
        let v = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
        let b = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        return "\(v) (\(b))"
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle("Vibração nas batidas", isOn: $vibracao)
                    Picker("Som", selection: $som) {
                        Text("Sempre tocar").tag("playback")
                        Text("Seguir o silencioso").tag("ambient")
                    }
                    Picker("Orientação", selection: $orientacao) {
                        Text("Gira com o iPhone").tag("livre")
                        Text("Só paisagem").tag("paisagem")
                    }
                    Toggle("120 Hz", isOn: $fps120)
                } header: {
                    Text("Jogo")
                } footer: {
                    Text("\"Sempre tocar\" toca mesmo com o iPhone no silencioso e pausa a música de outros apps; \"Seguir o silencioso\" é como no Safari. Em pé, a página mostra a imagem em cima e os controles embaixo. 120 Hz: a imagem acompanha a tela ProMotion em vez de ficar em 60 (gasta mais bateria e esquenta mais; no Modo Pouca Energia volta a 60; o Diagnóstico mostra a taxa medida). Vale na próxima vez que abrir o jogo.")
                }

                Section {
                    Picker("Servidor", selection: $multiplayer) {
                        Text("nfsu2.xyz").tag("nfsu2xyz")
                        Text("Meu servidor").tag("proprio")
                        Text("Nenhum").tag("nenhum")
                    }
                    if multiplayer != "nenhum" {
                        Button {
                            testar()
                        } label: {
                            HStack {
                                Text("Testar a conexão")
                                Spacer()
                                if testando { ProgressView() }
                            }
                        }
                        .disabled(testando)
                        if let teste {
                            Text(teste).font(.footnote).foregroundStyle(.secondary)
                        }
                    }
                } header: {
                    Text("Multiplayer do modo offline")
                } footer: {
                    Text("O jogo continua vindo do iPhone (com 120 Hz e tudo); só o multiplayer (free roam, salas, contagem de jogadores) passa pelo servidor escolhido. Use um servidor da mesma versão do pacote: o nfsu2.xyz atualiza sozinho, e um pacote antigo pode deixar de conversar com ele. Vale na próxima conexão da página.")
                }

                Section {
                    TextField("http://100.91.24.1:9091", text: $servidorProprio)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.done)
                    if let erroEndereco {
                        Text(erroEndereco).font(.footnote).foregroundStyle(.red)
                    }
                } header: {
                    Text("Meu servidor")
                } footer: {
                    Text("O seu servidor no GT-SRV (servidor-gt-srv/ no projeto): o endereço com http://, o IP do Tailscale e a porta (não o nome *.ts.net). Serve para o multiplayer (\"Meu servidor\" acima) e para o \"Baixar do meu servidor\" da tela inicial.")
                }

                Section {
                    LabeledContent("Pacote", value: estado.pacotePronto ? tamanhoLegivel(estado.pacoteBytes) : "—")
                    LabeledContent("Pasta do jogo", value: estado.jogoPronto ? tamanhoLegivel(estado.jogoBytes) : "—")
                    LabeledContent("Downloads", value: tamanhoLegivel(downloads))
                    if estado.pacotePronto {
                        Button("Apagar o pacote", role: .destructive) {
                            confirmar = Confirmacao(titulo: "Apagar o pacote?", mensagem: "Página e módulo do modo offline. Os saves ficam.") {
                                apagar(Pastas.pacote)
                            }
                        }
                    }
                    if estado.jogoPronto {
                        Button("Apagar a pasta do jogo", role: .destructive) {
                            confirmar = Confirmacao(titulo: "Apagar a pasta do jogo?", mensagem: "Libera \(tamanhoLegivel(estado.jogoBytes)). Os saves ficam; para jogar offline de novo, importe a pasta outra vez.") {
                                apagar(Pastas.jogo)
                            }
                        }
                    }
                } header: {
                    Text("Modo offline")
                } footer: {
                    Text("Ficam em Arquivos › No Meu iPhone › NFSU2. Os saves e o cache de cada modo ficam nos dados do próprio app: apagar o app apaga os saves que não estiverem na nuvem.")
                }

                Section {
                    LabeledContent("Versão do app", value: versao)
                    LabeledContent("Servidor local", value: ServidorLocal.noAr ? "127.0.0.1:\(ServidorLocal.porta)" : "desligado")
                    Link("nfsu2-web no GitHub (MIT)", destination: URL(string: "https://github.com/th0mk/nfsu2-web")!)
                    Link("nfsu2.xyz", destination: URL(string: "https://nfsu2.xyz/")!)
                } header: {
                    Text("Sobre")
                } footer: {
                    Text("Projeto de fã, sem relação com a Electronic Arts. Need for Speed e Underground 2 são marcas da EA.")
                }
            }
            .telaEscura()
            .navigationTitle("Ajustes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("OK") { fechar() }
                }
            }
            .confirmar($confirmar)
            .task { ler() }
            .onChange(of: multiplayer) { aplicar() }
            .onChange(of: servidorProprio) { aplicar() }
        }
        .preferredColorScheme(.dark)
    }

    private func ler() {
        Task {
            let (e, d) = await Task.detached(priority: .userInitiated) {
                (EstadoOffline.ler(), Pastas.medir(Pastas.downloads).bytes)
            }.value
            estado = e
            downloads = d
        }
    }

    /// passa a escolha para a ponte do servidor local na hora (a página pega na próxima conexão)
    private func aplicar() {
        teste = nil
        let e = ServidorLocal.aplicarMultiplayer()
        if multiplayer == "proprio" {
            erroEndereco = servidorProprio.isEmpty ? nil : e
        } else {
            let u = servidorProprio.trimmingCharacters(in: .whitespacesAndNewlines)
            erroEndereco = u.isEmpty || Ajustes.urlServidorProprio != nil ? nil : "Endereço inválido: use http://IP:porta"
        }
    }

    private func testar() {
        testando = true
        teste = nil
        Task {
            let r = await ServidorLocal.testarMultiplayer()
            testando = false
            switch r {
            case .respondeu(let n):
                teste = n.map { "Respondeu: \($0) jogando agora." } ?? "Respondeu."
            case .semServidor:
                teste = "Nenhum servidor escolhido."
            case .naoRespondeu(let m):
                teste = "Não respondeu: \(m)"
            }
        }
    }

    private func apagar(_ pasta: URL) {
        try? FileManager.default.removeItem(at: pasta)
        ServidorLocal.reiniciar()
        ler()
    }
}
