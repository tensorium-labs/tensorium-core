// tools/tensorium-miner/nvml_monitor.cpp
#include "nvml_monitor.h"
#include <stdio.h>
#include <unistd.h>

#ifdef WITH_NVML
#include <nvml.h>

void *nvml_monitor_thread(void *arg) {
    NvmlArgs    *a = (NvmlArgs *)arg;
    SharedState *s = a->state;

    if (nvmlInit() != NVML_SUCCESS) {
        fprintf(stderr, "[nvml] init failed — GPU temp/power stats unavailable\n");
        return NULL;
    }

    while (s->running) {
        sleep(30);
        if (!s->running) break;

        pthread_mutex_lock(&s->stats_mutex);
        for (int i = 0; i < s->gpu_count; i++) {
            GpuStats *g = &s->gpu_stats[i];

            nvmlDevice_t dev;
            if (nvmlDeviceGetHandleByIndex((unsigned int)g->gpu_id, &dev) != NVML_SUCCESS)
                continue;

            unsigned int val = 0;

            if (nvmlDeviceGetTemperature(dev, NVML_TEMPERATURE_GPU, &val) == NVML_SUCCESS)
                g->temp_c = (int)val;

            if (nvmlDeviceGetPowerUsage(dev, &val) == NVML_SUCCESS)
                g->power_w = (int)(val / 1000);   /* milliwatts → watts */

            if (nvmlDeviceGetFanSpeed(dev, &val) == NVML_SUCCESS)
                g->fan_pct = (int)val;
        }
        pthread_mutex_unlock(&s->stats_mutex);
    }

    nvmlShutdown();
    return NULL;
}

#else  /* !WITH_NVML */

void *nvml_monitor_thread(void *arg) {
    (void)arg;
    return NULL;   /* graceful no-op when compiled without NVML */
}

#endif /* WITH_NVML */
