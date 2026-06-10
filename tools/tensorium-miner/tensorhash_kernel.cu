// tools/tensorium-miner/tensorhash_kernel.cu
// Dataset generation + mining kernels and the C interface used by
// gpu_worker.cu and modes.cpp. Replaces the SHA256d mining_kernel.cu.
#include "tensorhash.cuh"
#include "host_tensorhash.h"
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* Headroom beyond the dataset for context buffers + driver overhead. */
#define TH_VRAM_HEADROOM (768ULL << 20)

__constant__ static uint8_t  c_prefix[TH_PREFIX_MAX];
__constant__ static uint32_t c_prefix_len;
__constant__ static uint8_t  c_gen_seed[32];

// ── Kernels ──────────────────────────────────────────────────────────────────

__global__ void th_dataset_gen_kernel(uint8_t *dataset) {
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint8_t buf[40];
    #pragma unroll
    for (int i = 0; i < 32; i++) buf[i] = c_gen_seed[i];

    for (uint64_t i = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
         i < TH_DATASET_N; i += stride) {
        th_le64_store_dev(buf + 32, i);
        th_blake2b256(buf, 40, dataset + i * 32ULL);
    }
}

__global__ void th_mine_kernel(const uint8_t *dataset, uint8_t difficulty_bits,
                               uint64_t start_nonce, uint32_t iters,
                               int *found, uint64_t *result_nonce) {
    if (__ldg(found)) return;

    uint64_t gid    = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint64_t nonce  = start_nonce + gid;
    uint8_t hash[32];

    for (uint32_t i = 0; i < iters; i++) {
        if (__ldg(found)) return;
        th_pow_hash_device(c_prefix, c_prefix_len, nonce, dataset, hash);
        if (th_leading_zero_bits_dev(hash) >= (int)difficulty_bits) {
            if (atomicCAS(found, 0, 1) == 0) *result_nonce = nonce;
            return;
        }
        nonce += stride;
    }
}

/* Computes the pow hash of exactly one nonce — selftest layers 3/4. */
__global__ void th_hash_one_kernel(const uint8_t *dataset, uint64_t nonce,
                                   uint8_t *hash_out) {
    if (blockIdx.x == 0 && threadIdx.x == 0)
        th_pow_hash_device(c_prefix, c_prefix_len, nonce, dataset, hash_out);
}

// ── C interface ──────────────────────────────────────────────────────────────

struct TensorHashCtx {
    uint8_t  *d_dataset;
    int      *d_found;
    uint64_t *d_result_nonce;
    uint8_t  *d_hash_out;
    uint8_t   current_seed[32];
    int       seed_valid;
    double    last_gen_seconds;
};

extern "C" {

/* Error codes for th_ctx_create. */
#define TH_ERR_NONE        0
#define TH_ERR_VRAM        1   /* not enough free VRAM (needs ~20 GB) */
#define TH_ERR_ALLOC       2   /* cudaMalloc failed */

TensorHashCtx *th_ctx_create(int *err, size_t *free_bytes_out) {
    *err = TH_ERR_NONE;
    size_t free_b = 0, total_b = 0;
    cudaMemGetInfo(&free_b, &total_b);
    if (free_bytes_out) *free_bytes_out = free_b;
    if (free_b < TH_DATASET_BYTES + TH_VRAM_HEADROOM) {
        *err = TH_ERR_VRAM;
        return NULL;
    }
    TensorHashCtx *ctx = (TensorHashCtx *)calloc(1, sizeof(TensorHashCtx));
    if (cudaMalloc(&ctx->d_dataset, TH_DATASET_BYTES) != cudaSuccess ||
        cudaMalloc(&ctx->d_found, sizeof(int)) != cudaSuccess ||
        cudaMalloc(&ctx->d_result_nonce, sizeof(uint64_t)) != cudaSuccess ||
        cudaMalloc(&ctx->d_hash_out, 32) != cudaSuccess) {
        *err = TH_ERR_ALLOC;
        if (ctx->d_dataset)      cudaFree(ctx->d_dataset);
        if (ctx->d_found)        cudaFree(ctx->d_found);
        if (ctx->d_result_nonce) cudaFree(ctx->d_result_nonce);
        if (ctx->d_hash_out)     cudaFree(ctx->d_hash_out);
        free(ctx);
        return NULL;
    }
    ctx->seed_valid = 0;
    return ctx;
}

void th_ctx_destroy(TensorHashCtx *ctx) {
    if (!ctx) return;
    cudaFree(ctx->d_dataset);
    cudaFree(ctx->d_found);
    cudaFree(ctx->d_result_nonce);
    cudaFree(ctx->d_hash_out);
    free(ctx);
}

int th_ctx_seed_matches(TensorHashCtx *ctx, const uint8_t seed[32]) {
    return ctx->seed_valid && memcmp(ctx->current_seed, seed, 32) == 0;
}

double th_last_dataset_gen_seconds(TensorHashCtx *ctx) {
    return ctx->last_gen_seconds;
}

/* Generates the full dataset for `seed`, then spot-checks element 0,
   element N-1 and `spot_count` deterministic pseudo-random indices against
   the host reference (selftest layer 2 — runs on EVERY generation).
   Returns 0 on success, -1 on CUDA error, index+1 of a mismatching spot
   check otherwise. */
int th_ctx_generate_dataset(TensorHashCtx *ctx, const uint8_t seed[32],
                            int spot_count) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    cudaMemcpyToSymbol(c_gen_seed, seed, 32);
    th_dataset_gen_kernel<<<4096, 256>>>(ctx->d_dataset);
    if (cudaDeviceSynchronize() != cudaSuccess) return -1;

    clock_gettime(CLOCK_MONOTONIC, &t1);
    ctx->last_gen_seconds =
        (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) * 1e-9;

    /* spot checks: fixed boundary indices + xorshift64 sequence (fixed seed
       => deterministic, host and device check identical indices) */
    uint64_t rng = 0x9e3779b97f4a7c15ULL;
    for (int s = 0; s < spot_count + 2; s++) {
        uint64_t idx;
        if (s == 0)      idx = 0;
        else if (s == 1) idx = TH_DATASET_N - 1;
        else {
            rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
            idx = rng % TH_DATASET_N;
        }
        uint8_t got[32], expect[32];
        if (cudaMemcpy(got, ctx->d_dataset + idx * 32ULL, 32,
                       cudaMemcpyDeviceToHost) != cudaSuccess) return -1;
        host_dataset_element(seed, idx, expect);
        if (memcmp(got, expect, 32) != 0) {
            fprintf(stderr,
                "[tensorhash] DATASET SPOT-CHECK FAILED at index %llu — "
                "GPU output does not match reference. Refusing to mine.\n",
                (unsigned long long)idx);
            return s + 1;
        }
    }

    memcpy(ctx->current_seed, seed, 32);
    ctx->seed_valid = 1;
    return 0;
}

