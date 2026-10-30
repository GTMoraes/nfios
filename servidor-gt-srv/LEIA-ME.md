# Servidor de multiplayer próprio (GT-SRV)

O servidor oficial do nfsu2-web rodando no GT-SRV, só para você (e quem estiver na sua tailnet ou na LAN de casa).
Duas coisas:

- **Multiplayer:** o app continua rodando o jogo do iPhone; nos Ajustes, **Multiplayer › Meu servidor** liga o free
  roam, as salas e a contagem de jogadores a este servidor em vez do nfsu2.xyz.
- **Os arquivos do jogo:** o botão **Baixar do meu servidor** da tela inicial do app baixa a pasta do jogo e o
  pacote daqui, pelo Tailscale, conferindo o SHA-256 de cada arquivo. Para reinstalar ou trocar de iPhone sem
  copiar pasta à mão. Nada disso fica público: só a sua tailnet e a LAN chegam à porta.

- Imagem: `ghcr.io/th0mk/nfsu2-web-rs:sha-82742d0`, a mesma versão do pacote do app (página, módulo e servidor
  do mesmo commit).
- Acesso: `http://100.91.24.1:9091` pelo Tailscale (e `http://10.0.0.129:9091` na LAN). Sem Cloudflare Tunnel e
  sem hostname público; o roteador não encaminha a porta.
- Disco: ~40 MB no C: (o módulo e o SPEED2.EXE do multiplayer) + as imagens (~40 MB). Os arquivos para o app
  (~1,7 GB) ficam no D:, fora do C: (é cópia parada, só lida quando o app baixa). RAM: 1 GB no máximo
  (`mem_limit`), bem menos em uso.

## Instalar

**0. O que precisa:** a pasta do jogo do notebook (`C:\Games\NFSU2`), com o `SPEED2.EXE` v1.2 e o `pacote-nfsu2`
dentro (o `montar-pacote.sh` já deixou assim).

**1. Conferir a porta no GT-SRV** (WSL). A 9091 está na faixa livre do mapa, mas o `docker ps` é o que decide:

```bash
docker ps --format '{{.Names}}\t{{.Ports}}' | grep -E ':(30[0-9][0-9]|80[0-9][0-9]|9[0-9]{3})->' | sort -t: -k2 -n
```

Se a 9091 aparecer, escolha outra da faixa 9092–9889 e use-a nos passos 4, 6 e 7 (`NFSU2_PORTA` no `.env`).

**2. Levar os arquivos** para `C:\Users\GTM\mover\nfsu2\`: esta pasta (`servidor-gt-srv`) e a pasta `NFSU2` inteira do
notebook (1,7 GB; sai do C: no passo 3).

**3. Montar a pasta do servidor** (WSL; cria `~/nfsu2` e `D:\nfsu2\arquivos`, confere o EXE e a versão, copia só o
que o jogo lê e mostra os hashes):

```bash
bash /mnt/c/Users/GTM/mover/nfsu2/servidor-gt-srv/preparar.sh /mnt/c/Users/GTM/mover/nfsu2/NFSU2 \
  --arquivos /mnt/d/nfsu2/arquivos
```

Terminou com "Pronto"? Pode apagar `C:\Users\GTM\mover\nfsu2\NFSU2` (o C: agradece).

**4. Subir:**

```bash
cd ~/nfsu2 && docker compose up -d && sleep 20 && docker compose ps
```

Os quatro (`net`, `caddy`, `arquivos`, `server`) precisam estar `Up`, e o `server` como `healthy` (leva até um
minuto).

**5. Conferir o que entrou no container** (os hashes têm que bater com os que o `preparar.sh` mostrou) e se responde:

```bash
docker compose exec server sha256sum /module/nfsu2.wasm /game/SPEED2.EXE
docker compose exec arquivos sha256sum /srv/manifesto.json
curl -s http://127.0.0.1:9091/players; echo
curl -s -o /dev/null -w '%{http_code} %{size_download}\n' http://127.0.0.1:9091/arquivos/manifesto.json
```

O primeiro `curl` responde um JSON com `"players":0`; o segundo, `200` e o tamanho da lista.

**6. Firewall do Windows** (PowerShell como administrador), no molde da regra do Jellyfin: só a LAN e o Tailscale.

```powershell
New-NetFirewallRule -DisplayName "NFSU2 9091 (LAN + Tailscale)" -Direction Inbound -Protocol TCP -LocalPort 9091 `
  -RemoteAddress 10.0.0.0/24,100.64.0.0/10 -Action Allow -Profile Any
```

**7. No iPhone** (com o Tailscale ligado):
- app › Ajustes › **Meu servidor**: `http://100.91.24.1:9091`.
- Tela inicial › **Baixar do meu servidor**: baixa os ~1,7 GB conferindo cada arquivo. Deixe o app aberto (a tela
  não apaga sozinha durante o download). Se cair no meio, tocar de novo continua de onde parou.
- Para o multiplayer também passar por aqui: Ajustes › Multiplayer › **Meu servidor** › **Testar a conexão** (tem
  que dizer "Respondeu").

**8. Mapa de infra:** com o servidor no ar, acrescentar a linha da porta na tabela da seção 4
(`9091 | NFSU2 — multiplayer e arquivos do jogo para o app de iPhone (stack ~/nfsu2: pause + caddy + caddy de
arquivos + nfsu2-web-rs; arquivos em D:\nfsu2\arquivos), só Tailscale/LAN, firewall "NFSU2 9091 (LAN + Tailscale)" | —`).

## Depois

- **Logs:** `cd ~/nfsu2 && docker compose logs -f server` (as linhas `[lan]` mostram quem entra e sai).
- **Rodar o `preparar.sh` de novo** (pasta do jogo mudou, pacote novo): ele troca as pastas inteiras e, se a stack
  estiver no ar, recria os containers sozinho no fim (um container rodando ficaria preso às pastas antigas, vazias, e o
  `server` ficaria `unhealthy` com 404 no `/game.json`).
- **Atualizar** (quando o pacote do app for remontado com outro commit): rode o `preparar.sh` de novo com a pasta do
  jogo e o pacote novo, ponha `NFSU2_IMAGEM=ghcr.io/th0mk/nfsu2-web-rs:sha-<7 letras do commit>` no `~/nfsu2/.env` (o `preparar.sh`
  avisa) e `docker compose up -d`. O módulo do servidor e o do app têm que ser do mesmo build.
- **Depois de `wsl --shutdown` ou atualização do WSL:** `docker compose ps`. Se o `server` estiver reiniciando em laço
  com `usage: nfsu2-server --data` nos logs, ou o `arquivos` não subiu, o Docker subiu antes das pastas:
  `docker compose up -d --force-recreate`. (Se só o `arquivos` cair, o multiplayer continua.)
- **Parar:** `docker compose down` (o volume `state`, com as chaves do servidor, fica).
- **Jogar pelo navegador do PC:** não por este endereço. O jogo precisa de HTTPS (ou `localhost`) para as threads;
  em `http://100.91.24.1` o navegador não as libera. Este servidor é para o app: o multiplayer e os downloads.
