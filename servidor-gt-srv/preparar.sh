#!/usr/bin/env bash
# Monta a pasta do servidor de multiplayer no WSL do GT-SRV (padrão ~/nfsu2) a partir da pasta de passagem:
#
#   bash preparar.sh <pasta pacote-nfsu2> <SPEED2.EXE ou a pasta do jogo> [pasta destino]
#
#   <pacote-nfsu2>  o que o montar-pacote.sh gerou (game.wasm, game.json, pacote.json): o módulo do servidor vem dele,
#                   para página, módulo e servidor serem o mesmo build
#   <SPEED2.EXE>    o executável v1.2 (ou a pasta do jogo que o tem): o servidor só sobe com ele.  Só ele é copiado.
#
# Não apaga nada: um destino que já tem .env fica com o .env dele; module/ e jogo/ são substituídos.
set -euo pipefail

EXE_SHA=f9dd86c054878ce6276beb07c1fd61874f7a1e4bf1f241b084c65b73e24168a7
REV_IMAGEM=82742d0

erro() { echo "ERRO: $*" >&2; exit 1; }
[ $# -ge 2 ] || erro "uso: bash preparar.sh <pasta pacote-nfsu2> <SPEED2.EXE ou pasta do jogo> [destino, padrão ~/nfsu2]"
AQUI="$(cd "$(dirname "$0")" && pwd)"
PACOTE="$1"
EXE="$2"
DEST="${3:-$HOME/nfsu2}"

[ -f "$PACOTE/game.wasm" ] && [ -f "$PACOTE/game.json" ] || erro "$PACOTE não tem game.wasm e game.json (é a pasta pacote-nfsu2?)"
if [ -d "$EXE" ]; then
  achado="$(find "$EXE" -maxdepth 1 -iname speed2.exe | head -1)"
  [ -n "$achado" ] || erro "$EXE não tem SPEED2.EXE"
  EXE="$achado"
fi
[ -f "$EXE" ] || erro "não achei $EXE"
sha="$(sha256sum "$EXE" | cut -d' ' -f1)"
[ "$sha" = "$EXE_SHA" ] || erro "esse SPEED2.EXE não é o v1.2 que o nfsu2-web usa (sha256 $sha)"

# a versão do pacote e a da imagem
commit=""
[ -f "$PACOTE/pacote.json" ] && commit="$(sed -n 's/.*"commit":"\([0-9a-f]*\)".*/\1/p' "$PACOTE/pacote.json")"
if [ -n "$commit" ] && [ "${commit:0:7}" != "$REV_IMAGEM" ]; then
  echo "AVISO: o pacote é do commit ${commit:0:7} e a imagem padrão é sha-$REV_IMAGEM."
  echo "       Ponha NFSU2_IMAGEM=ghcr.io/th0mk/nfsu2-web-rs:sha-${commit:0:7} no $DEST/.env."
fi

mkdir -p "$DEST/module.novo" "$DEST/jogo.novo"
cp "$PACOTE/game.wasm" "$DEST/module.novo/nfsu2.wasm"
cp "$PACOTE/game.json" "$DEST/module.novo/nfsu2.json"
cp "$EXE" "$DEST/jogo.novo/SPEED2.EXE"
rm -rf "$DEST/module" "$DEST/jogo"
mv "$DEST/module.novo" "$DEST/module"
mv "$DEST/jogo.novo" "$DEST/jogo"
cp "$AQUI/compose.yml" "$DEST/compose.yml"
cp "$AQUI/env.exemplo" "$DEST/env.exemplo"
[ -f "$DEST/.env" ] || cp "$AQUI/env.exemplo" "$DEST/.env"
# o servidor roda como uid 1000 e só lê
chmod -R a+rX "$DEST/module" "$DEST/jogo"

# conferência: o que está no destino é o que veio
for par in "$PACOTE/game.wasm:$DEST/module/nfsu2.wasm" "$PACOTE/game.json:$DEST/module/nfsu2.json" "$EXE:$DEST/jogo/SPEED2.EXE" "$AQUI/compose.yml:$DEST/compose.yml"; do
  a="$(sha256sum "${par%%:*}" | cut -d' ' -f1)"; b="$(sha256sum "${par##*:}" | cut -d' ' -f1)"
  [ "$a" = "$b" ] || erro "a cópia de ${par%%:*} saiu diferente"
done
echo "Pronto em $DEST:"
( cd "$DEST" && sha256sum compose.yml module/nfsu2.wasm module/nfsu2.json jogo/SPEED2.EXE )
echo
echo "Próximo: cd $DEST && docker compose up -d && docker compose ps"
