// Diagnóstico do app: o que o WebKit deste iPhone oferece ao jogo, na mesma origem do modo local
// (http://127.0.0.1:<porta>), com os mesmos cabeçalhos de isolamento.  Cada verificação é isolada: uma que falha
// ou trava (tempo esgotado) não impede as outras.  "Essencial" = sem isso o jogo não roda.

const out = document.getElementById('out');
const verdict = document.getElementById('verdict');
const results = [];

function row(name, ok, detail = '', essential = false) {
  results.push({ name, ok, detail: String(detail ?? ''), essential });
  const tr = document.createElement('tr');
  const s = document.createElement('td'); s.className = 's'; s.textContent = ok === true ? '✅' : ok === false ? (essential ? '❌' : '⚠️') : 'ℹ️';
  const n = document.createElement('td'); n.className = 'n'; n.textContent = name;
  const d = document.createElement('td'); d.className = 'd'; d.textContent = String(detail ?? '');
  tr.append(s, n, d);
  out.append(tr);
}

const withTimeout = (p, ms, what) => Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error(`${what}: sem resposta em ${ms / 1000} s`)), ms))]);
const mb = n => n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : `${Math.round(n / (1 << 20))} MB`;

// módulos mínimos de cada recurso do WebAssembly que o build do jogo usa (gerados com o binaryen, validados no V8)
const FEATS = {
  'WebAssembly SIMD': [0,97,115,109,1,0,0,0,1,5,1,96,0,1,123,3,2,1,0,10,22,1,20,0,253,12,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,11],
  'WebAssembly tail calls': [0,97,115,109,1,0,0,0,1,4,1,96,0,0,3,2,1,0,10,6,1,4,0,18,0,11],
  'WebAssembly bulk memory': [0,97,115,109,1,0,0,0,1,4,1,96,0,0,3,2,1,0,5,3,1,0,1,10,14,1,12,0,65,0,65,0,65,0,252,10,0,0,11],
  'WebAssembly conversões saturadas': [0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,10,11,1,9,0,67,0,0,0,0,252,0,11],
  'WebAssembly threads (atômicos)': [0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,5,4,1,3,1,1,10,10,1,8,0,65,0,254,16,2,0,11],
  'WebAssembly exceções (exnref)': [0,97,115,109,1,0,0,0,1,4,1,96,0,0,3,2,1,0,13,3,1,0,0,10,17,1,15,0,2,105,31,64,1,3,0,8,0,11,0,11,26,11],
};

async function checkGpu() {
  const gpu = navigator.gpu;
  if (!gpu) { row('WebGPU', false, 'navigator.gpu não existe neste WebKit (precisa do iOS 26)', true); return; }
  let ad = null;
  try { ad = await withTimeout(gpu.requestAdapter({ powerPreference: 'high-performance' }), 10000, 'requestAdapter'); }
  catch (e) { row('WebGPU', false, e, true); return; }
  if (!ad) { row('WebGPU', false, 'sem adaptador (requestAdapter devolveu null)', true); return; }
  const info = ad.info ?? {};
  row('WebGPU', true, [info.vendor, info.architecture, info.description].filter(Boolean).join(' · ') || 'adaptador presente', true);
  const f = new Set(ad.features);
  row('Texturas ASTC', f.has('texture-compression-astc'), f.has('texture-compression-astc') ? 'DXT1 vira ASTC (menos memória)' : 'texturas descompactadas (mais memória)');
  row('Texturas BC (DXT)', f.has('texture-compression-bc'), f.has('texture-compression-bc') ? 'como no PC' : 'normal no iPhone: a página converte');
  try {
    const dev = await withTimeout(ad.requestDevice({ requiredFeatures: [...f].filter(x => x === 'texture-compression-astc' || x === 'texture-compression-bc') }), 10000, 'requestDevice');
    row('Dispositivo WebGPU', true, `maxBufferSize ${mb(dev.limits.maxBufferSize)}, storage binding ${mb(dev.limits.maxStorageBufferBindingSize)}, textura ${dev.limits.maxTextureDimension2D}px`, true);
    dev.destroy();
  } catch (e) { row('Dispositivo WebGPU', false, e, true); }
}

