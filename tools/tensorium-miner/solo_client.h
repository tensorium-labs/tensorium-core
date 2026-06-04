// tools/tensorium-miner/solo_client.h
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Blocking: polls node, publishes jobs to SharedState, submits shares from queue.
   Returns only when state->running == 0. */
void solo_client_run(const MinerConfig *cfg, SharedState *state);

/* Build Tensorium block header bytes from job + nonce.
   Returns header length in bytes (122 for mainnet chain_id).
   Returns 0 on error. */
int build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]);

#ifdef __cplusplus
}
#endif
