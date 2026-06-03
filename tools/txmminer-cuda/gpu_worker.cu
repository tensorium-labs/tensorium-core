// tools/txmminer-cuda/gpu_worker.cu
#include "gpu_worker.h"
#include "solo_client.h"    /* for build_header */
#include <cuda_runtime.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

/* Include the mining kernel declarations */
struct MiningCtx;
extern "C" {
MiningCtx *mining_ctx_create(uint16_t header_len);
void       mining_ctx_destroy(MiningCtx *ctx);
uint16_t   mining_ctx_header_len(MiningCtx *ctx);
int launch_mining_kernel_ctx(
    MiningCtx      *ctx,
    const uint8_t  *header_template,
    uint8_t         difficulty_bits,
    uint64_t        start_nonce,
    int             cuda_blocks,
    int             cuda_threads,
    uint32_t        iters_per_thread,
    uint64_t       *nonce_out
);
}

// ── Host SHA256d for share verification ──────────────────────────────────────

static void host_sha256_compress(uint32_t h[8], const uint32_t w[16]) {
    static const uint32_t K[64] = {
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    };
#define RR32(x,n) (((x)>>(n))|((x)<<(32-(n))))
    uint32_t W[64];
    for (int i = 0; i < 16; i++) W[i] = w[i];
    for (int i = 16; i < 64; i++) {
        uint32_t s0 = RR32(W[i-15],7)^RR32(W[i-15],18)^(W[i-15]>>3);
        uint32_t s1 = RR32(W[i-2],17)^RR32(W[i-2],19)^(W[i-2]>>10);
        W[i] = W[i-16] + s0 + W[i-7] + s1;
    }
    uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];
    for (int i = 0; i < 64; i++) {
        uint32_t S1 = RR32(e,6)^RR32(e,11)^RR32(e,25);
        uint32_t ch = (e&f)^(~e&g);
        uint32_t t1 = hh + S1 + ch + K[i] + W[i];
        uint32_t S0 = RR32(a,2)^RR32(a,13)^RR32(a,22);
        uint32_t maj = (a&b)^(a&c)^(b&c);
        uint32_t t2 = S0 + maj;
        hh=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;
    }
    h[0]+=a; h[1]+=b; h[2]+=c; h[3]+=d; h[4]+=e; h[5]+=f; h[6]+=g; h[7]+=hh;
#undef RR32
}

static void host_sha256(const uint8_t *data, int len, uint8_t out[32]) {
    uint32_t h[8] = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
                     0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};
    uint64_t bitlen = (uint64_t)len * 8;
    int nblocks = (len + 9 + 63) / 64;
    for (int b = 0; b < nblocks; b++) {
        uint8_t blk[64] = {0};
        int base = b * 64;
        int copy = len - base; if (copy > 64) copy = 64; if (copy > 0) memcpy(blk, data + base, copy);
        if (base < len && base + 64 > len) blk[len - base] = 0x80;
        if (b == nblocks - 1) {
            blk[56]=(uint8_t)(bitlen>>56); blk[57]=(uint8_t)(bitlen>>48);
            blk[58]=(uint8_t)(bitlen>>40); blk[59]=(uint8_t)(bitlen>>32);
            blk[60]=(uint8_t)(bitlen>>24); blk[61]=(uint8_t)(bitlen>>16);
            blk[62]=(uint8_t)(bitlen>>8);  blk[63]=(uint8_t)(bitlen);
        }
        uint32_t W[16];
        for (int i=0;i<16;i++) W[i]=((uint32_t)blk[i*4]<<24)|((uint32_t)blk[i*4+1]<<16)|((uint32_t)blk[i*4+2]<<8)|blk[i*4+3];
        host_sha256_compress(h, W);
    }
    for (int i=0;i<8;i++){out[i*4]=(uint8_t)(h[i]>>24);out[i*4+1]=(uint8_t)(h[i]>>16);out[i*4+2]=(uint8_t)(h[i]>>8);out[i*4+3]=(uint8_t)(h[i]);}
}

static void host_sha256d(const uint8_t *data, int len, uint8_t out[32]) {
    uint8_t tmp[32];
    host_sha256(data, len, tmp);
    host_sha256(tmp, 32, out);
}

static int host_leading_zeros(const uint8_t hash[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (hash[i] == 0) { bits += 8; }
        else { bits += __builtin_clz((unsigned int)hash[i] << 24); break; }
    }
    return bits;
}

// ── Share verification ────────────────────────────────────────────────────────

static int verify_share(const JobDesc *job, uint64_t nonce) {
    uint8_t header[HEADER_MAX];
    int hlen = build_header(job, nonce, header);
    if (hlen <= 0) return 0;
    uint8_t hash[32];
    host_sha256d(header, hlen, hash);
    return host_leading_zeros(hash);
}

// ── gpu_worker_thread ─────────────────────────────────────────────────────────

