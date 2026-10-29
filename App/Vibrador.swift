import UIKit
import CoreHaptics

/// navigator.vibrate da página → vibração do iPhone.
///
/// A página vibra de dois jeitos (web/src/input/gamepad.ts e app/touch.ts do nfsu2-web): um toque de 8 ms quando um
/// botão de toque é apertado, e pulsos de 30 a 120 ms enquanto o motor de vibração do jogo está forte (batidas,
/// zebras), no máximo a cada 150 ms.  Toque curto vira um "tap" leve; pulso vira vibração contínua com a força
/// proporcional à duração (o jogo manda mais tempo quando o motor está mais forte).
@MainActor
final class Vibrador {
    private var motor: CHHapticEngine?
    private let leve = UIImpactFeedbackGenerator(style: .light)
    private let forte = UIImpactFeedbackGenerator(style: .heavy)

    init() {
        guard CHHapticEngine.capabilitiesForHardware().supportsHaptics else { return }
        do {
            let m = try CHHapticEngine()
            m.isAutoShutdownEnabled = true
            m.playsHapticsOnly = true
            m.resetHandler = { [weak m] in try? m?.start() }
            try m.start()
            motor = m
        } catch {
            motor = nil
        }
        leve.prepare()
    }

    /// padrão do navigator.vibrate: [vibra, pausa, vibra, ...] em milissegundos
    func vibrar(_ padrao: [Double]) {
        let p = padrao.map { max(0, min($0, 2000)) }
        guard let primeiro = p.first, primeiro > 0 else { return }
        if p.count == 1 && primeiro <= 15 {
            leve.impactOccurred(intensity: 0.7)
            return
        }
        guard let motor else {
            forte.impactOccurred(intensity: CGFloat(intensidade(primeiro)))
            return
        }
        var eventos: [CHHapticEvent] = []
        var t: TimeInterval = 0
        for (i, ms) in p.enumerated() {
            let d = ms / 1000
            if i % 2 == 0 && d > 0 {
                let forca = CHHapticEventParameter(parameterID: .hapticIntensity, value: intensidade(ms))
                let nitidez = CHHapticEventParameter(parameterID: .hapticSharpness, value: 0.3)
                eventos.append(CHHapticEvent(eventType: .hapticContinuous, parameters: [forca, nitidez], relativeTime: t, duration: d))
            }
            t += d
        }
        guard !eventos.isEmpty else { return }
        do {
            let desenho = try CHHapticPattern(events: eventos, parameters: [])
            let player = try motor.makePlayer(with: desenho)
            try motor.start()
            try player.start(atTime: CHHapticTimeImmediate)
        } catch {
            forte.impactOccurred(intensity: CGFloat(intensidade(primeiro)))
        }
    }

    /// 30 ms → 0,45; 120 ms → 1 (a página manda 30 + 90 × nível)
    private func intensidade(_ ms: Double) -> Float {
        Float(0.45 + 0.55 * min(1, max(0, (ms - 30) / 90)))
    }

    func parar() {
        motor?.stop(completionHandler: nil)
    }
}
