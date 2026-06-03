// tools/txmminer-cuda/main.cpp
#include "common.h"
#include "solo_client.h"
#include "stratum_client.h"
#include "gpu_worker.h"
#include "nvml_monitor.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include <pthread.h>
#include <stdint.h>

static SharedState g_state;
static void on_signal(int s) { (void)s; g_state.running = 0; }

static void print_usage(const char *prog) {
    fprintf(stderr,
        "Tensorium Miner v" TENSORIUM_MINER_VERSION "\n\n"
        "Solo mode:\n"
        "  %s --mode solo --rpc http://HOST:PORT --wallet ADDR [options]\n\n"
        "Pool mode:\n"
        "  %s --mode pool --pool stratum+tcp://HOST:PORT --wallet ADDR [options]\n\n"
        "Options:\n"
        "  --worker NAME        worker name for pool stats (default: hostname)\n"
        "  --gpu all|0,1,2      GPUs to use (default: all)\n"
        "  --intensity auto|N   1-10 kernel size (default: auto)\n"
        "  --share-diff N       pool share difficulty (default: %llu)\n\n"
        "Backward-compat (txmminer-cuda style):\n"
        "  %s HOST:PORT ADDR [device_id] [cuda_blocks] [cuda_threads]\n",
        prog, prog, (unsigned long long)DEFAULT_SHARE_DIFF, prog);
}

/* intensity 1-10 → cuda_blocks, cuda_threads */
static void intensity_to_launch(int intensity, int *blocks, int *threads) {
    /* auto = 7 */
    if (intensity <= 0) intensity = 7;
    if (intensity > 10) intensity = 10;
    static const int BLOCKS[10]  = {1024,1024,2048,2048,4096,4096,8192,8192,12288,16384};
    static const int THREADS[10] = {128,  256, 128, 256, 128, 256, 128, 256,  256,  256};
    *blocks  = BLOCKS[intensity - 1];
    *threads = THREADS[intensity - 1];
}

