import SwiftUI

/// Cores e peças visuais do app (vidro do iOS 26 sobre um fundo escuro com brilho neon).
enum Tema {
    /// o verde-limão da página do jogo (#C8F088)
    static let acento = Color(red: 0.784, green: 0.941, blue: 0.533)
    static let fundo = Color(red: 0.051, green: 0.051, blue: 0.051)       // #0D0D0D
    static let texto2 = Color(white: 0.70)
    static let alerta = Color(red: 1.0, green: 0.82, blue: 0.40)

    /// Fundo escuro com dois brilhos (azul e roxo, as luzes de Bayview à noite), para o vidro ter o que refletir.
    static var gradiente: some View {
        ZStack {
            fundo
            RadialGradient(colors: [Color(red: 0.20, green: 0.45, blue: 1.0).opacity(0.28), .clear],
                           center: .topTrailing, startRadius: 10, endRadius: 520)
            RadialGradient(colors: [Color(red: 0.65, green: 0.25, blue: 0.95).opacity(0.20), .clear],
                           center: .bottomLeading, startRadius: 10, endRadius: 480)
        }
        .ignoresSafeArea()
    }
}

/// Cartão de vidro usado nas telas.
struct Cartao<Conteudo: View>: View {
    var titulo: String?
    var icone: String?
    @ViewBuilder var conteudo: Conteudo

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let titulo {
                Label(titulo, systemImage: icone ?? "circle")
                    .labelStyle(.titleAndIcon)
                    .font(.headline)
                    .foregroundStyle(.primary)
            }
            conteudo
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassEffect(.regular, in: .rect(cornerRadius: 24))
    }
}

/// Botão principal, verde.
struct BotaoPrincipal: View {
    var titulo: String
    var icone: String
    var desativado = false
    var acao: () -> Void

    var body: some View {
        Button(action: acao) {
            Label(titulo, systemImage: icone)
                .font(.body.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
        }
        .buttonStyle(.glassProminent)
        .tint(Tema.acento)
        .foregroundStyle(.black)
        .disabled(desativado)
    }
}

/// Uma linha de estado: ícone verde (pronto) ou cinza (falta), título e detalhe.
struct LinhaEstado: View {
    var ok: Bool
    var titulo: String
    var detalhe: String
    var atencao = false

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: atencao ? "exclamationmark.triangle.fill" : ok ? "checkmark.circle.fill" : "circle.dashed")
                .foregroundStyle(atencao ? Tema.alerta : ok ? .green : Tema.texto2)
                .font(.title3)
            VStack(alignment: .leading, spacing: 2) {
                Text(titulo).font(.subheadline.weight(.semibold))
                Text(detalhe).font(.caption).foregroundStyle(atencao ? Tema.alerta : Tema.texto2)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
    }
}

extension View {
    func telaEscura() -> some View {
        self.scrollContentBackground(.hidden)
            .background(Tema.gradiente)
    }
}

/// Pergunta antes de apagar. Uso: `confirmar = Confirmacao(titulo: "Apagar …?") { … }` e
/// `.confirmar($confirmar)` num contêiner da tela (não num botão de vidro).
struct Confirmacao: Identifiable {
    let id = UUID()
    var titulo: String
    var mensagem = "Isso não pode ser desfeito."
    var botao = "Apagar"
    var destrutivo = true
    var acao: () -> Void
}

extension View {
    func confirmar(_ c: Binding<Confirmacao?>) -> some View {
        alert(c.wrappedValue?.titulo ?? "Tem certeza?",
              isPresented: Binding(get: { c.wrappedValue != nil }, set: { if !$0 { c.wrappedValue = nil } }),
              presenting: c.wrappedValue) { x in
            if x.destrutivo { Button(x.botao, role: .destructive) { x.acao() } }
            else { Button(x.botao) { x.acao() } }
            Button("Cancelar", role: .cancel) {}
        } message: { x in
            Text(x.mensagem)
        }
    }
}

/// Tamanho em bytes para pessoas ("2,1 GB")
func tamanhoLegivel(_ bytes: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
}
