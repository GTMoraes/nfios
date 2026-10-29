# Servidor de multiplayer próprio (GT-SRV)

O servidor oficial do nfsu2-web rodando no GT-SRV, só para você (e quem estiver na sua tailnet ou na LAN de casa).
O app continua rodando o jogo do iPhone; nos Ajustes do app, **Multiplayer › Meu servidor** liga o free roam, as
salas e a contagem de jogadores a este servidor em vez do nfsu2.xyz.

- Imagem: `ghcr.io/th0mk/nfsu2-web-rs:sha-82742d0`, a mesma versão do pacote do app (página, módulo e servidor
  do mesmo commit).
- Acesso: `http://100.91.24.1:9091` pelo Tailscale (e `http://10.0.0.129:9091` na LAN). Sem Cloudflare Tunnel e
  sem hostname público; o roteador não encaminha a porta.
- Disco: ~70 MB no C: (o módulo e o SPEED2.EXE; para o multiplayer o servidor não precisa do resto do jogo) + as
  imagens (~40 MB). RAM: 1 GB no máximo (`mem_limit`), bem menos em uso.

## Instalar

**0. O que precisa:** a pasta `pacote-nfsu2` que o `montar-pacote.sh` gerou e o `SPEED2.EXE` v1.2.

**1. Conferir a porta no GT-SRV** (WSL). A 9091 está na faixa livre do mapa, mas o `docker ps` é o que decide:

```bash
docker ps --format '{{.Names}}\t{{.Ports}}' | grep -E ':(30[0-9][0-9]|80[0-9][0-9]|9[0-9]{3})->' | sort -t: -k2 -n
```

Se a 9091 aparecer, escolha outra da faixa 9092–9889 e use-a nos passos 4, 6 e 7 (`NFSU2_PORTA` no `.env`).

**2. Levar os arquivos** para `C:\Users\GTM\mover\nfsu2\`: esta pasta (`servidor-gt-srv`), a pasta `pacote-nfsu2` e o
`SPEED2.EXE`.

**3. Montar a pasta do servidor** (WSL; cria `~/nfsu2`, confere o EXE e a versão, e mostra os hashes):

```bash
bash /mnt/c/Users/GTM/mover/nfsu2/servidor-gt-srv/preparar.sh \
  /mnt/c/Users/GTM/mover/nfsu2/pacote-nfsu2 /mnt/c/Users/GTM/mover/nfsu2/SPEED2.EXE
```

**4. Subir:**

```bash
cd ~/nfsu2 && docker compose up -d && sleep 20 && docker compose ps
```

Os três (`net`, `caddy`, `server`) precisam estar `Up`, e o `server` como `healthy` (leva até um minuto).

**5. Conferir o que entrou no container** (os hashes têm que bater com os que o `preparar.sh` mostrou) e se responde:

```bash
docker compose exec server sha256sum /module/nfsu2.wasm /game/SPEED2.EXE
curl -s http://127.0.0.1:9091/players; echo
```

O `curl` responde um JSON com `"players":0`.

**6. Firewall do Windows** (PowerShell como administrador), no molde da regra do Jellyfin: só a LAN e o Tailscale.

```powershell
New-NetFirewallRule -DisplayName "NFSU2 9091 (LAN + Tailscale)" -Direction Inbound -Protocol TCP -LocalPort 9091 `
  -RemoteAddress 10.0.0.0/24,100.64.0.0/10 -Action Allow -Profile Any
```

**7. No iPhone:** app › Ajustes › Multiplayer do modo offline › **Meu servidor** › `http://100.91.24.1:9091` ›
**Testar a conexão** (com o Tailscale ligado no iPhone). Tem que dizer "Respondeu".

**8. Mapa de infra:** com o servidor no ar, acrescentar a linha da porta na tabela da seção 4
(`9091 | NFSU2 — servidor de multiplayer do jogo (stack ~/nfsu2: pause + caddy + nfsu2-web-rs), só Tailscale/LAN,
firewall "NFSU2 9091 (LAN + Tailscale)" | —`).

## Depois

- **Logs:** `cd ~/nfsu2 && docker compose logs -f server` (as linhas `[lan]` mostram quem entra e sai).
- **Atualizar** (quando o pacote do app for remontado com outro commit): rode o `preparar.sh` de novo com o pacote
  novo, ponha `NFSU2_IMAGEM=ghcr.io/th0mk/nfsu2-web-rs:sha-<7 letras do commit>` no `~/nfsu2/.env` (o `preparar.sh`
  avisa) e `docker compose up -d`. O módulo do servidor e o do app têm que ser do mesmo build.
- **Depois de `wsl --shutdown` ou atualização do WSL:** `docker compose ps`. Se o `server` estiver reiniciando em laço
  com `usage: nfsu2-server --data` nos logs, o Docker subiu antes da pasta: `docker compose up -d --force-recreate`.
- **Parar:** `docker compose down` (o volume `state`, com as chaves do servidor, fica).
- **Jogar pelo navegador do PC:** não por este endereço. O jogo precisa de HTTPS (ou `localhost`) para as threads;
  em `http://100.91.24.1` o navegador não as libera. Só o multiplayer do app passa por aqui.