void *gpu_worker_thread(void *arg) {
    GpuWorkerArgs *a = (GpuWorkerArgs *)arg;
    SharedState   *s = a->state;

    if (a->gpu_id < 0 || a->gpu_id >= MAX_GPUS) {
        fprintf(stderr, "[GPU ?] invalid gpu_id=%d (MAX_GPUS=%d)\n", a->gpu_id, MAX_GPUS);
        return NULL;
    }

    /* Select CUDA device */
    cudaError_t err = cudaSetDevice(a->gpu_id);
    if (err != cudaSuccess) {
        fprintf(stderr, "[GPU %d] cudaSetDevice failed: %s\n",
                a->gpu_id, cudaGetErrorString(err));
        return NULL;
    }

    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, a->gpu_id);

    /* Register in stats */
    pthread_mutex_lock(&s->stats_mutex);
    GpuStats *gs = &s->gpu_stats[a->gpu_id];
    gs->gpu_id  = a->gpu_id;
    snprintf(gs->name, sizeof(gs->name), "%s", prop.name);
    gs->temp_c  = -1;
    gs->power_w = -1;
    gs->fan_pct = -1;
    pthread_mutex_unlock(&s->stats_mutex);

    printf("[GPU %d] %s  blocks=%d  threads=%d\n",
           a->gpu_id, prop.name, a->cuda_blocks, a->cuda_threads);
    fflush(stdout);

    /* Wait for first job */
    JobDesc job;
    job_wait(s, &job);
    if (!s->running) return NULL;

    int last_gen = s->job_generation;

    /* Pre-allocate GPU buffers */
    uint8_t probe[HEADER_MAX] = {0};
    int probe_len = build_header(&job, 0, probe);
    if (probe_len <= 0) probe_len = 122;
    MiningCtx *mctx = mining_ctx_create((uint16_t)probe_len);
    if (!mctx) {
        fprintf(stderr, "[GPU %d] failed to create MiningCtx\n", a->gpu_id);
        return NULL;
    }

    /* ITERS: target ~1B nonces per launch */
    uint32_t iters = (uint32_t)(1ULL << 30) / ((uint32_t)a->cuda_blocks * (uint32_t)a->cuda_threads);
    if (iters < 1) iters = 1;
    uint64_t nonces_per_launch = (uint64_t)a->cuda_blocks * a->cuda_threads * iters;

    uint64_t nonce = a->nonce_start;
    uint64_t hashes_since_reset = 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    while (s->running) {
        /* Check for new job (non-blocking) */
        pthread_mutex_lock(&s->job_mutex);
        if (s->job_generation != last_gen) {
            job      = s->current_job;
            last_gen = s->job_generation;
            /* Reset nonce to GPU's range start on new job */
            nonce = a->nonce_start;
            hashes_since_reset = 0;
            clock_gettime(CLOCK_MONOTONIC, &t0);
        }
        pthread_mutex_unlock(&s->job_mutex);

        /* Build header template for this batch */
        uint8_t header_tmpl[HEADER_MAX];
        int hlen = build_header(&job, nonce, header_tmpl);
        if (hlen <= 0) { usleep(100000); continue; }

        /* Recreate context if header length changed (chain_id change) */
        if ((uint16_t)hlen != mining_ctx_header_len(mctx)) {
            mining_ctx_destroy(mctx);
            mctx = mining_ctx_create((uint16_t)hlen);
            if (!mctx) { fprintf(stderr, "[GPU %d] MiningCtx realloc failed\n", a->gpu_id); break; }
        }

        /* Launch kernel — test against share_bits (lower threshold for pool shares) */
        uint64_t found_nonce = 0;
        int found = launch_mining_kernel_ctx(
            mctx, header_tmpl, job.share_bits,
            nonce, a->cuda_blocks, a->cuda_threads, iters, &found_nonce);

        hashes_since_reset += nonces_per_launch;
        nonce += nonces_per_launch;
        /* Wrap within GPU's nonce range */
        if (nonce >= a->nonce_end || nonce < a->nonce_start)
            nonce = a->nonce_start;

        /* Update hashrate stats */
        clock_gettime(CLOCK_MONOTONIC, &t1);
        double elapsed = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) * 1e-9;
        if (elapsed > 0.0) {
            pthread_mutex_lock(&s->stats_mutex);
            gs->hashrate_ghs = (double)hashes_since_reset / elapsed / 1e9;
            gs->hashes_total += nonces_per_launch;
            pthread_mutex_unlock(&s->stats_mutex);
        }

        if (found) {
            /* CPU-verify the nonce before submitting */
            int zeros = verify_share(&job, found_nonce);
            int is_share = (zeros >= (int)job.share_bits);
            int is_block = (zeros >= (int)job.difficulty_bits);

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

                /* Reset nonce range after a find to avoid duplicate work */
                nonce = a->nonce_start;
                hashes_since_reset = 0;
                clock_gettime(CLOCK_MONOTONIC, &t0);
            }
        }
    }

    mining_ctx_destroy(mctx);
    return NULL;
}
