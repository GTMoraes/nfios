#!/bin/bash
# Monta o pacote do modo offline do app NFSU2 (página + módulo) a partir do SEU SPEED2.EXE.
#
#   bash montar-pacote.sh "<pasta do jogo>" [--saida <pasta>] [--rev <commit>] [--ptbr <pasta da tradução>]
#
# Roda no Linux ou no WSL 2 do Windows (Ubuntu), em x86_64 ou arm64.  O que ele faz, tudo dentro de
# ~/nfsu2-ios-build (nada é instalado no sistema além dos pacotes do apt da primeira vez):
#   1. baixa o nfsu2-web (github.com/th0mk/nfsu2-web, MIT) na versão fixada abaixo
#   2. Python (venv com capstone), wasi-sdk 34 e, se o do sistema for velho, um Node 22 portátil
#   3. confere o SPEED2.EXE (v1.2 americana: 4.800.512 bytes e o SHA-256 do README)
#   4. web/gen.sh: traduz o código x86 do jogo para C;  web/build.sh: compila para WebAssembly (nfsu2.wasm)
#   5. node build.mjs: a página
#   6. monta pacote-nfsu2/ = web/ + game.wasm + game.json + pacote.json  (~45 MB)
# Por padrão o pacote vai para dentro da pasta do jogo (<pasta do jogo>/pacote-nfsu2): no app, "Importar pasta…"
# com a pasta do jogo traz as duas coisas de uma vez.
#
# --ptbr <pasta>: também faz o pacote de idioma em português do Brasil (tradução TraduMix/MixMods, extraída numa
# pasta: veja o LEIA-ME), que vai para <pasta do jogo>/LANGPACKS.  No jogo: Settings › Language.
#
# O jogo e o que é feito dele (o módulo) são da EA: ficam no seu PC e no seu iPhone, nunca no GitHub.
set -euo pipefail

# a versão do nfsu2-web que o app conhece (testada com o servidor local do app); --rev main pega a mais nova
REV_PADRAO="82742d02d70985cbbfe03e56c0e46936df4cb049"
WASI_VERSAO=34
NODE_VERSAO=22.20.0
EXE_TAMANHO=4800512
EXE_SHA256="f9dd86c054878ce6276beb07c1fd61874f7a1e4bf1f241b084c65b73e24168a7"

erro() { echo; echo "ERRO: $*" >&2; exit 1; }
passo() { echo; echo "==> $*"; }

JOGO=""; SAIDA=""; REV="$REV_PADRAO"; PTBR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --saida) SAIDA="${2:-}"; shift 2 ;;
    --rev) REV="${2:-}"; shift 2 ;;
    --ptbr) PTBR="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) erro "opção desconhecida: $1 (use --help)" ;;
    *) [ -z "$JOGO" ] || erro "só uma pasta do jogo, por favor"; JOGO="$1"; shift ;;
  esac
done
[ -n "$JOGO" ] || erro "diga a pasta do jogo: bash montar-pacote.sh \"/mnt/c/Games/Need for Speed Underground 2\""
# aceita também o caminho do próprio SPEED2.EXE
if [ -f "$JOGO" ]; then JOGO="$(dirname "$JOGO")"; fi
[ -d "$JOGO" ] || erro "a pasta \"$JOGO\" não existe (no WSL o C: do Windows é /mnt/c)"
JOGO="$(cd "$JOGO" && pwd)"
EXE="$(find "$JOGO" -maxdepth 1 -iname speed2.exe -print -quit)"
[ -n "$EXE" ] || erro "não achei o SPEED2.EXE em \"$JOGO\""

passo "Conferindo o SPEED2.EXE"
TAM=$(stat -c %s "$EXE")
SHA=$(sha256sum "$EXE" | cut -d' ' -f1)
echo "  $EXE"
echo "  $TAM bytes, SHA-256 $SHA"
if [ "$TAM" != "$EXE_TAMANHO" ] || [ "$SHA" != "$EXE_SHA256" ]; then
  erro "este SPEED2.EXE não é o da v1.2 americana (o projeto só funciona com ele: $EXE_TAMANHO bytes, SHA-256 $EXE_SHA256). Aplique o patch 1.2 (North America) no jogo e rode de novo."
