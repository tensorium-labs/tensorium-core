// tools/tensorium-miner/modes.h
// Standalone run modes: --selftest, --benchmark, --mode genesis.
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/* All return a process exit code (0 = success). */
int run_selftest(int gpu_id);
int run_benchmark(int gpu_id, int seconds, int cuda_blocks, int cuda_threads);
int run_genesis(const char *prefix_hex, int bits, uint64_t start_nonce,
                int gpu_count, const int *gpu_ids,
                int cuda_blocks, int cuda_threads);

#ifdef __cplusplus
}
#endif
