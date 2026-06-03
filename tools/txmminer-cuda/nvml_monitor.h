// tools/txmminer-cuda/nvml_monitor.h
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    SharedState *state;
} NvmlArgs;

/* Entry point for pthread_create.
   Polls NVML every 30s for temp, power, fan per GPU.
   No-op if compiled without WITH_NVML or if nvmlInit fails. */
void *nvml_monitor_thread(void *arg);

#ifdef __cplusplus
}
#endif