async function checkOpfs() {
  if (!navigator.storage?.getDirectory) { row('Arquivos da origem (OPFS)', false, 'navigator.storage.getDirectory não existe', true); return; }
  try {
    const w = new Worker('/__diag/opfs-worker.js', { type: 'module' });
    const r = await withTimeout(new Promise((res, rej) => { w.onmessage = e => res(e.data); w.onerror = e => rej(new Error(e.message || 'erro no worker')); w.postMessage('go'); }), 15000, 'worker OPFS');
    w.terminate();
    row('Workers de módulo', true, 'new Worker(…, {type: "module"})', true);
    row('OPFS com acesso síncrono (saves e cache)', r.ok, r.detail, true);
  } catch (e) { row('OPFS com acesso síncrono (saves e cache)', false, e, true); }
}

async function checkAudio() {
  try {
    const AC = window.AudioContext || window.webkitAudioContext;
    const ac = new AC();
    await withTimeout(ac.audioWorklet.addModule('/__diag/worklet.js'), 10000, 'addModule');
    new AudioWorkletNode(ac, 'diag-noop');
    row('Som (AudioWorklet)', true, `${ac.sampleRate} Hz`, true);
    await ac.close();
  } catch (e) { row('Som (AudioWorklet)', false, e, true); }
}

async function checkAv1() {
  if (!('VideoDecoder' in self)) { row('Vídeos AV1 (WebCodecs)', false, 'sem VideoDecoder: os vídeos tocam em VP6'); return; }
  try {
    const s = await withTimeout(VideoDecoder.isConfigSupported({ codec: 'av01.0.04M.08', codedWidth: 768, codedHeight: 512 }), 5000, 'isConfigSupported');
    row('Vídeos AV1 (WebCodecs)', !!s.supported, s.supported ? 'decodificador do iPhone' : 'não: os vídeos tocam em VP6');
  } catch (e) { row('Vídeos AV1 (WebCodecs)', false, e); }
}

function refreshRate() {
  return new Promise(resolve => {
    const t = [];
    const step = now => { t.push(now); if (t.length <= 64) requestAnimationFrame(step); else {
      const d = t.slice(4).map((x, i) => (x - t[i]) / 4).sort((a, b) => a - b);
      resolve(Math.round(1000 / d[d.length >> 1]));
    } };
    requestAnimationFrame(step);
  });
}

async function checkServer() {
  try {
    const r = await withTimeout(fetch('/__status', { cache: 'no-store' }), 5000, '/__status');
    const s = await r.json();
    row('Pacote (página + módulo)', s.pack.page && s.pack.module && s.pack.info, `página ${s.pack.page ? 'sim' : 'não'}, módulo ${s.pack.module ? 'sim' : 'não'}, game.json ${s.pack.info ? 'sim' : 'não'}`);
    row('Pasta do jogo', s.game.exe, s.game.files ? `${s.game.files} arquivos, ${mb(s.game.bytes)}${s.game.exe ? '' : ', sem SPEED2.EXE'}` : 'não importada');
    document.getElementById('module').disabled = !(s.pack.module && s.pack.info);
  } catch (e) { row('Servidor local', false, e); }
}

