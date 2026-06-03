// tools/txmminer-cuda/gpu_worker.h
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    int               gpu_id;
    uint64_t          nonce_start;   /* start of this GPU's nonce range */
    uint64_t          nonce_end;     /* exclusive end */
    int               cuda_blocks;
    int               cuda_threads;
    SharedState      *state;
    const MinerConfig *cfg;
} GpuWorkerArgs;

/* Entry point for pthread_create */
void *gpu_worker_thread(void *arg);

#ifdef __cplusplus
}
#endif
