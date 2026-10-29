import SwiftUI
import UniformTypeIdentifiers

/// A tela inicial: os dois modos (online pelo nfsu2.xyz e offline com a sua cópia), a importação e o diagnóstico.
struct InicioView: View {
    @Environment(\.scenePhase) private var fase
    @State private var estado = EstadoOffline()
    @State private var lendo = true
    @State private var importador = Importador()
    @State private var escolhendoPasta = false
    @State private var mostrandoAjustes = false
    @State private var mostrandoAjuda = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                cabecalho
                cartaoOnline
                cartaoOffline
                cartaoFerramentas
                rodape
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 20)
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        .telaEscura()
        .fileImporter(isPresented: $escolhendoPasta, allowedContentTypes: [.folder]) { resultado in
            switch resultado {
            case .success(let pasta):
                importador.importar(pasta)
            case .failure(let e):
                importador.erro = e.localizedDescription
            }
        }
        .sheet(isPresented: $mostrandoAjustes, onDismiss: { atualizar() }) { AjustesView() }
        .sheet(isPresented: $mostrandoAjuda) { AjudaView() }
        .task { atualizar() }
        .onChange(of: fase) {
            if fase == .active { atualizar() }
        }
        // a importação terminou (bem ou mal): o estado das pastas mudou
        .onChange(of: importador.ativo) {
            if !importador.ativo { atualizar() }
        }
    }

    /// lê o estado das pastas fora da thread principal (medir 2,5 GB de arquivos leva alguns milissegundos)
    private func atualizar() {
        lendo = true
        Task {
            let e = await Task.detached(priority: .userInitiated) { EstadoOffline.ler() }.value
            estado = e
            lendo = false
        }
    }

    // MARK: - partes

    private var cabecalho: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("NFSU2")
                .font(.system(size: 40, weight: .black, design: .rounded))
                .foregroundStyle(LinearGradient(colors: [Tema.acento, Color(red: 0.35, green: 0.75, blue: 1.0)],
                                                startPoint: .leading, endPoint: .trailing))
            Text("Underground 2 no iPhone, em tela cheia")
                .font(.subheadline)
                .foregroundStyle(Tema.texto2)
        }
        .padding(.top, 8)
    }

    private var cartaoOnline: some View {
        Cartao(titulo: "Online · nfsu2.xyz", icone: "globe") {
            Text("O site do projeto, dentro do app: os carros de outros jogadores no free roam, corridas Online e saves na nuvem por frase-senha. Depois de \"Preload the game data\" (no fim da página), abre também sem internet.")
                .font(.footnote)
                .foregroundStyle(Tema.texto2)
            BotaoPrincipal(titulo: "Jogar online", icone: "play.fill") {
                Abrir.jogo(.online)
            }
        }
    }

    private var cartaoOffline: some View {
        Cartao(titulo: "Offline · sua cópia", icone: "internaldrive") {
            Text("Tudo do iPhone, sem depender de site: a página e o módulo feitos do seu SPEED2.EXE (o pacote) e a pasta do seu jogo.")
                .font(.footnote)
                .foregroundStyle(Tema.texto2)
            if lendo && !estado.podeJogar && !importador.ativo {
                ProgressView().frame(maxWidth: .infinity)
            } else {
                linhasOffline
            }
            if importador.ativo {
                progressoImportacao
            } else {
                BotaoPrincipal(titulo: "Jogar offline", icone: "play.fill", desativado: !estado.podeJogar) {
                    Abrir.jogo(.local)
                }
                HStack(spacing: 10) {
                    Button {
                        importador.erro = nil
                        importador.resultado = nil
                        escolhendoPasta = true
                    } label: {
                        Label("Importar pasta…", systemImage: "folder.badge.plus")
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 4)
                    }
                    .buttonStyle(.glass)
                    Button {
                        mostrandoAjuda = true
                    } label: {
                        Label("Como fazer", systemImage: "questionmark.circle")
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 4)
                    }
                    .buttonStyle(.glass)
                }
            }
            if let erro = importador.erro {
                Label(erro, systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(Tema.alerta)
            } else if let ok = importador.resultado {
                Label(ok, systemImage: "checkmark.circle.fill")
                    .font(.footnote)
                    .foregroundStyle(.green)
            }
        }
    }

    @ViewBuilder private var linhasOffline: some View {
        if estado.pacotePronto {
            LinhaEstado(ok: true, titulo: "Pacote", detalhe: "rev \(estado.pacoteRev ?? "?") · \(tamanhoLegivel(estado.pacoteBytes))")
        } else {
            LinhaEstado(ok: false, titulo: "Pacote", detalhe: "Não importado (página + módulo, feitos no PC pelo montar-pacote.sh)")
        }
        if estado.jogoPronto {
            if !estado.exeOficial {
                LinhaEstado(ok: false, titulo: "Pasta do jogo", detalhe: "O SPEED2.EXE não é o da v1.2 americana: o jogo não roda com outra versão.", atencao: true)
            } else if estado.exeDiferenteDoPacote {
                LinhaEstado(ok: false, titulo: "Pasta do jogo", detalhe: "O pacote foi feito de outro SPEED2.EXE.", atencao: true)
            } else {
                LinhaEstado(ok: true, titulo: "Pasta do jogo", detalhe: "\(estado.jogoArquivos) arquivos · \(tamanhoLegivel(estado.jogoBytes)) · SPEED2.EXE v1.2")
            }
        } else {
            LinhaEstado(ok: false, titulo: "Pasta do jogo", detalhe: "Não importada (a pasta do NFSU2 de PC, v1.2, com SPEED2.EXE)")
        }
    }

    private var progressoImportacao: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(importador.titulo).font(.subheadline.weight(.semibold))
            if importador.total > 0 {
                ProgressView(value: Double(importador.feitos), total: Double(max(importador.total, 1)))
                Text("\(tamanhoLegivel(importador.feitos)) de \(tamanhoLegivel(importador.total))")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(Tema.texto2)
            } else {
                ProgressView()
            }
            if !importador.arquivoAtual.isEmpty {
                Text(importador.arquivoAtual)
                    .font(.caption2)
                    .foregroundStyle(Tema.texto2)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Text("Mantenha o app aberto até terminar.")
                .font(.caption)
                .foregroundStyle(Tema.alerta)
            Button("Cancelar", role: .destructive) { importador.cancelar() }
                .buttonStyle(.glass)
        }
    }

    private var cartaoFerramentas: some View {
        Cartao(titulo: "Ferramentas", icone: "wrench.and.screwdriver") {
            Text("O diagnóstico confere se este iPhone tem tudo o que o jogo usa (WebGPU, threads, memória, som) — rode antes de importar os 2,5 GB.")
                .font(.footnote)
                .foregroundStyle(Tema.texto2)
            HStack(spacing: 10) {
                Button {
                    Abrir.jogo(.diagnostico)
                } label: {
                    Label("Diagnóstico", systemImage: "stethoscope")
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 4)
                }
                .buttonStyle(.glass)
                Button {
                    mostrandoAjustes = true
                } label: {
                    Label("Ajustes", systemImage: "gearshape")
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 4)
                }
                .buttonStyle(.glass)
            }
        }
    }

    private var rodape: some View {
        Text("Projeto de fã, sem relação com a Electronic Arts. O código que roda o jogo é o nfsu2-web (licença MIT); o jogo e os dados são da EA: no modo offline, use só a sua cópia.")
            .font(.caption2)
            .foregroundStyle(Tema.texto2)
            .padding(.horizontal, 4)
    }
}

