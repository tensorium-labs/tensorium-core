// tools/txmminer-cuda/common.h
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <pthread.h>
#include <stdatomic.h>
#include <unistd.h>

#define TENSORIUM_MINER_VERSION "2.0.0"
#define DEFAULT_SHARE_DIFF 1048576ULL   /* ~20 bits */
#define MAX_GPUS           16
#define JOB_ID_LEN         48
#define ADDR_LEN           128
#define WORKER_LEN         64
#define CHAIN_ID_LEN       64
#define HEADER_MAX         192
#define SHARE_QUEUE_MAX    256

typedef enum { MODE_SOLO = 0, MODE_POOL = 1 } MiningMode;

typedef struct {
    MiningMode  mode;
    char        rpc_host[128];
    char        rpc_port[16];
    char        pool_host[128];
    char        pool_port[16];
    char        wallet[ADDR_LEN];
    char        worker[WORKER_LEN];
    int         gpu_ids[MAX_GPUS];
    int         gpu_count;       /* 0 = use all */
    int         cuda_blocks;
    int         cuda_threads;
    uint64_t    share_diff;
    int         nvml_enabled;
} MinerConfig;

typedef struct {
    char     job_id[JOB_ID_LEN];
    char     chain_id[CHAIN_ID_LEN];
    uint64_t height;
    uint8_t  previous_hash[32];
    uint8_t  merkle_root[32];
    uint64_t timestamp;
    uint8_t  difficulty_bits;    /* network difficulty */
    uint8_t  share_bits;         /* floor(log2(share_diff)) */
    uint64_t version;
    int      valid;              /* 0 = no job yet */
} JobDesc;

typedef struct {
    char     job_id[JOB_ID_LEN];
    char     worker[WORKER_LEN];
    uint64_t nonce;
    int      gpu_id;
    int      is_block;           /* 1 if meets network difficulty */
} ShareResult;

typedef struct {
    int      gpu_id;
    char     name[256];
    double   hashrate_ghs;
    int      temp_c;             /* -1 if NVML unavailable */
    int      power_w;
    int      fan_pct;
    uint64_t shares_found;
    uint64_t hashes_total;
} GpuStats;

/* Shared state between all threads */
typedef struct {
    /* Current job — updated by solo/stratum client thread */
    pthread_mutex_t  job_mutex;
    pthread_cond_t   job_cond;
    JobDesc          current_job;
    int              job_generation; /* increments on each new job */

    /* Share queue — GPU workers push, client thread pops */
    pthread_mutex_t  share_mutex;
    pthread_cond_t   share_cond;
    ShareResult      share_queue[SHARE_QUEUE_MAX];
    int              share_head;
    int              share_tail;
    int              share_count;

    /* Per-GPU stats — written by GPU threads, read by stats printer */
    pthread_mutex_t  stats_mutex;
    GpuStats         gpu_stats[MAX_GPUS];
    int              gpu_count;

    volatile int     running; /* 0 = shutdown */
} SharedState;

static inline void shared_state_init(SharedState *s) {
    memset(s, 0, sizeof(*s));
    pthread_mutex_init(&s->job_mutex, NULL);
    pthread_cond_init(&s->job_cond, NULL);
    pthread_mutex_init(&s->share_mutex, NULL);
    /* Use CLOCK_MONOTONIC for share_cond so NTP adjustments don't affect timeouts */
    pthread_condattr_t attr;
    pthread_condattr_init(&attr);
    pthread_condattr_setclock(&attr, CLOCK_MONOTONIC);
    pthread_cond_init(&s->share_cond, &attr);
    pthread_condattr_destroy(&attr);
    pthread_mutex_init(&s->stats_mutex, NULL);
    s->running = 1;
}

/* Push share — called from GPU worker thread */
static inline void share_push(SharedState *s, const ShareResult *r) {
    pthread_mutex_lock(&s->share_mutex);
    if (s->share_count < SHARE_QUEUE_MAX) {
        s->share_queue[s->share_tail] = *r;
        s->share_tail = (s->share_tail + 1) % SHARE_QUEUE_MAX;
        s->share_count++;
        pthread_cond_signal(&s->share_cond);
    }
    pthread_mutex_unlock(&s->share_mutex);
}

/* Pop share — blocks up to 1s, returns 1 if got share */
static inline int share_pop(SharedState *s, ShareResult *r) {
    pthread_mutex_lock(&s->share_mutex);
    while (s->share_count == 0 && s->running) {
        struct timespec ts;
        if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
            /* clock unavailable — fall back to unconditional wait with short sleep */
            pthread_mutex_unlock(&s->share_mutex);
            usleep(50000);
            pthread_mutex_lock(&s->share_mutex);
            continue;
        }
        ts.tv_sec += 1;
        pthread_cond_timedwait(&s->share_cond, &s->share_mutex, &ts);
    }
    if (s->share_count == 0) {
        pthread_mutex_unlock(&s->share_mutex);
        return 0;
    }
    *r = s->share_queue[s->share_head];
    s->share_head = (s->share_head + 1) % SHARE_QUEUE_MAX;
    s->share_count--;
    pthread_mutex_unlock(&s->share_mutex);
    return 1;
}

/* Publish new job — wakes all GPU workers */
static inline void job_publish(SharedState *s, const JobDesc *j) {
    pthread_mutex_lock(&s->job_mutex);
    s->current_job = *j;
    s->job_generation++;
    pthread_cond_broadcast(&s->job_cond);
    pthread_mutex_unlock(&s->job_mutex);
}

/* Wait for first valid job — called from GPU worker at startup */
static inline void job_wait(SharedState *s, JobDesc *out) {
    pthread_mutex_lock(&s->job_mutex);
    while (!s->current_job.valid && s->running)
        pthread_cond_wait(&s->job_cond, &s->job_mutex);
    *out = s->current_job;
    pthread_mutex_unlock(&s->job_mutex);
}

/* Compute share_bits from share_diff (floor log2) */
static inline uint8_t share_bits_from_diff(uint64_t diff) {
    uint8_t bits = 0;
    while (diff > 1) { diff >>= 1; bits++; }
    return bits;
}