fi
echo "  v1.2 americana: OK"

passo "Ferramentas do sistema"
FALTA=()
for c in git python3 curl tar xz perl shasum; do command -v "$c" >/dev/null 2>&1 || FALTA+=("$c"); done
python3 -c 'import ensurepip, venv' 2>/dev/null || FALTA+=("python3-venv")
if [ ${#FALTA[@]} -gt 0 ]; then
  echo "  faltam: ${FALTA[*]}"
  erro "instale uma vez com:  sudo apt update && sudo apt install -y git python3 python3-venv curl xz-utils perl"
fi
case "$(uname -m)" in
  x86_64) ARQ_WASI=x86_64-linux; ARQ_NODE=linux-x64 ;;
  aarch64|arm64) ARQ_WASI=arm64-linux; ARQ_NODE=linux-arm64 ;;
  *) erro "arquitetura $(uname -m) não suportada (x86_64 ou arm64)" ;;
esac
OBRA="$HOME/nfsu2-ios-build"
mkdir -p "$OBRA"
case "$OBRA" in /mnt/*) erro "a pasta de trabalho precisa estar no Linux (~), não em /mnt (muito mais lento)";; esac
echo "  pasta de trabalho: $OBRA"
MEM_GB=$(awk '/MemTotal/ { printf "%d", $2 / 1048576 }' /proc/meminfo)
CPUS=$(nproc)
# cada compilação de shard pode passar de 1,5 GB: no máximo uma por 2 GB de memória
J=$(( MEM_GB / 2 )); [ "$J" -ge 1 ] || J=1; [ "$J" -le "$CPUS" ] || J=$CPUS
echo "  $CPUS núcleos, $MEM_GB GB de memória: $J compilações ao mesmo tempo"

passo "Node.js (20 ou mais novo)"
NODE_OK=0
if command -v node >/dev/null 2>&1; then
  V=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)
  [ "$V" -ge 20 ] && NODE_OK=1
fi
if [ "$NODE_OK" = 0 ]; then
  ND="$OBRA/node-v$NODE_VERSAO-$ARQ_NODE"
  if [ ! -x "$ND/bin/node" ]; then
    echo "  baixando um Node $NODE_VERSAO portátil para $OBRA (o do sistema não serve)"
    curl -fL --retry 3 -o "$OBRA/node.tar.xz" "https://nodejs.org/dist/v$NODE_VERSAO/node-v$NODE_VERSAO-$ARQ_NODE.tar.xz"
    tar -xJf "$OBRA/node.tar.xz" -C "$OBRA"
    rm -f "$OBRA/node.tar.xz"
  fi
  export PATH="$ND/bin:$PATH"
fi
echo "  node $(node --version), npm $(npm --version)"

passo "nfsu2-web ($REV)"
REPO="$OBRA/nfsu2-web"
if [ ! -d "$REPO/.git" ]; then
  git clone https://github.com/th0mk/nfsu2-web "$REPO"
fi
cd "$REPO"
git fetch --quiet origin || echo "  (sem rede para atualizar: usando o que já está aqui)"
if [ "$REV" = main ]; then REV="$(git rev-parse origin/main)"; fi
git checkout --quiet --force "$REV" || erro "não achei a versão $REV do nfsu2-web"
git clean -fdq -e .venv -e deps -e gen -e out -e web/node_modules
COMMIT="$(git rev-parse HEAD)"
CURTO="$(git rev-parse --short HEAD)"
echo "  commit $COMMIT"

passo "Python (venv com capstone e pefile)"
if [ ! -x .venv/bin/python ]; then python3 -m venv .venv; fi
.venv/bin/pip install --quiet --upgrade pip
.venv/bin/pip install --quiet -r requirements.txt
.venv/bin/python -c 'import capstone, pefile; print("  capstone", capstone.__version__)'

passo "wasi-sdk $WASI_VERSAO ($ARQ_WASI)"
WASI_DIR="$REPO/deps/wasi-sdk-$WASI_VERSAO.0-$ARQ_WASI"
if [ ! -x "$WASI_DIR/bin/clang" ]; then
  mkdir -p deps
  curl -fL --retry 3 -o deps/wasi-sdk.tar.gz \
    "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-$WASI_VERSAO/wasi-sdk-$WASI_VERSAO.0-$ARQ_WASI.tar.gz"
  tar -xzf deps/wasi-sdk.tar.gz -C deps
  rm -f deps/wasi-sdk.tar.gz
fi
[ -x "$WASI_DIR/bin/clang" ] || erro "o wasi-sdk não ficou em $WASI_DIR"
export WASI="$WASI_DIR"
"$WASI/bin/clang" --version | head -1 | sed 's/^/  /'

passo "Traduzindo o jogo para C (web/gen.sh; leva vários minutos)"
# o gen.sh lê o server.dll ao lado do SPEED2.EXE (o servidor de LAN do jogo); sem ele, só o LAN fica de fora
T0=$(date +%s)
web/gen.sh "$EXE"
echo "  em $(( ($(date +%s) - T0) / 60 )) min"

passo "Compilando para WebAssembly (web/build.sh; leva vários minutos)"
T0=$(date +%s)
J="$J" web/build.sh
[ -f out/web/nfsu2.wasm ] && [ -f out/web/nfsu2.json ] || erro "o build não gerou out/web/nfsu2.wasm"
echo "  em $(( ($(date +%s) - T0) / 60 )) min: $(du -h out/web/nfsu2.wasm | cut -f1)"

passo "A página (node build.mjs)"
cd web
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --no-audit --no-fund --loglevel=error
node build.mjs > /dev/null
cd ..
[ -f web/dist/index.html ] && [ -f web/dist/main.js ] || erro "a página não foi gerada (web/dist)"

if [ -n "$PTBR" ]; then
  passo "Pacote de idioma: português do Brasil (LANGPACKS na pasta do jogo)"
  [ -d "$PTBR" ] || erro "a pasta da tradução \"$PTBR\" não existe"
  .venv/bin/python tools/langpacks.py "$JOGO" --ptbr "$PTBR"
fi

passo "Montando o pacote"
[ -n "$SAIDA" ] || SAIDA="$JOGO"
PAC="$SAIDA/pacote-nfsu2"
if ! mkdir -p "$SAIDA" 2>/dev/null || ! touch "$SAIDA/.nfsu2-teste" 2>/dev/null; then
  echo "  sem permissão para gravar em \"$SAIDA\": o pacote vai para $OBRA"
  PAC="$OBRA/pacote-nfsu2"
fi
rm -f "$SAIDA/.nfsu2-teste" 2>/dev/null || true
TMP="$PAC.tmp"
rm -rf "$TMP"
mkdir -p "$TMP/web"
# a página inteira, menos as páginas de teste e os mapas de código (que o build já deixa em web/dist-dev)
(cd web/dist && find . -type f ! -path './test/*' ! -name '*.map' -print0) | while IFS= read -r -d '' f; do
  mkdir -p "$TMP/web/$(dirname "$f")"
  cp "web/dist/$f" "$TMP/web/$f"
done
cp out/web/nfsu2.wasm "$TMP/game.wasm"
cp out/web/nfsu2.json "$TMP/game.json"
GJREV=$(sed -n 's/.*"rev":"\([^"]*\)".*/\1/p' out/web/nfsu2.json)
cat > "$TMP/pacote.json" <<EOF
{"rev":"${GJREV:-$CURTO}","commit":"$COMMIT","exe_sha256":"$SHA","wasi_sdk":"$WASI_VERSAO","feito":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
EOF
rm -rf "$PAC"
mv "$TMP" "$PAC"
echo "  $PAC ($(du -sh "$PAC" | cut -f1))"
ls -la "$PAC"

echo
echo "PRONTO."
echo "Leve para o iPhone a pasta do jogo (com o pacote-nfsu2 dentro) ou só o pacote-nfsu2, e no app use"
echo "\"Importar pasta…\".  O caminho no Windows:"
command -v wslpath >/dev/null 2>&1 && wslpath -w "$PAC" 2>/dev/null | sed 's/^/  /' || echo "  $PAC"