/* Parse --gpu all|0|0,1,2 — returns count (0 means all) */
static int parse_gpus(const char *s, int *ids) {
    if (strcmp(s, "all") == 0) return 0;
    int count = 0;
    char buf[64];
    strncpy(buf, s, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *tok = strtok(buf, ",");
    while (tok && count < MAX_GPUS) {
        ids[count++] = atoi(tok);
        tok = strtok(NULL, ",");
    }
    return count;
}

/* Stats printer thread — prints every 5s */
static void *stats_thread(void *arg) {
    SharedState *s = (SharedState *)arg;
    while (s->running) {
        sleep(5);
        if (!s->running) break;
        pthread_mutex_lock(&s->stats_mutex);
        double total_ghs    = 0.0;
        uint64_t total_shares = 0;
        for (int i = 0; i < s->gpu_count; i++) {
            GpuStats *g = &s->gpu_stats[i];
            if (g->hashrate_ghs <= 0.0) continue;
            if (g->temp_c >= 0) {
                printf("[GPU %d] %6.2f GH/s  temp=%d°C  power=%dW  fan=%d%%  shares=%llu\n",
                       g->gpu_id, g->hashrate_ghs, g->temp_c, g->power_w, g->fan_pct,
                       (unsigned long long)g->shares_found);
            } else {
                printf("[GPU %d] %6.2f GH/s  shares=%llu\n",
                       g->gpu_id, g->hashrate_ghs,
                       (unsigned long long)g->shares_found);
            }
            total_ghs    += g->hashrate_ghs;
            total_shares += g->shares_found;
        }
        if (s->gpu_count > 1)
            printf("[total] %6.2f GH/s  shares=%llu\n\n",
                   total_ghs, (unsigned long long)total_shares);
        else
            printf("\n");
        fflush(stdout);
        pthread_mutex_unlock(&s->stats_mutex);
    }
    return NULL;
}

typedef struct { const MinerConfig *cfg; SharedState *state; } NetThreadArgs;

static void *solo_thread(void *arg) {
    NetThreadArgs *a = (NetThreadArgs *)arg;
    solo_client_run(a->cfg, a->state);
    return NULL;
}

static void *pool_thread(void *arg) {
    NetThreadArgs *a = (NetThreadArgs *)arg;
    stratum_client_run(a->cfg, a->state);
    return NULL;
}

int main(int argc, char *argv[]) {
    signal(SIGINT,  on_signal);
    signal(SIGTERM, on_signal);

    MinerConfig cfg;
    memset(&cfg, 0, sizeof(cfg));

    /* Defaults */
    cfg.mode       = MODE_SOLO;
    strncpy(cfg.rpc_host, "127.0.0.1", sizeof(cfg.rpc_host) - 1);
    strncpy(cfg.rpc_port, "33332",     sizeof(cfg.rpc_port) - 1);
    strncpy(cfg.worker,   "miner",     sizeof(cfg.worker) - 1);
    cfg.gpu_count  = 0;   /* 0 = all GPUs */
    cfg.share_diff = DEFAULT_SHARE_DIFF;

    int use_intensity = 7; /* default auto = 7 */

    /* ── Backward-compat mode: tensorium-miner HOST:PORT ADDR [dev] [blks] [thr] ── */
    if (argc >= 3 && argv[1][0] != '-') {
        const char *colon = strrchr(argv[1], ':');
        if (colon) {
            int hl = (int)(colon - argv[1]);
            if (hl > 0 && hl < 128) {
                memcpy(cfg.rpc_host, argv[1], hl);
                cfg.rpc_host[hl] = '\0';
            }
            strncpy(cfg.rpc_port, colon + 1, sizeof(cfg.rpc_port) - 1);
        }
        strncpy(cfg.wallet, argv[2], sizeof(cfg.wallet) - 1);
        int dev     = argc > 3 ? atoi(argv[3]) : 0;
        int blocks  = argc > 4 ? atoi(argv[4]) : 8192;
        int threads = argc > 5 ? atoi(argv[5]) : 256;
        cfg.gpu_ids[0] = dev;
        cfg.gpu_count  = 1;
        cfg.cuda_blocks  = blocks;
        cfg.cuda_threads = threads;
        cfg.mode = MODE_SOLO;
        goto run;
    }

    /* ── Flag-based mode ── */
    for (int i = 1; i < argc; i++) {
#define NEXTARG() ((i + 1 < argc) ? argv[++i] : "")
        if      (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            print_usage(argv[0]); return 0;
        }
        else if (strcmp(argv[i], "--mode") == 0) {
            const char *m = NEXTARG();
            cfg.mode = (strcmp(m, "pool") == 0) ? MODE_POOL : MODE_SOLO;
        }
        else if (strcmp(argv[i], "--rpc") == 0) {
            const char *url = NEXTARG();
            if (strncmp(url, "http://", 7) == 0) url += 7;
            const char *c = strrchr(url, ':');
            if (c) {
                int hl = (int)(c - url);
                if (hl > 0 && hl < 128) {
                    memcpy(cfg.rpc_host, url, hl);
                    cfg.rpc_host[hl] = '\0';
                }
                strncpy(cfg.rpc_port, c + 1, sizeof(cfg.rpc_port) - 1);
            }
        }
        else if (strcmp(argv[i], "--pool") == 0) {
            const char *url = NEXTARG();
            if (strncmp(url, "stratum+tcp://", 14) == 0) url += 14;
            const char *c = strrchr(url, ':');
            if (c) {
                int hl = (int)(c - url);
                if (hl > 0 && hl < 128) {
                    memcpy(cfg.pool_host, url, hl);
                    cfg.pool_host[hl] = '\0';
                }
                strncpy(cfg.pool_port, c + 1, sizeof(cfg.pool_port) - 1);
            }
        }
        else if (strcmp(argv[i], "--wallet") == 0 ||
                 strcmp(argv[i], "--reward-address") == 0) {
            strncpy(cfg.wallet, NEXTARG(), sizeof(cfg.wallet) - 1);
        }
        else if (strcmp(argv[i], "--worker") == 0) {
            strncpy(cfg.worker, NEXTARG(), sizeof(cfg.worker) - 1);
        }
        else if (strcmp(argv[i], "--gpu") == 0) {
            cfg.gpu_count = parse_gpus(NEXTARG(), cfg.gpu_ids);
        }
        else if (strcmp(argv[i], "--intensity") == 0) {
            const char *iv = NEXTARG();
            use_intensity = (strcmp(iv, "auto") == 0) ? 7 : atoi(iv);
        }
        else if (strcmp(argv[i], "--share-diff") == 0) {
            cfg.share_diff = (uint64_t)strtoull(NEXTARG(), NULL, 10);
            if (cfg.share_diff == 0) cfg.share_diff = DEFAULT_SHARE_DIFF;
        }
        else {
            fprintf(stderr, "unknown flag: %s\n", argv[i]);
            print_usage(argv[0]); return 1;
        }
#undef NEXTARG
    }

    /* Validate required args */
    if (cfg.wallet[0] == '\0') {
        fprintf(stderr, "error: --wallet is required\n");
        print_usage(argv[0]); return 1;
    }
    if (cfg.mode == MODE_POOL && cfg.pool_host[0] == '\0') {
        fprintf(stderr, "error: --pool is required in pool mode\n");
        return 1;
    }

    /* Compute cuda_blocks/threads from intensity */
    if (cfg.cuda_blocks == 0)
        intensity_to_launch(use_intensity, &cfg.cuda_blocks, &cfg.cuda_threads);

run:;
    /* ── GPU discovery ── */
    int total_gpus = 0;
    cudaGetDeviceCount(&total_gpus);
    if (total_gpus == 0) {
        fprintf(stderr, "error: no CUDA GPUs found\n");
        return 1;
    }

    int gpu_ids[MAX_GPUS];
    int gpu_count;
    if (cfg.gpu_count == 0) {
        /* Use all GPUs */
        gpu_count = (total_gpus > MAX_GPUS) ? MAX_GPUS : total_gpus;
        for (int i = 0; i < gpu_count; i++) gpu_ids[i] = i;
    } else {
        gpu_count = cfg.gpu_count;
        memcpy(gpu_ids, cfg.gpu_ids, gpu_count * sizeof(int));
        /* Validate GPU ids */
        for (int i = 0; i < gpu_count; i++) {
            if (gpu_ids[i] < 0 || gpu_ids[i] >= total_gpus) {
                fprintf(stderr, "error: GPU %d not found (only %d GPUs available)\n",
                        gpu_ids[i], total_gpus);
                return 1;
            }
        }
    }

    /* ── Print banner ── */
    printf("tensorium-miner v" TENSORIUM_MINER_VERSION " — %d GPU(s)\n\n", gpu_count);
    if (cfg.mode == MODE_SOLO) {
        printf("mode=solo  rpc=%s:%s  wallet=%.24s...\n\n",
               cfg.rpc_host, cfg.rpc_port, cfg.wallet);
    } else {
        printf("mode=pool  pool=%s:%s  worker=%s  share_diff=%llu\n\n",
               cfg.pool_host, cfg.pool_port, cfg.worker,
               (unsigned long long)cfg.share_diff);
    }
    fflush(stdout);

    /* ── Init shared state ── */
    shared_state_init(&g_state);
    g_state.gpu_count = gpu_count;

    /* ── Nonce space split across GPUs ── */
    uint64_t range = (gpu_count > 0) ? (UINT64_MAX / (uint64_t)gpu_count) : UINT64_MAX;

    /* ── Spawn GPU worker threads ── */
    pthread_t       gpu_threads[MAX_GPUS];
    GpuWorkerArgs   gpu_args[MAX_GPUS];
    for (int i = 0; i < gpu_count; i++) {
        gpu_args[i].gpu_id       = gpu_ids[i];
        gpu_args[i].nonce_start  = (uint64_t)i * range;
        gpu_args[i].nonce_end    = (i == gpu_count - 1) ? UINT64_MAX : (uint64_t)(i + 1) * range;
        gpu_args[i].cuda_blocks  = cfg.cuda_blocks;
        gpu_args[i].cuda_threads = cfg.cuda_threads;
        gpu_args[i].state        = &g_state;
        gpu_args[i].cfg          = &cfg;
        if (pthread_create(&gpu_threads[i], NULL, gpu_worker_thread, &gpu_args[i]) != 0) {
            fprintf(stderr, "error: failed to create GPU %d thread\n", gpu_ids[i]);
            g_state.running = 0;
            goto cleanup;
        }
    }

    {
        /* ── Spawn network client thread ── */
        pthread_t    net_thread;
        NetThreadArgs net_args = { &cfg, &g_state };
        pthread_create(&net_thread, NULL,
                       cfg.mode == MODE_SOLO ? solo_thread : pool_thread,
                       &net_args);

        /* ── Spawn stats printer thread ── */
        pthread_t stats_t;
        pthread_create(&stats_t, NULL, stats_thread, &g_state);

#ifdef WITH_NVML
        /* ── Spawn NVML monitor thread ── */
        pthread_t nvml_t;
        NvmlArgs  nvml_args = { &g_state };
        pthread_create(&nvml_t, NULL, nvml_monitor_thread, &nvml_args);
#endif

        /* ── Wait for all threads ── */
        for (int i = 0; i < gpu_count; i++) pthread_join(gpu_threads[i], NULL);
        pthread_join(net_thread, NULL);
        pthread_join(stats_t, NULL);
#ifdef WITH_NVML
        pthread_join(nvml_t, NULL);
#endif
    }

cleanup:
    return 0;
}
