#include "gpu_worker.h"
#include <stdio.h>
void *gpu_worker_thread(void *arg) {
    GpuWorkerArgs *a = (GpuWorkerArgs *)arg;
    fprintf(stderr, "[GPU %d] worker stub\n", a->gpu_id);
    return NULL;
}