/// O passo a passo do modo offline, em tela.
struct AjudaView: View {
    @Environment(\.dismiss) private var fechar

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Cartao(titulo: "1. O pacote, no PC", icone: "desktopcomputer") {
                        Text("No WSL do PC, rode o montar-pacote.sh com o caminho do seu SPEED2.EXE (v1.2 americana). Ele traduz o jogo para WebAssembly e monta a pasta pacote-nfsu2 (página + módulo, ~45 MB).")
                            .font(.footnote)
                    }
                    Cartao(titulo: "2. Passar para o iPhone", icone: "arrow.left.arrow.right") {
                        Text("Qualquer um destes: iCloud Drive; um pendrive USB-C; o app Arquivos › ••• › Conectar ao Servidor (smb://IP-do-PC) com a pasta compartilhada no Windows; ou o app Dispositivos Apple do Windows › Arquivos › NFSU2.")
                            .font(.footnote)
                    }
                    Cartao(titulo: "3. Importar", icone: "folder.badge.plus") {
                        Text("Em \"Importar pasta…\", escolha a pasta pacote-nfsu2 e depois a pasta do jogo (a que tem SPEED2.EXE), ou uma pasta que tenha as duas dentro. Do jogo só vem o que ele lê (~2,5 GB); saves e mods do PC ficam de fora.")
                            .font(.footnote)
                    }
                    Cartao(titulo: "4. Jogar", icone: "flag.checkered") {
                        Text("Os saves do modo offline ficam no iPhone, separados dos do modo online. Para levar de um para o outro, use os saves na nuvem por frase-senha no modo online, ou o backup de saves da página (vai para Arquivos › NFSU2 › Downloads).")
                            .font(.footnote)
                    }
                }
                .padding(16)
            }
            .telaEscura()
            .navigationTitle("Modo offline")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("OK") { fechar() }
                }
            }
        }
        .preferredColorScheme(.dark)
    }
}