/* header_template = full header bytes INCLUDING the trailing 8 nonce bytes
   (same convention as the old SHA256d kernel); the prefix is everything but
   those 8 bytes. Returns 1 + *nonce_out when a nonce meeting
   difficulty_bits is found. */
int th_launch_mining(TensorHashCtx *ctx, const uint8_t *header_template,
                     uint16_t header_len, uint8_t difficulty_bits,
                     uint64_t start_nonce, int blocks, int threads,
                     uint32_t iters_per_thread, uint64_t *nonce_out) {
    if (!ctx->seed_valid || header_len <= 8 ||
        (uint32_t)(header_len - 8) > TH_PREFIX_MAX) return 0;

    uint32_t prefix_len = (uint32_t)header_len - 8;
    cudaMemcpyToSymbol(c_prefix, header_template, prefix_len);
    cudaMemcpyToSymbol(c_prefix_len, &prefix_len, sizeof(uint32_t));

    int      h_found = 0;
    uint64_t h_nonce = UINT64_MAX;
    cudaMemcpy(ctx->d_found, &h_found, sizeof(int), cudaMemcpyHostToDevice);
    cudaMemcpy(ctx->d_result_nonce, &h_nonce, sizeof(uint64_t), cudaMemcpyHostToDevice);

    th_mine_kernel<<<blocks, threads>>>(ctx->d_dataset, difficulty_bits,
                                        start_nonce, iters_per_thread,
                                        ctx->d_found, ctx->d_result_nonce);
    cudaDeviceSynchronize();

    cudaMemcpy(&h_found, ctx->d_found, sizeof(int), cudaMemcpyDeviceToHost);
    cudaMemcpy(&h_nonce, ctx->d_result_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);
    if (h_found) { *nonce_out = h_nonce; return 1; }
    return 0;
}

/* Computes the pow hash of a single (prefix, nonce) through the REAL device
   code path. prefix here EXCLUDES nonce bytes. Returns 1 on success. */
int th_hash_one(TensorHashCtx *ctx, const uint8_t *prefix, uint16_t prefix_len,
                uint64_t nonce, uint8_t out_hash[32]) {
    if (!ctx->seed_valid || prefix_len > TH_PREFIX_MAX) return 0;
    uint32_t plen = prefix_len;
    cudaMemcpyToSymbol(c_prefix, prefix, plen);
    cudaMemcpyToSymbol(c_prefix_len, &plen, sizeof(uint32_t));
    th_hash_one_kernel<<<1, 1>>>(ctx->d_dataset, nonce, ctx->d_hash_out);
    if (cudaDeviceSynchronize() != cudaSuccess) return 0;
    cudaMemcpy(out_hash, ctx->d_hash_out, 32, cudaMemcpyDeviceToHost);
    return 1;
}

} // extern "C"
