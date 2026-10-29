#!/usr/bin/env bash
# Monta a pasta do servidor no WSL do GT-SRV (padrão ~/nfsu2) a partir da pasta do jogo copiada para a passagem:
#
#   bash preparar.sh <pasta do jogo> [--destino <pasta>] [--arquivos <pasta>]
#
#   <pasta do jogo>  a pasta do NFSU2 com o SPEED2.EXE v1.2 e o pacote-nfsu2 dentro (como o montar-pacote.sh deixa)
#   --destino        a pasta do servidor (compose.yml, .env, module/, jogo/); padrão ~/nfsu2
#   --arquivos       onde montar os arquivos que o app baixa (Jogo/, Pacote/, manifesto.json); padrão o
#                    NFSU2_ARQUIVOS do .env, ou <destino>/arquivos
#
# O multiplayer usa só o módulo e o SPEED2.EXE (~40 MB, em <destino>).  Os arquivos para o app são uma cópia só do que
# o jogo lê (as mesmas entradas que o servidor oficial serve), mais o pacote, com o tamanho e o SHA-256 de cada um no
# manifesto.json: o app confere cada arquivo que baixa.  Não apaga nada fora do que ele mesmo monta; um .env que já
# existe fica como está.
set -euo pipefail

EXE_SHA=f9dd86c054878ce6276beb07c1fd61874f7a1e4bf1f241b084c65b73e24168a7
REV_IMAGEM=82742d0

erro() { echo "ERRO: $*" >&2; exit 1; }
AQUI="$(cd "$(dirname "$0")" && pwd)"
JOGO=""
DEST="$HOME/nfsu2"
ARQ=""
while [ $# -gt 0 ]; do
  case "$1" in
    --destino) DEST="${2:?--destino precisa de uma pasta}"; shift 2 ;;
    --arquivos) ARQ="${2:?--arquivos precisa de uma pasta}"; shift 2 ;;
    -*) erro "opção desconhecida: $1" ;;
    *) [ -z "$JOGO" ] || erro "uma pasta do jogo só"; JOGO="$1"; shift ;;
  esac
done
[ -n "$JOGO" ] || erro "uso: bash preparar.sh <pasta do jogo, com SPEED2.EXE e pacote-nfsu2> [--destino ~/nfsu2] [--arquivos /mnt/d/nfsu2/arquivos]"
[ -d "$JOGO" ] || erro "não achei a pasta $JOGO"
command -v python3 >/dev/null || erro "falta o python3 (sudo apt install -y python3)"

EXE="$(find "$JOGO" -maxdepth 1 -iname speed2.exe | head -1)"
[ -n "$EXE" ] || erro "$JOGO não tem SPEED2.EXE"
[ "$(sha256sum "$EXE" | cut -d' ' -f1)" = "$EXE_SHA" ] || erro "o SPEED2.EXE de $JOGO não é o v1.2 que o nfsu2-web usa"
PACOTE="$JOGO/pacote-nfsu2"
[ -f "$PACOTE/game.wasm" ] && [ -f "$PACOTE/game.json" ] && [ -d "$PACOTE/web" ] \
  || erro "$JOGO não tem o pacote-nfsu2 (rode o montar-pacote.sh antes)"

commit="$(sed -n 's/.*"commit":"\([0-9a-f]*\)".*/\1/p' "$PACOTE/pacote.json" 2>/dev/null || true)"
if [ -n "$commit" ] && [ "${commit:0:7}" != "$REV_IMAGEM" ]; then
  echo "AVISO: o pacote é do commit ${commit:0:7} e a imagem padrão é sha-$REV_IMAGEM."
  echo "       Ponha NFSU2_IMAGEM=ghcr.io/th0mk/nfsu2-web-rs:sha-${commit:0:7} no $DEST/.env."
fi

# ---- a pasta do servidor
mkdir -p "$DEST"
cp "$AQUI/compose.yml" "$DEST/compose.yml"
cp "$AQUI/env.exemplo" "$DEST/env.exemplo"
[ -f "$DEST/.env" ] || cp "$AQUI/env.exemplo" "$DEST/.env"
rm -rf "$DEST/module.novo" "$DEST/jogo.novo"
mkdir -p "$DEST/module.novo" "$DEST/jogo.novo"
cp "$PACOTE/game.wasm" "$DEST/module.novo/nfsu2.wasm"
cp "$PACOTE/game.json" "$DEST/module.novo/nfsu2.json"
cp "$EXE" "$DEST/jogo.novo/SPEED2.EXE"
rm -rf "$DEST/module" "$DEST/jogo"
mv "$DEST/module.novo" "$DEST/module"
mv "$DEST/jogo.novo" "$DEST/jogo"
chmod -R a+rX "$DEST/module" "$DEST/jogo"

# ---- os arquivos para o app (o caminho: --arquivos, o NFSU2_ARQUIVOS do .env, ou <destino>/arquivos)
if [ -z "$ARQ" ]; then
  ARQ="$(sed -n 's/^NFSU2_ARQUIVOS=//p' "$DEST/.env" | tail -1)"
  ARQ="${ARQ:-./arquivos}"
