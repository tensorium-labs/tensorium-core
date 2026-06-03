#pragma once
#include "common.h"
#ifdef __cplusplus
extern "C" {
#endif
void solo_client_run(const MinerConfig *cfg, SharedState *state);
int  build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]);
#ifdef __cplusplus
}
#endif
