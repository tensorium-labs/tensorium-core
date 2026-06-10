// tools/tensorium-miner/modes.cpp
#include "modes.h"
#include "host_tensorhash.h"
#include "tensorhash_params.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <pthread.h>

struct TensorHashCtx;
extern "C" {
TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out);
void   th_ctx_destroy(TensorHashCtx *ctx);
int    th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32], int spot_count);
double th_last_dataset_gen_seconds(TensorHashCtx *ctx);
int    th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                        uint16_t header_len, uint8_t difficulty_bits,
                        uint64_t start_nonce, int blocks, int threads,
                        uint32_t iters_per_thread, uint64_t *nonce_out);
int    th_hash_one(TensorHashCtx *ctx, const uint8_t *prefix, uint16_t prefix_len,
                   uint64_t nonce, uint8_t out_hash[32]);
}

static double now_mono(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

static TensorHashCtx *mode_ctx_create(int gpu_id) {
    if (cudaSetDevice(gpu_id) != cudaSuccess) {
        fprintf(stderr, "cudaSetDevice(%d) failed\n", gpu_id);
        return NULL;
    }
    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, gpu_id);
    int err = 0;
    size_t free_b = 0;
    TensorHashCtx *ctx = th_ctx_create(&err, &free_b);
    if (!ctx) {
        if (err == 1)
            fprintf(stderr,
                "GPU %d (%s): %.1f GiB free VRAM — TensorHash needs ~20 GiB. "
                "Minimum supported card: RTX 3090 / 24 GB.\n",
                gpu_id, prop.name, free_b / (1024.0 * 1024.0 * 1024.0));
        else
            fprintf(stderr, "GPU %d: context allocation failed\n", gpu_id);
        return NULL;
    }
    printf("GPU %d: %s\n", gpu_id, prop.name);
    return ctx;
}

// ── --selftest ────────────────────────────────────────────────────────────────

