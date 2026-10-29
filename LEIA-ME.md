# NFSU2 (iPhone)

App de iPhone que roda o **Need for Speed: Underground 2** do projeto [nfsu2-web](https://github.com/th0mk/nfsu2-web)
(o jogo de PC recompilado para WebAssembly, gráficos em WebGPU) **em tela cheia, como app**: sem as barras do
Safari, sem zoom em toques rápidos, sem o gesto de Início acidental durante a corrida, com vibração nas batidas e a
tela sempre ligada. iOS 26+ (WebGPU).

Dois modos:

| | **Online · nfsu2.xyz** | **Offline · sua cópia** |
|---|---|---|
| De onde vem | o site do projeto, dentro do app | o servidor local do app (`127.0.0.1:48021`), do iPhone |
| Imagem | até 120 Hz (ajuste "120 Hz"; no Safari o site fica em 60) | até 120 Hz |
| Precisa de | internet na primeira vez | o **pacote** (feito do seu SPEED2.EXE) + a **pasta do seu jogo** |
| Outros jogadores | sim: free roam, Online, crews | sim, pela ponte: com o nfsu2.xyz (padrão) ou o seu servidor (Ajustes) |
| Sem internet | depois de abrir uma vez e usar o **Preload the game data** (fim da página do site) | sempre |
| Depende do site | sim (se o site sair do ar, sobra a cópia que o modo offline do site guardou) | não |
| Saves | os do site (e na nuvem por frase-senha) | no iPhone, separados dos do modo online |

## Como funciona

- **O app** (`App/`, SwiftUI + UIKit) abre a página num `WKWebView` em tela cheia. Ele injeta um script pequeno
  (`JogoViewController.scriptDaPonte`): `navigator.standalone = true` (a página se trata como app da Tela de Início),
  `navigator.vibrate` → vibração do iPhone (Core Haptics), `navigator.audioSession.type` (tocar no silencioso ou não),
  sem seleção de texto ao segurar o dedo, e avisa o app quando a página entra e sai da corrida
  (`body[data-phase]`), para o botão de menu do app sumir durante o jogo.
- **O servidor local** (`Servidor/`, Rust, só biblioteca padrão + `libc`) é a parte do servidor oficial que a página
  usa para jogar sozinho: a página, `/game.json`, `/game.wasm`, `/data-manifest`, `/data/` (com Range) e
  `/data-blocks/v2/` (blocos de 1 MiB), com os cabeçalhos de isolamento (COOP/COEP/CORP) que liberam o
  `SharedArrayBuffer` das threads do jogo. Escuta só em `127.0.0.1`. `/vault`, `/names` e `/sw.js` respondem 404
  (sem nuvem de saves, sem nomes reservados e sem o modo offline do site, que não faz falta: o servidor está sempre
  no aparelho). A porta é fixa porque a origem (`http://127.0.0.1:48021`) é onde a página guarda os saves.
- **A ponte de multiplayer** (`Servidor/src/ponte.rs`): `/lan` (o WebSocket da LAN virtual: free roam, salas, Online)
  e `/players` vão para o servidor escolhido nos Ajustes, o nfsu2.xyz ou o seu. A ponte abre a conexão com o
  servidor (TLS em Rust, `rustls`, quando é `https`), repete o aperto de mão da página e daí em diante só copia os
  bytes nos dois sentidos. Sem servidor escolhido (ou fora do ar), `/players` responde 503 e a página entra no modo
  sem servidor dela ("Not connected to the server"), e o jogo sozinho segue normal. É o híbrido: o jogo roda do
  iPhone, com 120 Hz, e só o multiplayer sai pela rede.
- **120 Hz**: o WebKit segura a página perto de 60 quadros por segundo (preferência
  `PreferPageRenderingUpdatesNear60FPSEnabled`, a mesma de Safari › Avançado › Recursos do WebKit). O app a desliga
  no `WKWebView` dele (pela interface interna do WebKit; se um iOS futuro a tirar, o jogo só volta a 60) e declara
  `CADisableMinimumFrameDurationOnPhone`. Vale para os dois modos; o site no Safari continua em 60.
- **App-Bound Domains** (`WKAppBoundDomains` no Info.plist: `nfsu2.xyz`): é o que liga o service worker do site
  dentro do app (no `WKWebView`, service worker só existe em domínio "do app"), e com ele o modo offline do próprio
  site. `127.0.0.1` conta como do app pelo próprio WebKit (endereço local).
- **O pacote** (`pacote/montar-pacote.sh`, roda no WSL do PC): traduz o `SPEED2.EXE` para C, compila para
  WebAssembly e monta `pacote-nfsu2/` (página + módulo, ~45 MB). O módulo é feito do código da EA: fica no seu PC e
  no seu iPhone, **nunca no GitHub**. O IPA não tem nada do jogo.

## 1. Gerar o IPA (GitHub Actions, como os outros apps)

1. Crie um repositório no GitHub (por exemplo `nfsu2-ios`). Pode ser público: não há nada da EA nele.
   Privado também funciona, mas o runner macOS gasta minutos do plano (cada build leva ~10 min).
2. Suba **o conteúdo desta pasta**, menos `Servidor/target` (se existir).
3. Crie `.github/workflows/build-ipa.yml` (no GitHub: *Add file › Create new file*, digitando o caminho) com o
   conteúdo de `PARA-A-PASTA-.github-workflows/build-ipa.yml`.
4. O workflow roda a cada push: compila o servidor em Rust para iPhone (`scripts/compilar-servidor.sh`), gera o
   projeto (XcodeGen), compila, empacota e publica `NFSU2.ipa` em Releases
   (`https://github.com/<você>/<repo>/releases/latest/download/NFSU2.ipa`).
5. Instale pelo AltServer, assinando com a sua conta de desenvolvedor, como os outros.

Se o build falhar, o passo "Resumo dos erros" e o artefato `xcodebuild-log` dizem onde.

## 2. Primeiro teste: o diagnóstico

Antes de importar os 2,5 GB: abra o app › **Diagnóstico**. Ele confere, no WebKit do iPhone e na origem do modo
offline, tudo o que o jogo usa: WebGPU (e texturas ASTC), isolamento entre origens, `SharedArrayBuffer`,
`Atomics.waitAsync`, os recursos do WebAssembly do build (SIMD, tail calls, exceções exnref, threads), a memória
de 1 GB compartilhada, OPFS com acesso síncrono num worker, AudioWorklet. "Tudo o que o jogo precisa está
disponível" = pode seguir. **Compartilhar relatório** manda o texto para onde você quiser.

Depois de importar o pacote, **Compilar o módulo do jogo** confere que o módulo de ~35 MB compila neste iPhone.

## 3. Modo online

**Jogar online**: é o nfsu2.xyz em tela cheia. Para jogar sem internet depois: abra uma vez online e, na página do
site, use **Preload the game data** (no fim da página, "Game data on this PC"). O botão de menu (canto inferior
direito, fora da corrida) volta para a tela inicial do app.

## 4. Modo offline (sua cópia)

Precisa do NFSU2 de PC, **v1.2 americana**: o `SPEED2.EXE` exato (4.800.512 bytes, SHA-256
`f9dd86c054878ce6276beb07c1fd61874f7a1e4bf1f241b084c65b73e24168a7`). Outra versão não funciona (o projeto foi feito
sobre essa).

### 4.1 O pacote, no PC (WSL)

Uma vez, no Ubuntu do WSL:

```bash
sudo apt update && sudo apt install -y git python3 python3-venv curl xz-utils perl
```

E então (troque pelo caminho do seu jogo; o C: do Windows é `/mnt/c` no WSL):

```bash
bash /mnt/c/claudin/gtm/2026/random/nfsu2-ios/pacote/montar-pacote.sh "/mnt/c/Games/Need for Speed Underground 2"
```

O script trabalha em `~/nfsu2-ios-build` (no Linux: em `/mnt/c` seria bem mais lento), baixa o nfsu2-web na
versão que o app conhece, o wasi-sdk 34 e, se precisar, um Node portátil; confere o SPEED2.EXE; traduz e compila
(vários minutos: use o GT-PC, ou o notebook, que tem 12 núcleos); e deixa `pacote-nfsu2` **dentro da pasta do
jogo**. Opções:

- `--ptbr "<pasta>"`: também faz o **português do Brasil** (tradução TraduMix/MixMods, "Tradução PT-BR, V1.1",
  joao13traducoes.com; extraia com `unrar x -p'João13'` numa pasta vazia). Vai para `LANGPACKS` na pasta do jogo;
  no jogo, Settings › Language.
- `--saida "<pasta>"`: o pacote em outro lugar.
- `--rev main`: a versão mais nova do nfsu2-web (em vez da fixada; pode precisar de ajuste no servidor do app).

### 4.2 Levar para o iPhone e importar

Qualquer um destes caminhos até o app Arquivos do iPhone:

- **Compartilhamento do Windows**: compartilhe a pasta do jogo no PC; no iPhone, Arquivos › ••• ›
  **Conectar ao Servidor** › `smb://IP-do-PC`.
- **Pendrive/SSD USB-C** no iPhone.
- **iCloud Drive** (baixe a pasta toda antes: toque e segure › Baixar Agora).
- **App Dispositivos Apple do Windows** › iPhone › Arquivos › NFSU2: a pasta vai direto para dentro do app.

No app: **Importar pasta…** e escolha a pasta do jogo (com o `pacote-nfsu2` dentro, as duas coisas vêm de uma vez).

Ou, com o seu servidor no GT-SRV no ar (`servidor-gt-srv/`): **Baixar do meu servidor**, pelo Tailscale, com o
endereço em Ajustes › Meu servidor. Baixa o jogo e o pacote conferindo o SHA-256 de cada arquivo; caiu no meio,
tocar de novo continua de onde parou.
Da pasta do jogo só vem o que o jogo lê (SPEED2.EXE, server.dll, server.cfg e as pastas de dados): saves, mods e
desinstaladores do PC ficam de fora. Fica tudo em Arquivos › No Meu iPhone › NFSU2 (Jogo, Pacote, Downloads),
fora do backup do iCloud.

### 4.3 Jogar

**Jogar offline**. Os controles de toque, o mouse por toque nos menus, a inclinação (o app libera o sensor sem
perguntar), controle de PS/Xbox e teclado Bluetooth funcionam como no Safari. No modo offline o "Preload the game
data" não é necessário (os dados já estão no iPhone).

## Ajustes do app

- **Vibração nas batidas** (e no toque dos botões).
- **Som**: *Sempre tocar* (toca com o iPhone no silencioso e pausa a música de outros apps) ou *Seguir o silencioso*
  (como no Safari).
- **Orientação**: gira com o iPhone (em pé, a página mostra a imagem em cima e os controles embaixo) ou só paisagem.
- **120 Hz**: a imagem acompanha a tela ProMotion. Gasta mais bateria e esquenta mais; no Modo Pouca Energia o iOS
  volta a 60. O **Diagnóstico** mostra a taxa medida.

Valem na próxima vez que o jogo abrir.

- **Multiplayer do modo offline**: *nfsu2.xyz* (padrão), *Meu servidor* (o endereço, ex. `http://100.91.24.1:9091`
  pelo Tailscale; o kit para o GT-SRV está em `servidor-gt-srv/`) ou *Nenhum*. **Testar a conexão** pergunta o
  número de jogadores pelo mesmo caminho que a página usa. Vale na próxima conexão da página.
  Versões: o pacote é de um commit fixo do nfsu2-web e o nfsu2.xyz atualiza quando o autor publica. Hoje são o
  mesmo (`82742d0`); se o nfsu2.xyz andar e o multiplayer parar de funcionar com ele, remonte o pacote com
  `--rev main`, ou use o seu servidor, que fica na versão do pacote.

## Downloads da página

"Download log", capturas de tela (F12) e backups de saves vão para Arquivos › NFSU2 › Downloads; um aviso no topo
oferece compartilhar.

## Estrutura

- `App/`: o app. `JogoViewController.swift` (a página em tela cheia: WKWebView, ponte, downloads, diálogos,
  permissões), `InicioView.swift` (tela inicial), `Importador.swift` (cópia das pastas), `Pastas.swift` (onde fica
  cada coisa, estado do modo offline, SHA-256 do SPEED2.EXE), `ServidorLocal.swift` (liga e religa o servidor),
  `Vibrador.swift` (Core Haptics), `AjustesView.swift`, `Tema.swift`.
- `Servidor/`: o servidor local em Rust. `src/routes.rs` (rotas), `src/manifest.rs` (o manifesto, igual ao do
  oficial), `src/http.rs`, `src/lib.rs` (laço de aceitar, API C em `include/nfsu2srv.h`), `src/diag/` (a página de
  diagnóstico, embutida). `cargo test` roda os testes do contrato HTTP; `cargo run --example serve -- <pacote> <jogo>`
  serve no computador.
- `scripts/compilar-servidor.sh`: a biblioteca para iPhone, no Actions.
- `pacote/montar-pacote.sh`: o pacote, no PC.
- `servidor-gt-srv/`: o servidor de multiplayer próprio (Docker no GT-SRV, só Tailscale/LAN), com o passo a passo.
- `project.yml`: XcodeGen. `PARA-A-PASTA-.github-workflows/build-ipa.yml`: o workflow.

## O que foi verificado

- **Ponte de multiplayer**: 6 testes contra servidores remotos falsos (mais 2 de unidade, endereços e `chunked`): WebSocket por `ws://` e por `wss://` (TLS com
  certificado de teste; um certificado desconhecido é recusado), o pedido que chega ao remoto (mesmo caminho e chave,
  Host do remoto, sem Origin, que o servidor oficial exige de quem não é página dele), o quadro que chega grudado no
  101, 9 MB nos dois sentidos ao mesmo tempo, o remoto fechando, a página fechando, desligar o servidor no meio,
  recusas (503 sem servidor, 502 fora do ar, o 429 do remoto repassado, 403 de outra origem) e o `/players`
  repassado (com corpo `chunked`, `Content-Length` e HEAD). Conferidos por mutação (tirar o quadro grudado ou a
  escrita do TLS faz os testes falharem). A API C, também por um programa em C linkado na `.a`.
- **Servidor (Rust)**: 23 testes passando no total (`cargo test`, com os da ponte): rotas, cabeçalhos de isolamento, Range, blocos, pedidos
  recusados (outro Host, caminho com `..`/`%2f`, links simbólicos, arquivos fora das entradas do jogo), keep-alive e
  pedidos em sequência no mesmo pacote, 16 conexões simultâneas, religar na mesma porta (inclusive quando o
  sistema derruba o socket de escuta, como o iOS faz com app suspenso: o app confere a cada 10 s e ao voltar para a
  frente), a API C. O manifesto sai
  **byte a byte igual** ao do walk de referência do servidor oficial (o mesmo teste que o oficial usa), inclusive o
  arredondamento do mtime.
- **No Chromium** (Playwright), pelo servidor de verdade: página isolada (`crossOriginIsolated`), `/game.json` →
  `compileStreaming` do módulo, manifesto, blocos e Range como a página pede, worker com `SharedArrayBuffer`; o
  script que o app injeta (standalone, vibração, fases, seleção de texto) e a página de diagnóstico inteira.
- **WebKit (código-fonte)**: WebGPU ligado por padrão no iOS 26 também no `WKWebView`
  (`ENABLE_WEBGPU_BY_DEFAULT`); `127.0.0.1` tratado como domínio do app; service worker liberado em visão com
  `limitsNavigationsToAppBoundDomains`.
- **Swift**: sem compilador aqui (precisa de Mac). Sintaxe conferida com o parser tree-sitter-swift; assinaturas
  dos delegates do WebKit copiadas da documentação da Apple (com `@MainActor @Sendable`); APIs revisadas uma a uma.
  **O primeiro build no Actions é o primeiro compilador**: se der erro, ele aparece em "Resumo dos erros".
- **Não testado ainda**: no iPhone (a desempenho, o modo offline do nfsu2.xyz dentro do app) e o
  `montar-pacote.sh` de ponta a ponta (precisa do seu SPEED2.EXE).

## Histórico

- **0.3.0**: TestFlight (o workflow envia cada build, com a assinatura gerenciada pela Apple); só iPhone; **Baixar do
  meu servidor** (jogo e pacote do GT-SRV pelo Tailscale, conferidos por SHA-256, retomando de onde parou); o
  endereço do servidor próprio vira um ajuste próprio, usado pelo multiplayer e pelo download.
- **0.2.0**: 120 Hz; multiplayer no modo offline pela ponte (nfsu2.xyz ou servidor próprio); kit do servidor
  próprio para o GT-SRV; sem servidor de multiplayer, a página agora entra no modo sem servidor dela (antes o 404
  fazia a página achar que havia servidor).
- **0.1.0**: primeira versão. Modos online (nfsu2.xyz) e offline (servidor local), importação de pastas,
  diagnóstico, vibração, áudio, orientação, downloads.

---

Projeto de fã, sem relação com a Electronic Arts. *Need for Speed* e *Underground 2* são marcas da EA. O
nfsu2-web é MIT (th0mk, e Talha Müderrisoğlu pelo recompilador original).
