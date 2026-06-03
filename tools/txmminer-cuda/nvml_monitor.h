#pragma once
#include "common.h"
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { SharedState *state; } NvmlArgs;
void *nvml_monitor_thread(void *arg);
#ifdef __cplusplus
}
#endif
