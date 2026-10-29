// Diagnóstico: o menor processador de AudioWorklet (o som do jogo passa por um)
class DiagNoop extends AudioWorkletProcessor {
  process() { return true; }
}
registerProcessor('diag-noop', DiagNoop);
