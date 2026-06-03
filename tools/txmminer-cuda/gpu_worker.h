#pragma once
#include "common.h"
#ifdef __cplusplus
extern "C" {
#endif
typedef struct {
    int           gpu_id;
    uint64_t      nonce_start;
    uint64_t      nonce_end;
    int           cuda_blocks;
    int           cuda_threads;
    SharedState  *state;
    const MinerConfig *cfg;
} GpuWorkerArgs;
void *gpu_worker_thread(void *arg);
#ifdef __cplusplus
}
#endif
