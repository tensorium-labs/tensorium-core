// tools/tensorium-miner/blake2b.cuh
// Blake2b-256 (RFC 7693, sequential, unkeyed) — single source compiled for
// both host (g++) and device (nvcc). CONSENSUS-CRITICAL: must match the
// `blake2` Rust crate used by crates/tensorium-tensorhash bit-for-bit;
// pinned by the KAT vectors in test_host_tensorhash.cpp and --selftest.
#pragma once
#include <stdint.h>

#ifdef __CUDACC__
#define TH_HD __host__ __device__ __forceinline__
#else
#define TH_HD static inline
#endif

#define TH_B2B_IV_INIT { \
    0x6a09e667f3bcc908ULL, 0xbb67ae8584caa73bULL, \
    0x3c6ef372fe94f82bULL, 0xa54ff53a5f1d36f1ULL, \
    0x510e527fade682d1ULL, 0x9b05688c2b3e6c1fULL, \
    0x1f83d9abfb41bd6bULL, 0x5be0cd19137e2179ULL }

#define TH_B2B_SIGMA_INIT { \
    { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15}, \
    {14,10, 4, 8, 9,15,13, 6, 1,12, 0, 2,11, 7, 5, 3}, \
    {11, 8,12, 0, 5, 2,15,13,10,14, 3, 6, 7, 1, 9, 4}, \
    { 7, 9, 3, 1,13,12,11,14, 2, 6, 5,10, 4, 0,15, 8}, \
    { 9, 0, 5, 7, 2, 4,10,15,14, 1,11,12, 6, 8, 3,13}, \
    { 2,12, 6,10, 0,11, 8, 3, 4,13, 7, 5,15,14, 1, 9}, \
    {12, 5, 1,15,14,13, 4,10, 0, 7, 6, 3, 9, 2, 8,11}, \
    {13,11, 7,14,12, 1, 3, 9, 5, 0,15, 4, 8, 6, 2,10}, \
    { 6,15,14, 9,11, 3, 0, 8,12, 2,13, 7, 1, 4,10, 5}, \
    {10, 2, 8, 4, 7, 6, 1, 5,15,11, 9,14, 3,12,13, 0}, \
    { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,15}, \
    {14,10, 4, 8, 9,15,13, 6, 1,12, 0, 2,11, 7, 5, 3} }

/* Two physical copies so the device pass reads constant memory while host
   code (same translation unit) reads ordinary statics. The literal tables
   come from one macro each, so they cannot diverge. */
#ifdef __CUDACC__
__constant__ static const uint64_t TH_B2B_IV_D[8]        = TH_B2B_IV_INIT;
__constant__ static const uint8_t  TH_B2B_SIGMA_D[12][16] = TH_B2B_SIGMA_INIT;
#endif
static const uint64_t TH_B2B_IV_H[8]        = TH_B2B_IV_INIT;
static const uint8_t  TH_B2B_SIGMA_H[12][16] = TH_B2B_SIGMA_INIT;

#ifdef __CUDA_ARCH__
#define TH_B2B_IV    TH_B2B_IV_D
#define TH_B2B_SIGMA TH_B2B_SIGMA_D
#else
#define TH_B2B_IV    TH_B2B_IV_H
#define TH_B2B_SIGMA TH_B2B_SIGMA_H
#endif

TH_HD uint64_t th_rotr64(uint64_t x, int n) { return (x >> n) | (x << (64 - n)); }

#define TH_G(v, a, b, c, d, x, y) do { \
    v[a] = v[a] + v[b] + (x); v[d] = th_rotr64(v[d] ^ v[a], 32); \
    v[c] = v[c] + v[d];       v[b] = th_rotr64(v[b] ^ v[c], 24); \
    v[a] = v[a] + v[b] + (y); v[d] = th_rotr64(v[d] ^ v[a], 16); \
    v[c] = v[c] + v[d];       v[b] = th_rotr64(v[b] ^ v[c], 63); \
} while (0)

TH_HD void th_blake2b_compress(uint64_t h[8], const uint8_t block[128],
                               uint64_t t, int last) {
    uint64_t m[16];
    for (int i = 0; i < 16; i++) {
        uint64_t w = 0;
        for (int k = 7; k >= 0; k--) w = (w << 8) | block[i * 8 + k];
        m[i] = w;  /* little-endian load */
    }
    uint64_t v[16];
    for (int i = 0; i < 8; i++) v[i] = h[i];
    for (int i = 0; i < 8; i++) v[8 + i] = TH_B2B_IV[i];
    v[12] ^= t;                 /* t_hi is always 0 for our input sizes */
    if (last) v[14] = ~v[14];
    for (int r = 0; r < 12; r++) {
        const uint8_t *s = TH_B2B_SIGMA[r];
        TH_G(v, 0, 4,  8, 12, m[s[0]],  m[s[1]]);
        TH_G(v, 1, 5,  9, 13, m[s[2]],  m[s[3]]);
        TH_G(v, 2, 6, 10, 14, m[s[4]],  m[s[5]]);
        TH_G(v, 3, 7, 11, 15, m[s[6]],  m[s[7]]);
        TH_G(v, 0, 5, 10, 15, m[s[8]],  m[s[9]]);
        TH_G(v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        TH_G(v, 2, 7,  8, 13, m[s[12]], m[s[13]]);
        TH_G(v, 3, 4,  9, 14, m[s[14]], m[s[15]]);
    }
    for (int i = 0; i < 8; i++) h[i] ^= v[i] ^ v[8 + i];
}

/* One-shot Blake2b-256 of `len` bytes (len <= a few hundred in TensorHash). */
TH_HD void th_blake2b256(const uint8_t *data, uint32_t len, uint8_t out[32]) {
    uint64_t h[8] = TH_B2B_IV_INIT;
    h[0] ^= 0x01010000ULL ^ 32ULL;  /* digest_length=32, fanout=1, depth=1 */

    uint32_t off = 0;
    while (len - off > 128) {       /* full non-final blocks */
        th_blake2b_compress(h, data + off, (uint64_t)off + 128, 0);
        off += 128;
    }
    uint8_t block[128];
    uint32_t rem = len - off;       /* 0..128 — final block, zero-padded */
    for (uint32_t i = 0; i < 128; i++) block[i] = (i < rem) ? data[off + i] : 0;
    th_blake2b_compress(h, block, (uint64_t)len, 1);

    for (int i = 0; i < 32; i++) out[i] = (uint8_t)(h[i / 8] >> (8 * (i % 8)));
}
