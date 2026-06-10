// tools/tensorium-miner/tensorhash.cuh
// Device-side TensorHash v1 — mirrors host_tensorhash.cpp / the Rust
// reference exactly. Dataset element loads come from the VRAM dataset
// instead of being recomputed.
#pragma once
#include "blake2b.cuh"
#include "tensorhash_params.h"

__device__ __forceinline__ void th_le64_store_dev(uint8_t *b, uint64_t v) {
    #pragma unroll
    for (int i = 0; i < 8; i++) { b[i] = (uint8_t)v; v >>= 8; }
}

__device__ __forceinline__ uint64_t th_le64_load_dev(const uint8_t *b) {
    uint64_t v = 0;
    #pragma unroll
    for (int i = 7; i >= 0; i--) v = (v << 8) | b[i];
    return v;
}

__device__ __forceinline__ uint64_t th_rotl64_dev(uint64_t x, int n) {
    return (x << n) | (x >> (64 - n));
}

/* Full TensorHash v1 pow hash for one (prefix, nonce) attempt.
   prefix points at the nonce-less header bytes (constant or global mem),
   dataset is the 19.2 GB VRAM element table (32-byte aligned rows). */
__device__ void th_pow_hash_device(const uint8_t *prefix, uint32_t prefix_len,
                                   uint64_t nonce, const uint8_t *dataset,
                                   uint8_t out[32]) {
    uint8_t buf[TH_PREFIX_MAX + 8 + 32];
    for (uint32_t i = 0; i < prefix_len; i++) buf[i] = prefix[i];
    th_le64_store_dev(buf + prefix_len, nonce);

    uint8_t digest[32];
    th_blake2b256(buf, prefix_len + 8, digest);

    uint64_t acc[4];
    #pragma unroll
    for (int m = 0; m < 4; m++) acc[m] = th_le64_load_dev(digest + m * 8);

    uint8_t ibuf[40], iseed[32];
    #pragma unroll
    for (int i = 0; i < 32; i++) ibuf[i] = digest[i];

    for (uint64_t j = 0; j < TH_K; j++) {
        th_le64_store_dev(ibuf + 32, j);
        th_blake2b256(ibuf, 40, iseed);
        uint64_t idx = th_le64_load_dev(iseed) % TH_DATASET_N;

        /* rows are 32-byte aligned; little-endian arch => direct u64 loads
           match from_le_bytes. __ldg routes through the read-only cache. */
        const uint64_t *e = (const uint64_t *)(dataset + idx * 32ULL);
        uint64_t elem[4];
        elem[0] = __ldg(e + 0); elem[1] = __ldg(e + 1);
        elem[2] = __ldg(e + 2); elem[3] = __ldg(e + 3);

        uint64_t next[4];
        #pragma unroll
        for (int m = 0; m < 4; m++)
            next[m] = acc[m] * (elem[m] | 1ULL)
                    + th_rotl64_dev(elem[(m + 1) & 3], 13);
        #pragma unroll
        for (int m = 0; m < 4; m++) acc[m] = next[m];
    }

    #pragma unroll
    for (int m = 0; m < 4; m++) th_le64_store_dev(buf + prefix_len + 8 + m * 8, acc[m]);
    th_blake2b256(buf, prefix_len + 8 + 32, out);
}

__device__ __forceinline__ int th_leading_zero_bits_dev(const uint8_t h[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (h[i] == 0) { bits += 8; continue; }
        bits += __clz((unsigned)h[i]) - 24;
        break;
    }
    return bits;
}