int run_selftest(int gpu_id) {
    printf("=== TensorHash v1 selftest ===\n");

    /* Layer 1: host reference vs hardcoded Rust KATs */
    int rc = host_tensorhash_kat_check();
    if (rc != 0) {
        fprintf(stderr, "FAIL layer 1: host KAT vector %d\n", rc);
        return 1;
    }
    printf("layer 1 PASS  host reference matches Rust KATs\n");

    TensorHashCtx *ctx = mode_ctx_create(gpu_id);
    if (!ctx) return 1;

    /* Layer 2: dataset generation + spot-check, zero seed */
    uint8_t zero_seed[32] = {0};
    printf("generating dataset (zero seed)...\n");
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        fprintf(stderr, "FAIL layer 2: dataset spot-check (zero seed)\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("layer 2 PASS  dataset spot-check (4098 elements) in %.1fs\n",
           th_last_dataset_gen_seconds(ctx));

    /* Layer 3: the Rust pow_hash KAT through the real kernel path */
    uint8_t got[32], expect[32];
    th_hex32_to_bytes(TH_KAT_POW_HEX, expect);
    if (!th_hash_one(ctx, (const uint8_t *)TH_KAT_POW_HEADER,
                     (uint16_t)strlen(TH_KAT_POW_HEADER),
                     TH_KAT_POW_NONCE, got) ||
        memcmp(got, expect, 32) != 0) {
        fprintf(stderr, "FAIL layer 3: kernel pow_hash KAT mismatch\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("layer 3 PASS  kernel reproduces the Rust pow_hash KAT\n");

    /* Layer 4a: 1024 random (prefix, nonce) GPU-vs-host, zero seed */
    uint64_t rng = 0xdeadbeefcafef00dULL;
    uint8_t prefix[102], host_hash[32];
    for (int t = 0; t < 1024; t++) {
        for (int i = 0; i < 102; i++) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            prefix[i] = (uint8_t)rng;
        }
        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
        uint64_t nonce = rng;
        if (!th_hash_one(ctx, prefix, 102, nonce, got)) {
            fprintf(stderr, "FAIL layer 4a: th_hash_one error at trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
        host_pow_hash(prefix, 102, nonce, zero_seed, host_hash);
        if (memcmp(got, host_hash, 32) != 0) {
            fprintf(stderr, "FAIL layer 4a: GPU/host mismatch at trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
    }
    printf("layer 4a PASS  1024 random attempts match host (zero seed)\n");

    /* Layer 4b: regenerate with seed=[1;32], verify the V8 vector + 16 randoms */
    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    printf("regenerating dataset (seed = [1;32])...\n");
    if (th_ctx_generate_dataset(ctx, one_seed, 1024) != 0) {
        fprintf(stderr, "FAIL layer 4b: dataset spot-check (one seed)\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    th_hex32_to_bytes("cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491", expect);
    if (!th_hash_one(ctx, xprefix, 102, 777, got) || memcmp(got, expect, 32) != 0) {
        fprintf(stderr, "FAIL layer 4b: V8 vector mismatch on kernel path\n");
        th_ctx_destroy(ctx);
        return 1;
    }
    for (int t = 0; t < 16; t++) {
        for (int i = 0; i < 102; i++) {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            prefix[i] = (uint8_t)rng;
        }
        rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
        uint64_t nonce = rng;
        th_hash_one(ctx, prefix, 102, nonce, got);
        host_pow_hash(prefix, 102, nonce, one_seed, host_hash);
        if (memcmp(got, host_hash, 32) != 0) {
            fprintf(stderr, "FAIL layer 4b: GPU/host mismatch (one seed) trial %d\n", t);
            th_ctx_destroy(ctx);
            return 1;
        }
    }
    printf("layer 4b PASS  non-zero seed: V8 + 16 random attempts match\n");

    th_ctx_destroy(ctx);
    printf("\n=== ALL SELFTEST LAYERS PASS — GPU implementation is consensus-equivalent ===\n");
    return 0;
}

// ── --benchmark ───────────────────────────────────────────────────────────────

int run_benchmark(int gpu_id, int seconds, int cuda_blocks, int cuda_threads) {
    if (seconds <= 0) seconds = 60;
    printf("=== TensorHash v1 benchmark (%ds) ===\n", seconds);

    TensorHashCtx *ctx = mode_ctx_create(gpu_id);
    if (!ctx) return 1;

    uint8_t zero_seed[32] = {0};
    printf("generating dataset...\n");
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        th_ctx_destroy(ctx);
        return 1;
    }
    printf("dataset generation: %.2fs (regenerates every %llu blocks / ~5.7 days)\n",
           th_last_dataset_gen_seconds(ctx), (unsigned long long)TH_EPOCH_LENGTH);

    /* random fake 102-byte prefix (plus 8 nonce bytes => 110 total) at
       impossible difficulty 64 so the loop never exits early */
    uint8_t header[110];
    for (int i = 0; i < 110; i++) header[i] = (uint8_t)(i * 37 + 11);

    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)cuda_blocks * (uint64_t)cuda_threads));
    if (iters < 1) iters = 1;
    uint64_t per_launch = (uint64_t)cuda_blocks * cuda_threads * iters;

    double t_start = now_mono();
    uint64_t total = 0, nonce = 0, dummy;
    while (now_mono() - t_start < (double)seconds) {
        th_launch_mining(ctx, header, 110, 64, nonce,
                         cuda_blocks, cuda_threads, iters, &dummy);
        total += per_launch;
        nonce += per_launch;
    }
    double elapsed = now_mono() - t_start;
    double mhs = total / elapsed / 1e6;
    printf("\nhashrate: %.2f MH/s  (%llu hashes in %.1fs, blocks=%d threads=%d)\n",
           mhs, (unsigned long long)total, elapsed, cuda_blocks, cuda_threads);
    printf("expected time to 42-bit genesis on this GPU alone: %.1f hours\n",
           4398046511104.0 /* 2^42 */ / (mhs * 1e6) / 3600.0);

    th_ctx_destroy(ctx);
    return 0;
}

// ── --mode genesis ────────────────────────────────────────────────────────────

typedef struct {
    int      gpu_id;
    const uint8_t *header;   /* prefix + 8 zero nonce bytes */
    int      header_len;
    int      bits;
    uint64_t nonce_start, nonce_end;
    int      blocks, threads;
} GenesisArgs;

static volatile int      g_gen_found = 0;
static volatile uint64_t g_gen_nonce = 0;
static pthread_mutex_t   g_gen_mutex = PTHREAD_MUTEX_INITIALIZER;

static void *genesis_thread(void *p) {
    GenesisArgs *a = (GenesisArgs *)p;
    TensorHashCtx *ctx = mode_ctx_create(a->gpu_id);
    if (!ctx) return NULL;

    uint8_t zero_seed[32] = {0};   /* genesis is epoch 0 */
    printf("[GPU %d] generating dataset...\n", a->gpu_id);
    if (th_ctx_generate_dataset(ctx, zero_seed, 4096) != 0) {
        th_ctx_destroy(ctx);
        return NULL;
    }

    uint32_t iters = (uint32_t)((1ULL << 24) /
        ((uint64_t)a->blocks * (uint64_t)a->threads));
    if (iters < 1) iters = 1;
    uint64_t per_launch = (uint64_t)a->blocks * a->threads * iters;

    uint64_t nonce = a->nonce_start, done = 0;
    double t0 = now_mono(), last_print = t0;

    while (!g_gen_found && nonce < a->nonce_end) {
        uint64_t found_nonce = 0;
        if (th_launch_mining(ctx, a->header, (uint16_t)a->header_len,
                             (uint8_t)a->bits, nonce,
                             a->blocks, a->threads, iters, &found_nonce)) {
            pthread_mutex_lock(&g_gen_mutex);
            if (!g_gen_found) { g_gen_found = 1; g_gen_nonce = found_nonce; }
            pthread_mutex_unlock(&g_gen_mutex);
            break;
        }
        nonce += per_launch;
        done  += per_launch;
        double now = now_mono();
        if (now - last_print >= 10.0) {
            double mhs = done / (now - t0) / 1e6;
            double expect_h = (double)(1ULL << a->bits) / (mhs * 1e6) / 3600.0;
            printf("[GPU %d] %.2f MH/s  %llu MH done  (E[total] ~ %.1f GPU-hours at %d bits)\n",
                   a->gpu_id, mhs, (unsigned long long)(done / 1000000ULL),
                   expect_h, a->bits);
            fflush(stdout);
            last_print = now;
        }
    }

    th_ctx_destroy(ctx);
    return NULL;
}

int run_genesis(const char *prefix_hex, int bits, uint64_t start_nonce,
                int gpu_count, const int *gpu_ids,
                int cuda_blocks, int cuda_threads) {
    size_t hexlen = strlen(prefix_hex);
    if (hexlen % 2 != 0 || hexlen / 2 > TH_PREFIX_MAX) {
        fprintf(stderr, "--prefix: bad hex length %zu (max %d bytes)\n",
                hexlen, TH_PREFIX_MAX);
        return 1;
    }
    int prefix_len = (int)(hexlen / 2);
    uint8_t header[HEADER_MAX] = {0};
    for (int i = 0; i < prefix_len; i++) {
        unsigned b;
        if (sscanf(prefix_hex + i * 2, "%2x", &b) != 1) {
            fprintf(stderr, "--prefix: invalid hex at byte %d\n", i);
            return 1;
        }
        header[i] = (uint8_t)b;
    }
    int header_len = prefix_len + 8;   /* trailing 8 nonce bytes, kernel-filled */

    printf("=== TensorHash v1 genesis mine ===\n");
    printf("prefix=%d bytes  bits=%d  start_nonce=%llu  gpus=%d\n",
           prefix_len, bits, (unsigned long long)start_nonce, gpu_count);
    printf("expected attempts: 2^%d ~ %.2e\n", bits, (double)(1ULL << bits));

    pthread_t   threads_arr[MAX_GPUS];
    GenesisArgs args[MAX_GPUS];
    uint64_t span = (UINT64_MAX - start_nonce) / (uint64_t)gpu_count;
    for (int i = 0; i < gpu_count; i++) {
        args[i].gpu_id      = gpu_ids[i];
        args[i].header      = header;
        args[i].header_len  = header_len;
        args[i].bits        = bits;
        args[i].nonce_start = start_nonce + (uint64_t)i * span;
        args[i].nonce_end   = (i == gpu_count - 1) ? UINT64_MAX
                                                   : start_nonce + (uint64_t)(i + 1) * span;
        args[i].blocks      = cuda_blocks;
        args[i].threads     = cuda_threads;
        pthread_create(&threads_arr[i], NULL, genesis_thread, &args[i]);
    }
    for (int i = 0; i < gpu_count; i++) pthread_join(threads_arr[i], NULL);

    if (!g_gen_found) {
        fprintf(stderr, "no nonce found (interrupted or nonce space exhausted)\n");
        return 1;
    }

    /* Host-verify before reporting. */
    uint8_t zero_seed[32] = {0}, hash[32];
    host_pow_hash(header, (uint32_t)prefix_len, g_gen_nonce, zero_seed, hash);
    int zeros = host_leading_zero_bits(hash);
    if (zeros < bits) {
        fprintf(stderr, "FOUND NONCE FAILED HOST VERIFICATION (zeros=%d < %d)\n",
                zeros, bits);
        return 1;
    }
    printf("\nGENESIS NONCE: %llu\n", (unsigned long long)g_gen_nonce);
    printf("verified on host: %d leading zero bits (need %d)\n", zeros, bits);
    printf("next: tensorium-node verify-genesis <timestamp> %llu\n",
           (unsigned long long)g_gen_nonce);
    return 0;
}
