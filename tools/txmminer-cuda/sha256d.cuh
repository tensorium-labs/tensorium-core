// sha256d.cuh — SHA256d device functions for Tensorium CUDA miner
// SHA256 implementation optimized for the 112-byte Tensorium block header.
//
// Header layout (112 bytes):
//   [0..3]   version      (u32 LE)
//   [4..22]  chain_id     ("tensorium-testnet-0", 19 bytes)
//   [23..30] height       (u64 LE)
//   [31..62] previous_hash (32 bytes)
//   [63..94] merkle_root   (32 bytes)
//   [95..102] timestamp    (u64 LE)
//   [103]    difficulty_bits (u8)
//   [104..111] nonce       (u64 LE)  ← varied per thread
//
// Midstate optimisation: block1 = header[0..63] is constant for all nonces.
// CPU precomputes the SHA256 state after block1, GPU only processes block2.

#pragma once
#include <stdint.h>

// ── SHA256 constants ─────────────────────────────────────────────────────────

__constant__ uint32_t K[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,
    0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,
    0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,
    0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,
    0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,
    0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,
    0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,
    0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,
    0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

#define ROTR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
#define CH(e,f,g)   (((e) & (f)) ^ (~(e) & (g)))
#define MAJ(a,b,c)  (((a) & (b)) ^ ((a) & (c)) ^ ((b) & (c)))
#define SIGMA0(a)   (ROTR32(a, 2)  ^ ROTR32(a, 13) ^ ROTR32(a, 22))
#define SIGMA1(e)   (ROTR32(e, 6)  ^ ROTR32(e, 11) ^ ROTR32(e, 25))
#define sigma0(x)   (ROTR32(x, 7)  ^ ROTR32(x, 18) ^ ((x) >> 3))
#define sigma1(x)   (ROTR32(x, 17) ^ ROTR32(x, 19) ^ ((x) >> 10))

// Big-endian u32 load from byte array
__device__ __forceinline__ uint32_t load_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] << 8)  |  (uint32_t)p[3];
}

// Store big-endian u32 to byte array
__device__ __forceinline__ void store_be32(uint8_t *p, uint32_t v) {
    p[0] = (v >> 24) & 0xff;
    p[1] = (v >> 16) & 0xff;
    p[2] = (v >>  8) & 0xff;
    p[3] =  v        & 0xff;
}

// ── SHA256 single block compression ─────────────────────────────────────────
// state[8]: in/out (H0..H7)
// block[64]: input block (big-endian 32-bit words)

__device__ void sha256_compress(uint32_t state[8], const uint32_t block[16]) {
    uint32_t W[64];
    for (int i = 0; i < 16; i++) W[i] = block[i];
    for (int i = 16; i < 64; i++)
        W[i] = sigma1(W[i-2]) + W[i-7] + sigma0(W[i-15]) + W[i-16];

    uint32_t a = state[0], b = state[1], c = state[2], d = state[3];
    uint32_t e = state[4], f = state[5], g = state[6], h = state[7];

    for (int i = 0; i < 64; i++) {
        uint32_t T1 = h + SIGMA1(e) + CH(e,f,g) + K[i] + W[i];
        uint32_t T2 = SIGMA0(a) + MAJ(a,b,c);
        h = g; g = f; f = e; e = d + T1;
        d = c; c = b; b = a; a = T1 + T2;
    }

    state[0] += a; state[1] += b; state[2] += c; state[3] += d;
    state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}

// ── SHA256d of 112-byte Tensorium header ─────────────────────────────────────
//
// Uses precomputed midstate after block1 (first 64 bytes).
// block2 is header[64..111] (48 bytes) + SHA256 padding.
//
// block2 bytes:
//   [0..47]  = header[64..111]
//   [48]     = 0x80
//   [49..55] = 0x00 (padding)
//   [56..63] = 0x0000000000000380  (big-endian bit length = 896)
//
// nonce_offset in block2 = 104 - 64 = 40  →  block2[40..47]

__device__ void sha256d_header(
    const uint32_t midstate[8],  // SHA256 state after block1
    const uint8_t  block2[64],   // second 64-byte block (pre-built with nonce)
    uint8_t        hash_out[32]  // SHA256d result
) {
    // ── First hash: finish block2 ─────────────────────────────────────────
    uint32_t state1[8];
    for (int i = 0; i < 8; i++) state1[i] = midstate[i];

    uint32_t W1[16];
    for (int i = 0; i < 16; i++)
        W1[i] = load_be32(block2 + i * 4);
    sha256_compress(state1, W1);

    // state1 now holds SHA256(header)

    // ── Second hash: SHA256 of 32-byte first hash ─────────────────────────
    // Input is 32 bytes. SHA256 block (64 bytes):
    //   [0..3]  state1[0], ..., state1[7]  in BE
    //   [32]    0x80
    //   [33..55] 0x00
    //   [56..63] 0x0000000000000100  (256 bits)

    uint32_t W2[16] = {0};
    W2[0] = state1[0]; W2[1] = state1[1]; W2[2] = state1[2]; W2[3] = state1[3];
    W2[4] = state1[4]; W2[5] = state1[5]; W2[6] = state1[6]; W2[7] = state1[7];
    W2[8] = 0x80000000;  // 0x80 followed by zeros
    // W2[9..13] = 0
    W2[14] = 0x00000000;
    W2[15] = 0x00000100;  // 256 bits in big-endian u32 pair [0, 256]

    uint32_t state2[8] = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };
    sha256_compress(state2, W2);

    for (int i = 0; i < 8; i++)
        store_be32(hash_out + i * 4, state2[i]);
}

// ── Leading zero bit count of a 32-byte hash ─────────────────────────────────
__device__ int leading_zero_bits(const uint8_t hash[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (hash[i] == 0) {
            bits += 8;
        } else {
            bits += __clz((uint32_t)hash[i] << 24);
            break;
        }
    }
    return bits;
}
