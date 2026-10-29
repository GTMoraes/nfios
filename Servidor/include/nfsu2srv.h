// nfsu2srv: o servidor HTTP local do app (Servidor/src/lib.rs), em Rust, linkado como libnfsu2srv.a.
// Escuta só em 127.0.0.1.  Todas as funções podem ser chamadas de qualquer thread.
#ifndef NFSU2SRV_H
#define NFSU2SRV_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Liga (ou religa) o servidor em 127.0.0.1:port, servindo pack_dir (web/, game.wasm, game.json) e game_dir
/// (SPEED2.EXE e as pastas de dados).  Já ligado, vivo e com as mesmas pastas e porta: não faz nada.
/// Devolve a porta (> 0) ou -1 (nfsu2srv_last_error diz por quê).
int32_t nfsu2srv_start(const char *pack_dir, const char *game_dir, uint16_t port);

/// Desliga o servidor (as conexões abertas fecham antes do próximo pedido).
void nfsu2srv_stop(void);

/// 1 se o servidor está aceitando conexões; 0 se não (nunca ligado, desligado, ou o iOS fechou o socket).
int32_t nfsu2srv_alive(void);

/// A porta em uso (0: desligado).
int32_t nfsu2srv_port(void);

/// Copia a última mensagem de erro (UTF-8, terminada em NUL) para buf, até len bytes; devolve o tamanho dela.
int32_t nfsu2srv_last_error(char *buf, int32_t len);

/// O servidor de multiplayer da ponte (/lan e /players vão para ele): "https://nfsu2.xyz", "http://100.91.24.1:9091"...
/// NULL ou "": nenhum (a página fica sem multiplayer).  Vale para as próximas conexões.
/// Devolve 0, ou -1 se o endereço é inválido (o anterior continua; nfsu2srv_last_error diz por quê).
int32_t nfsu2srv_set_remote(const char *url);

#ifdef __cplusplus
}
#endif

#endif
