// sha256d.cuh — SHA256d device helpers for Tensorium CUDA miner
// Supports the exact serialized Tensorium block header length, including
// mainnet chain IDs that do not fit the old 112-byte fixed-header assumption.

#pragma once
#include <stdint.h>

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

#define ROTR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
#define CH(e,f,g)   (((e) & (f)) ^ (~(e) & (g)))
#define MAJ(a,b,c)  (((a) & (b)) ^ ((a) & (c)) ^ ((b) & (c)))
#define SIGMA0(a)   (ROTR32(a, 2)  ^ ROTR32(a, 13) ^ ROTR32(a, 22))
#define SIGMA1(e)   (ROTR32(e, 6)  ^ ROTR32(e, 11) ^ ROTR32(e, 25))
#define sigma0(x)   (ROTR32(x, 7)  ^ ROTR32(x, 18) ^ ((x) >> 3))
#define sigma1(x)   (ROTR32(x, 17) ^ ROTR32(x, 19) ^ ((x) >> 10))

__device__ __forceinline__ uint32_t load_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] << 8)  |  (uint32_t)p[3];
}

__device__ __forceinline__ void store_be32(uint8_t *p, uint32_t v) {
    p[0] = (v >> 24) & 0xff;
    p[1] = (v >> 16) & 0xff;
    p[2] = (v >>  8) & 0xff;
    p[3] =  v        & 0xff;
}

// Optimised sha256_compress using a 16-element sliding W window instead of
// W[64]. Reduces register pressure by 75% (64 bytes vs 256 bytes for W),
// allowing ~3× higher thread occupancy on modern NVIDIA SMs.
__device__ __forceinline__ void sha256_compress(uint32_t state[8], const uint32_t block[16]) {
    uint32_t W[16];
    #pragma unroll
    for (int i = 0; i < 16; i++) W[i] = block[i];

    uint32_t a = state[0], b = state[1], c = state[2], d = state[3];
    uint32_t e = state[4], f = state[5], g = state[6], h = state[7];

    // Rounds 0-15: W already loaded
    #define STEP(i) { \
        uint32_t T1 = h + SIGMA1(e) + CH(e,f,g) + K[i] + W[(i)&15]; \
        uint32_t T2 = SIGMA0(a) + MAJ(a,b,c); \
        h=g; g=f; f=e; e=d+T1; d=c; c=b; b=a; a=T1+T2; }

    // Rounds 16-63: update W in-place (sliding window)
    #define STEPW(i) { \
        W[(i)&15] = sigma1(W[((i)-2)&15]) + W[((i)-7)&15] \
                  + sigma0(W[((i)-15)&15]) + W[((i)-16)&15]; \
        STEP(i) }

    STEP(0)  STEP(1)  STEP(2)  STEP(3)
    STEP(4)  STEP(5)  STEP(6)  STEP(7)
    STEP(8)  STEP(9)  STEP(10) STEP(11)
    STEP(12) STEP(13) STEP(14) STEP(15)
    STEPW(16) STEPW(17) STEPW(18) STEPW(19)
    STEPW(20) STEPW(21) STEPW(22) STEPW(23)
    STEPW(24) STEPW(25) STEPW(26) STEPW(27)
    STEPW(28) STEPW(29) STEPW(30) STEPW(31)
    STEPW(32) STEPW(33) STEPW(34) STEPW(35)
    STEPW(36) STEPW(37) STEPW(38) STEPW(39)
    STEPW(40) STEPW(41) STEPW(42) STEPW(43)
    STEPW(44) STEPW(45) STEPW(46) STEPW(47)
    STEPW(48) STEPW(49) STEPW(50) STEPW(51)
    STEPW(52) STEPW(53) STEPW(54) STEPW(55)
    STEPW(56) STEPW(57) STEPW(58) STEPW(59)
    STEPW(60) STEPW(61) STEPW(62) STEPW(63)

    #undef STEP
    #undef STEPW

    state[0] += a; state[1] += b; state[2] += c; state[3] += d;
    state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}

__device__ void sha256_bytes(const uint8_t *msg, uint16_t len, uint8_t hash_out[32]) {
    uint32_t state[8] = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };

    const int total_blocks = (len + 9 + 63) / 64;
    const uint64_t bit_len = (uint64_t)len * 8;

    for (int b = 0; b < total_blocks; b++) {
        uint8_t block_bytes[64];
        uint32_t block_words[16];
        int base = b * 64;

        for (int i = 0; i < 64; i++) {
            int idx = base + i;
            if (idx < len) block_bytes[i] = msg[idx];
            else if (idx == len) block_bytes[i] = 0x80;
            else block_bytes[i] = 0x00;
        }

        if (b == total_blocks - 1) {
            block_bytes[56] = (uint8_t)(bit_len >> 56);
            block_bytes[57] = (uint8_t)(bit_len >> 48);
            block_bytes[58] = (uint8_t)(bit_len >> 40);
            block_bytes[59] = (uint8_t)(bit_len >> 32);
            block_bytes[60] = (uint8_t)(bit_len >> 24);
            block_bytes[61] = (uint8_t)(bit_len >> 16);
            block_bytes[62] = (uint8_t)(bit_len >> 8);
            block_bytes[63] = (uint8_t)(bit_len);
        }

        for (int i = 0; i < 16; i++) {
            block_words[i] = load_be32(block_bytes + i * 4);
        }
        sha256_compress(state, block_words);
    }

    for (int i = 0; i < 8; i++) {
        store_be32(hash_out + i * 4, state[i]);
    }
}

__device__ void sha256d_bytes(const uint8_t *msg, uint16_t len, uint8_t hash_out[32]) {
    uint8_t first_hash[32];
    sha256_bytes(msg, len, first_hash);
    sha256_bytes(first_hash, 32, hash_out);
}

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
