// tools/tensorium-miner/gpu_worker.cu
#include "gpu_worker.h"
#include "solo_client.h"       /* build_header */
#include "host_tensorhash.h"   /* host verification of found nonces */
#include "tensorhash_params.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

/* TensorHash kernel C interface (tensorhash_kernel.cu) */
struct TensorHashCtx;
extern "C" {
TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out);
void   th_ctx_destroy(TensorHashCtx *ctx);
int    th_ctx_seed_matches(TensorHashCtx *ctx, const uint8_t seed[32]);
int    th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32], int spot_count);
double th_last_dataset_gen_seconds(TensorHashCtx *ctx);
int    th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                        uint16_t header_len, uint8_t difficulty_bits,
                        uint64_t start_nonce, int blocks, int threads,
                        uint32_t iters_per_thread, uint64_t *nonce_out);
}

/* Verify a found nonce on the CPU before submitting (cheap: K=32 elements
   recomputed on demand). Returns the leading-zero-bit count. */
static int verify_share(const JobDesc *job, uint64_t nonce) {
    uint8_t header[HEADER_MAX];
    int hlen = build_header(job, nonce, header);
    if (hlen <= 8) return 0;
    uint8_t hash[32];
    host_pow_hash(header, (uint32_t)(hlen - 8), nonce, job->epoch_seed, hash);
    return host_leading_zero_bits(hash);
}

/* (Re)generate the dataset for the job's epoch seed if it changed.
   Returns 0 on success. */
static int ensure_dataset(TensorHashCtx *ctx, int gpu_id, const JobDesc *job) {
    if (th_ctx_seed_matches(ctx, job->epoch_seed)) return 0;
    printf("[GPU %d] generating %.1f GiB TensorHash dataset (epoch seed changed)...\n",
           gpu_id, (double)TH_DATASET_BYTES / (1024.0 * 1024.0 * 1024.0));
    fflush(stdout);
    int rc = th_ctx_generate_dataset(ctx, job->epoch_seed, 4096);
    if (rc != 0) {
        fprintf(stderr, "[GPU %d] dataset generation/spot-check failed (rc=%d)\n",
                gpu_id, rc);
        return rc;
    }
    printf("[GPU %d] dataset ready in %.1fs (spot-check passed)\n",
           gpu_id, th_last_dataset_gen_seconds(ctx));
    fflush(stdout);
    return 0;
}

