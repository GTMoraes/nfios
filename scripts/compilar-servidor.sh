#!/bin/bash
# Compila o servidor local (Servidor/, Rust) como biblioteca estática para iPhone (arm64):
#   Servidor/target/aarch64-apple-ios/release/libnfsu2srv.a
# Roda no GitHub Actions (macOS), antes do xcodebuild.  Só usa a biblioteca padrão do Rust e a crate libc.
set -euo pipefail
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RAIZ/Servidor"

# o rustup do runner (vem instalado nas imagens macOS do GitHub); se não vier, instala
if ! command -v rustup >/dev/null 2>&1; then
  curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
rustup target add aarch64-apple-ios

# o mesmo iOS mínimo do app (sem isto o linker avisa que os objetos são de outra versão)
export IPHONEOS_DEPLOYMENT_TARGET=26.0
cargo build --release --lib --target aarch64-apple-ios
# as bibliotecas do sistema que a .a precisa (vão no log, para conferir com o OTHER_LDFLAGS do project.yml)
cargo rustc --release --lib --crate-type staticlib --target aarch64-apple-ios -- --print native-static-libs 2>&1 \
  | grep -i "native-static-libs" || true

A=target/aarch64-apple-ios/release/libnfsu2srv.a
[ -f "$A" ] || { echo "::error::faltou $A"; exit 1; }
for s in _nfsu2srv_start _nfsu2srv_stop _nfsu2srv_alive _nfsu2srv_port _nfsu2srv_last_error _nfsu2srv_set_remote; do
  nm -gU "$A" 2>/dev/null | grep -q " T $s\$" || { echo "::error::a biblioteca não exporta $s"; exit 1; }
done
lipo -info "$A"
echo "servidor OK: $A ($(du -h "$A" | cut -f1))"
