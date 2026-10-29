// Diagnóstico: um arquivo de 1 MB escrito e lido com FileSystemSyncAccessHandle (o que a página usa para os saves e
// para o cache dos dados), numa pasta "__diag" da OPFS que é apagada no fim.
self.onmessage = async () => {
  const t0 = performance.now();
  let root = null;
  try {
    root = await navigator.storage.getDirectory();
    const dir = await root.getDirectoryHandle('__diag', { create: true });
    const fh = await dir.getFileHandle('teste.bin', { create: true });
    let h;
    try { h = await fh.createSyncAccessHandle({ mode: 'readwrite-unsafe' }); } catch { h = await fh.createSyncAccessHandle(); }
    const b = new Uint8Array(1 << 20);
    for (let i = 0; i < b.length; i += 4096) b[i] = (i >> 12) & 255;
    const w = h.write(b, { at: 0 });
    h.flush();
    const r = new Uint8Array(1 << 20);
    const n = h.read(r, { at: 0 });
    h.close();
    const same = w === b.length && n === b.length && r[4096 * 7] === b[4096 * 7] && r[4096 * 200] === b[4096 * 200];
    postMessage({ ok: same, detail: same ? `1 MB escrito e lido em ${(performance.now() - t0).toFixed(1)} ms` : `escreveu ${w}, leu ${n} bytes, conteúdo ${same ? 'igual' : 'diferente'}` });
  } catch (e) {
    postMessage({ ok: false, detail: String(e) });
  } finally {
    try { await root?.removeEntry('__diag', { recursive: true }); } catch { /* já não existe */ }
  }
};