void *gpu_worker_thread(void *arg) {
    GpuWorkerArgs *a = (GpuWorkerArgs *)arg;
    SharedState   *s = a->state;

    if (a->gpu_id < 0 || a->gpu_id >= MAX_GPUS) {
        fprintf(stderr, "[GPU ?] invalid gpu_id=%d (MAX_GPUS=%d)\n", a->gpu_id, MAX_GPUS);
        return NULL;
    }

    cudaError_t err = cudaSetDevice(a->gpu_id);
    if (err != cudaSuccess) {
        fprintf(stderr, "[GPU %d] cudaSetDevice failed: %s\n",
                a->gpu_id, cudaGetErrorString(err));
        return NULL;
    }

    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, a->gpu_id);

    /* VRAM gate: TensorHash needs the full dataset resident. */
    int th_err = 0;
    size_t free_b = 0;
    TensorHashCtx *ctx = th_ctx_create(&th_err, &free_b);
    if (!ctx) {
        if (th_err == 1) {
            fprintf(stderr,
                "[GPU %d] %s has only %.1f GiB free VRAM — TensorHash v1 needs "
                "~20 GiB (dataset 17.9 GiB + headroom).\n"
                "[GPU %d] Minimum supported card: RTX 3090 / 24 GB.\n",
                a->gpu_id, prop.name, (double)free_b / (1024.0 * 1024.0 * 1024.0),
                a->gpu_id);
        } else {
            fprintf(stderr, "[GPU %d] TensorHash context allocation failed\n", a->gpu_id);
        }
        return NULL;
    }

    pthread_mutex_lock(&s->stats_mutex);
    GpuStats *gs = &s->gpu_stats[a->gpu_id];
    gs->gpu_id  = a->gpu_id;
    snprintf(gs->name, sizeof(gs->name), "%s", prop.name);
    gs->temp_c  = -1;
    gs->power_w = -1;
    gs->fan_pct = -1;
    pthread_mutex_unlock(&s->stats_mutex);

    printf("[GPU %d] %s  blocks=%d  threads=%d  (TensorHash v1)\n",
           a->gpu_id, prop.name, a->cuda_blocks, a->cuda_threads);
    fflush(stdout);

    JobDesc job;
    job_wait(s, &job);
    if (!s->running) { th_ctx_destroy(ctx); return NULL; }

    int last_gen = s->job_generation;
    if (ensure_dataset(ctx, a->gpu_id, &job) != 0) { th_ctx_destroy(ctx); return NULL; }

    /* ~16M nonces per launch: at TensorHash rates (tens of MH/s) that is a
       sub-second launch, keeping job switchover latency low. */
    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)a->cuda_blocks * (uint64_t)a->cuda_threads));
    if (iters < 1) iters = 1;
    uint64_t nonces_per_launch = (uint64_t)a->cuda_blocks * a->cuda_threads * iters;

    uint64_t nonce = a->nonce_start;
    uint64_t hashes_since_reset = 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    while (s->running) {
        pthread_mutex_lock(&s->job_mutex);
        if (s->job_generation != last_gen) {
            job      = s->current_job;
            last_gen = s->job_generation;
            nonce = a->nonce_start;
            hashes_since_reset = 0;
            clock_gettime(CLOCK_MONOTONIC, &t0);
        }
        pthread_mutex_unlock(&s->job_mutex);

        if (ensure_dataset(ctx, a->gpu_id, &job) != 0) break;

        uint8_t header_tmpl[HEADER_MAX];
        int hlen = build_header(&job, nonce, header_tmpl);
        if (hlen <= 8) { usleep(100000); continue; }

        uint64_t found_nonce = 0;
        int found = th_launch_mining(ctx, header_tmpl, (uint16_t)hlen,
                                     job.share_bits, nonce,
                                     a->cuda_blocks, a->cuda_threads, iters,
                                     &found_nonce);

        hashes_since_reset += nonces_per_launch;
        nonce += nonces_per_launch;
        if (nonce >= a->nonce_end || nonce < a->nonce_start)
            nonce = a->nonce_start;

        clock_gettime(CLOCK_MONOTONIC, &t1);
        double elapsed = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) * 1e-9;
        if (elapsed > 0.0) {
            pthread_mutex_lock(&s->stats_mutex);
            gs->hashrate_ghs = (double)hashes_since_reset / elapsed / 1e9;
            gs->hashes_total += nonces_per_launch;
            pthread_mutex_unlock(&s->stats_mutex);
        }

        if (found) {
            int zeros = verify_share(&job, found_nonce);
            int is_share = (zeros >= (int)job.share_bits);
            int is_block = (zeros >= (int)job.difficulty_bits);

            if (!is_share && !is_block) {
                fprintf(stderr,
                    "[GPU %d] kernel result FAILED host verification "
                    "(nonce=%llu zeros=%d) — possible GPU memory fault\n",
                    a->gpu_id, (unsigned long long)found_nonce, zeros);
            }

            if (is_share || is_block) {
                pthread_mutex_lock(&s->stats_mutex);
                gs->shares_found++;
                pthread_mutex_unlock(&s->stats_mutex);

                ShareResult sr;
                memset(&sr, 0, sizeof(sr));
                strncpy(sr.job_id, job.job_id, JOB_ID_LEN - 1);
                strncpy(sr.worker, a->cfg->worker, WORKER_LEN - 1);
                sr.nonce    = found_nonce;
                sr.gpu_id   = a->gpu_id;
                sr.is_block = is_block;
                share_push(s, &sr);

                nonce = found_nonce + nonces_per_launch;
                if (nonce >= a->nonce_end || nonce < a->nonce_start)
                    nonce = a->nonce_start;
                hashes_since_reset = 0;
                clock_gettime(CLOCK_MONOTONIC, &t0);
            }
        }
    }

    th_ctx_destroy(ctx);
    return NULL;
}