async function run() {
  const t0 = performance.now();
  row('Contexto seguro', isSecureContext, location.origin, true);
  row('Isolamento entre origens', self.crossOriginIsolated === true, self.crossOriginIsolated ? 'COOP + COEP ativos' : 'sem isolamento: sem SharedArrayBuffer', true);
  row('SharedArrayBuffer', typeof SharedArrayBuffer === 'function', '', true);
  row('Atomics.waitAsync', typeof Atomics.waitAsync === 'function', '', true);
  for (const [name, bytes] of Object.entries(FEATS)) {
    let ok = false, why = '';
    try { ok = WebAssembly.validate(new Uint8Array(bytes)); } catch (e) { why = String(e); }
    row(name, ok, why, true);
  }
  try {
    const m = new WebAssembly.Memory({ initial: 16384, maximum: 65536, shared: true });
    row('Memória do jogo (1 GB, até 4 GB)', m.buffer.byteLength === 1 << 30, `${mb(m.buffer.byteLength)} compartilhada`, true);
  } catch (e) { row('Memória do jogo (1 GB, até 4 GB)', false, e, true); }
  await checkGpu();
  await checkOpfs();
  await checkAudio();
  await checkAv1();
  row('Controles (Gamepad API)', typeof navigator.getGamepads === 'function', '');
  row('Sensor de inclinação', typeof DeviceOrientationEvent !== 'undefined', typeof DeviceOrientationEvent?.requestPermission === 'function' ? 'pede permissão (o app concede)' : '');
  row('Vibração (ponte do app)', typeof navigator.vibrate === 'function', typeof navigator.vibrate === 'function' ? 'navigator.vibrate → vibração do iPhone' : 'sem ponte: só no app');
  row('Tela sempre ligada', null, 'o app desliga o bloqueio automático durante o jogo');
  try {
    const est = await navigator.storage.estimate();
    row('Espaço desta origem', null, `em uso ${mb(est.usage ?? 0)} de ${mb(est.quota ?? 0)}`);
  } catch (e) { row('Espaço desta origem', null, e); }
  try { row('Armazenamento persistente', await navigator.storage.persisted(), ''); } catch { /* sem a API */ }
  row('Service worker', null, 'serviceWorker' in navigator ? 'disponível (no modo local não é usado)' : 'indisponível nesta origem');
  row('Tela', null, `${screen.width}×${screen.height} pt, ${devicePixelRatio}×, ${await refreshRate()} Hz, ${navigator.hardwareConcurrency} núcleos`);
  row('Navegador', null, navigator.userAgent);
  await checkServer();
  const bad = results.filter(r => r.essential && r.ok !== true);
  verdict.className = bad.length ? 'bad' : 'ok';
  verdict.textContent = bad.length
    ? `Faltam ${bad.length} recurso(s) essencial(is): ${bad.map(r => r.name).join(', ')}.`
    : `Tudo o que o jogo precisa está disponível (${Math.round(performance.now() - t0)} ms).`;
}

function report() {
  return ['Diagnóstico NFSU2 — ' + new Date().toISOString(), verdict.textContent, '',
    ...results.map(r => `${r.ok === true ? 'OK ' : r.ok === false ? (r.essential ? 'ERRO' : 'aviso') : 'info'}  ${r.name}${r.detail ? ': ' + r.detail : ''}`)].join('\n');
}

document.getElementById('share').addEventListener('click', async () => {
  const text = report();
  const h = window.webkit?.messageHandlers?.nfsu2;
  if (h) { h.postMessage({ type: 'share', text }); return; }
  try { await navigator.clipboard.writeText(text); verdict.textContent += ' (relatório copiado)'; } catch { alert(text); }
});

document.getElementById('module').addEventListener('click', async e => {
  const b = e.currentTarget; b.disabled = true; b.textContent = 'Compilando…';
  const t0 = performance.now();
  try {
    const info = await (await fetch('/game.json', { cache: 'no-store' })).json();
    const mod = await WebAssembly.compileStreaming(fetch('/game.wasm', { cache: 'no-store' }));
    const imports = WebAssembly.Module.imports(mod).length, exports = WebAssembly.Module.exports(mod).length;
    row('Módulo do jogo compila', true, `rev ${info.rev}, ${mb(info.size)}, ${imports} importações, ${exports} exportações, ${((performance.now() - t0) / 1000).toFixed(1)} s`, true);
  } catch (err) { row('Módulo do jogo compila', false, err, true); }
  b.textContent = 'Compilar o módulo do jogo';
});

run().catch(e => row('Diagnóstico', false, e, true));