fi
case "$ARQ" in /*) ;; *) ARQ="$DEST/${ARQ#./}" ;; esac
# o .env aponta para onde os arquivos foram montados
if grep -q '^NFSU2_ARQUIVOS=' "$DEST/.env"; then
  sed -i "s#^NFSU2_ARQUIVOS=.*#NFSU2_ARQUIVOS=$ARQ#" "$DEST/.env"
else
  printf '\nNFSU2_ARQUIVOS=%s\n' "$ARQ" >> "$DEST/.env"
fi
echo "Montando os arquivos para o app em $ARQ (cópia de ~1,7 GB)…"
NOVO="$ARQ.novo"
rm -rf "$NOVO"
mkdir -p "$NOVO/Jogo" "$NOVO/Pacote"
python3 - "$JOGO" "$PACOTE" "$NOVO" <<'PY'
import hashlib, json, os, shutil, sys, time
jogo, pacote, novo = sys.argv[1:4]
# as entradas do jogo que o servidor oficial serve (web/server-rs, GAME_ENTRIES) e as do pacote
ENTRADAS_JOGO = {"speed2.exe", "server.dll", "server.cfg", "cars", "credits", "frontend", "global", "langpacks",
                 "languages", "memcard", "movies", "nis", "pfdata", "sound", "speech", "subtitles", "texpacks", "tracks"}
ENTRADAS_PACOTE = {"web", "game.wasm", "game.json", "pacote.json"}
itens = []
def copiar(origem, raiz_dest, prefixo, permitidas):
    for nome in sorted(os.listdir(origem)):
        if nome.startswith(".") or nome.lower() not in permitidas:
            continue
        o = os.path.join(origem, nome)
        if os.path.islink(o):
            continue
        if os.path.isfile(o):
            caminhos = [(o, nome)]
        else:
            caminhos = []
            for d, pastas, arqs in os.walk(o):
                pastas[:] = sorted(p for p in pastas if not p.startswith(".") and not os.path.islink(os.path.join(d, p)))
                for a in sorted(arqs):
                    f = os.path.join(d, a)
                    if a.startswith(".") or os.path.islink(f) or not os.path.isfile(f):
                        continue
                    caminhos.append((f, os.path.relpath(f, origem).replace(os.sep, "/")))
        for f, rel in caminhos:
            dest = os.path.join(raiz_dest, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            h = hashlib.sha256()
            with open(f, "rb") as a, open(dest, "wb") as b:
                while True:
                    bloco = a.read(4 << 20)
                    if not bloco:
                        break
                    h.update(bloco)
                    b.write(bloco)
            st = os.stat(f)
            os.utime(dest, (st.st_atime, st.st_mtime))
            os.chmod(dest, 0o644)
            itens.append({"p": f"{prefixo}/{rel}", "s": os.path.getsize(dest), "h": h.hexdigest()})
copiar(jogo, os.path.join(novo, "Jogo"), "Jogo", ENTRADAS_JOGO)
copiar(pacote, os.path.join(novo, "Pacote"), "Pacote", ENTRADAS_PACOTE)
rev = ""
try:
    rev = json.load(open(os.path.join(pacote, "pacote.json"))).get("rev", "")
except Exception:
    pass
m = {"v": 1, "rev": rev, "feito": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
     "bytes": sum(i["s"] for i in itens), "arquivos": itens}
with open(os.path.join(novo, "manifesto.json"), "w") as f:
    json.dump(m, f, separators=(",", ":"))
jogo_n = sum(1 for i in itens if i["p"].startswith("Jogo/"))
print(f"  {len(itens)} arquivos ({jogo_n} do jogo, {len(itens) - jogo_n} do pacote), {m['bytes'] / 1e9:.2f} GB")
PY
chmod -R a+rX "$NOVO"
rm -rf "$ARQ.velho"
[ -d "$ARQ" ] && mv "$ARQ" "$ARQ.velho"
mv "$NOVO" "$ARQ"
rm -rf "$ARQ.velho"

# ---- conferência: o que está no destino é o que veio
for par in "$PACOTE/game.wasm:$DEST/module/nfsu2.wasm" "$PACOTE/game.json:$DEST/module/nfsu2.json" "$EXE:$DEST/jogo/SPEED2.EXE" \
           "$AQUI/compose.yml:$DEST/compose.yml" "$EXE:$ARQ/Jogo/$(basename "$EXE")" "$PACOTE/game.wasm:$ARQ/Pacote/game.wasm"; do
  a="$(sha256sum "${par%%:*}" | cut -d' ' -f1)"; b="$(sha256sum "${par##*:}" | cut -d' ' -f1)"
  [ "$a" = "$b" ] || erro "a cópia de ${par%%:*} saiu diferente"
done
echo
echo "Pronto em $DEST (arquivos para o app em $ARQ):"
( cd "$DEST" && sha256sum compose.yml module/nfsu2.wasm jogo/SPEED2.EXE )
sha256sum "$ARQ/manifesto.json"
echo
echo "Próximo: cd $DEST && docker compose up -d && docker compose ps"
