// mining_kernel.cu — CUDA SHA256d mining kernel for Tensorium
// Called from main.cu via launch_mining_kernel()

#include "sha256d.cuh"
#include <stdint.h>

// ── Constant memory — updated each new block template via cudaMemcpyToSymbol ─
// Broadcast from L1 constant cache → all 32 threads in a warp share one read.
__constant__ uint32_t c_midstate[8];
__constant__ uint8_t  c_block2const[50];

static __host__ __device__ __forceinline__ uint32_t rotr32_hostdev(uint32_t x, int n) {
    return (x >> n) | (x << (32 - n));
}

static void compute_midstate_64(const uint8_t *header, uint32_t midstate_out[8]) {
    midstate_out[0] = 0x6a09e667;
    midstate_out[1] = 0xbb67ae85;
    midstate_out[2] = 0x3c6ef372;
    midstate_out[3] = 0xa54ff53a;
    midstate_out[4] = 0x510e527f;
    midstate_out[5] = 0x9b05688c;
    midstate_out[6] = 0x1f83d9ab;
    midstate_out[7] = 0x5be0cd19;

    uint32_t W[64];
    static const uint32_t Ks[64] = {
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

    for (int i = 0; i < 16; i++) {
        const uint8_t *p = header + i * 4;
        W[i] = ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
               ((uint32_t)p[2] << 8)  |  (uint32_t)p[3];
    }
    for (int i = 16; i < 64; i++) {
        uint32_t x = W[i - 15];
        uint32_t y = W[i - 2];
        uint32_t s0 = rotr32_hostdev(x, 7) ^ rotr32_hostdev(x, 18) ^ (x >> 3);
        uint32_t s1 = rotr32_hostdev(y, 17) ^ rotr32_hostdev(y, 19) ^ (y >> 10);
        W[i] = W[i - 16] + s0 + W[i - 7] + s1;
    }

    uint32_t a = midstate_out[0], b = midstate_out[1], c = midstate_out[2], d = midstate_out[3];
    uint32_t e = midstate_out[4], f = midstate_out[5], g = midstate_out[6], h = midstate_out[7];

    for (int i = 0; i < 64; i++) {
        uint32_t S1 = rotr32_hostdev(e, 6) ^ rotr32_hostdev(e, 11) ^ rotr32_hostdev(e, 25);
        uint32_t ch = (e & f) ^ (~e & g);
        uint32_t T1 = h + S1 + ch + Ks[i] + W[i];
        uint32_t S0 = rotr32_hostdev(a, 2) ^ rotr32_hostdev(a, 13) ^ rotr32_hostdev(a, 22);
        uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint32_t T2 = S0 + maj;
        h = g; g = f; f = e; e = d + T1;
        d = c; c = b; b = a; a = T1 + T2;
    }

    midstate_out[0] += a; midstate_out[1] += b; midstate_out[2] += c; midstate_out[3] += d;
    midstate_out[4] += e; midstate_out[5] += f; midstate_out[6] += g; midstate_out[7] += h;
}

// sha256d_122 — optimised for 122-byte Tensorium headers.
// Reads midstate and block2_const from __constant__ memory (L1 broadcast).
// Returns hash result as uint32[8] big-endian — caller checks difficulty
// directly on s2[] without converting to bytes.
//
// SHA256 padding layout (122-byte message):
//   block1 (64 B): pre-computed as midstate on CPU
//   block2 (64 B): header[64..121] || 0x80 || 0x00×5     (bit-len in block3)
//   block3 (64 B): 0x00×56 || 0x000003D0                  (976 bits)
//   block4 (64 B): inner_hash[0..31] || 0x80 || 0x00×23 || 0x00000100 (256 bits)
__device__ __forceinline__ void sha256d_122_u32(uint64_t nonce, uint32_t s2_out[8]) {
    // ── block2: 12 full words from c_block2const, then nonce spliced in ──────
    uint32_t W2[16];
    #pragma unroll
    for (int i = 0; i < 12; i++) W2[i] = load_be32(c_block2const + i * 4);

    W2[12] = ((uint32_t)c_block2const[48] << 24) | ((uint32_t)c_block2const[49] << 16)
           | ((uint32_t)(nonce & 0xff) << 8) | (uint32_t)((nonce >> 8) & 0xff);
    W2[13] = ((uint32_t)((nonce >> 16) & 0xff) << 24)
           | ((uint32_t)((nonce >> 24) & 0xff) << 16)
           | ((uint32_t)((nonce >> 32) & 0xff) << 8)
           |  (uint32_t)((nonce >> 40) & 0xff);
    W2[14] = ((uint32_t)((nonce >> 48) & 0xff) << 24)
           | ((uint32_t)((nonce >> 56) & 0xff) << 16)
           | 0x00008000u;
    W2[15] = 0x00000000u;

    // ── inner SHA256: block2 + block3 ────────────────────────────────────────
    uint32_t s[8];
    #pragma unroll
    for (int i = 0; i < 8; i++) s[i] = c_midstate[i];
    sha256_compress(s, W2);

    uint32_t W3[16] = {0};
    W3[15] = 0x000003D0u;  // 976 bits
    sha256_compress(s, W3);

    // ── outer SHA256: block4 from inner hash ─────────────────────────────────
    uint32_t W4[16];
    #pragma unroll
    for (int i = 0; i < 8; i++) W4[i] = s[i];
    W4[8] = 0x80000000u;
    W4[9] = 0; W4[10] = 0; W4[11] = 0;
    W4[12] = 0; W4[13] = 0; W4[14] = 0;
    W4[15] = 0x00000100u;  // 256 bits

    s2_out[0] = 0x6a09e667u; s2_out[1] = 0xbb67ae85u;
    s2_out[2] = 0x3c6ef372u; s2_out[3] = 0xa54ff53au;
    s2_out[4] = 0x510e527fu; s2_out[5] = 0x9b05688cu;
    s2_out[6] = 0x1f83d9abu; s2_out[7] = 0x5be0cd19u;
    sha256_compress(s2_out, W4);
}

// Direct uint32 difficulty check — avoids byte-store + byte-loop.
// s2 is big-endian: s2[0]=bytes[0..3], s2[1]=bytes[4..7], etc.
__device__ __forceinline__ bool passes_difficulty(const uint32_t s2[8], uint8_t bits) {
    // Full 32-bit words that must be zero
    uint8_t full = bits >> 5;
    uint8_t rem  = bits & 31;
    #pragma unroll 8
    for (int i = 0; i < 8; i++) {
        if (i < full && s2[i] != 0) return false;
    }
    if (rem == 0) return true;
    return (full < 8) && ((s2[full] >> (32u - rem)) == 0);
}

__global__ void mine_kernel_122(
    uint8_t   difficulty_bits,
    uint64_t  start_nonce,
    uint32_t  iters,
    int      *found,
    uint64_t *result_nonce
) {
    if (__ldg(found)) return;

    uint64_t gid    = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)gridDim.x  * blockDim.x;
    uint64_t nonce  = start_nonce + gid;
    uint32_t s2[8];

    for (uint32_t i = 0; i < iters; i++) {
        if (__ldg(found)) return;
        sha256d_122_u32(nonce, s2);
        if (passes_difficulty(s2, difficulty_bits)) {
            if (atomicCAS(found, 0, 1) == 0) *result_nonce = nonce;
            return;
        }
        nonce += stride;
    }
}

__global__ void mine_kernel_generic(
    const uint8_t *header_template,
    uint16_t       header_len,
    uint8_t        difficulty_bits,
    uint64_t       start_nonce,
    uint32_t       iters,
    int           *found,
    uint64_t      *result_nonce
) {
    if (*found) return;

    uint32_t gid = blockIdx.x * blockDim.x + threadIdx.x;
    uint32_t stride = gridDim.x * blockDim.x;
    uint64_t nonce = start_nonce + (uint64_t)gid;

    uint8_t header[192];
    uint8_t hash[32];
    const int nonce_off = (int)header_len - 8;

    for (int i = 0; i < (int)header_len; i++) header[i] = header_template[i];

    for (uint32_t i = 0; i < iters; i++) {
        if (*found) return;

        header[nonce_off + 0] = (uint8_t)(nonce);
        header[nonce_off + 1] = (uint8_t)(nonce >>  8);
        header[nonce_off + 2] = (uint8_t)(nonce >> 16);
        header[nonce_off + 3] = (uint8_t)(nonce >> 24);
        header[nonce_off + 4] = (uint8_t)(nonce >> 32);
        header[nonce_off + 5] = (uint8_t)(nonce >> 40);
        header[nonce_off + 6] = (uint8_t)(nonce >> 48);
        header[nonce_off + 7] = (uint8_t)(nonce >> 56);

        sha256d_bytes(header, header_len, hash);

        if (leading_zero_bits(hash) >= (int)difficulty_bits) {
            if (atomicCAS(found, 0, 1) == 0) {
                *result_nonce = nonce;
            }
            return;
        }

        nonce += (uint64_t)stride;
    }
}

// ── Pre-allocated mining context ─────────────────────────────────────────────
// Allocate GPU buffers once, reuse every kernel launch to eliminate
// cudaMalloc/cudaFree overhead (~16ms per launch → ~0.1ms per launch).

struct MiningCtx {
    uint32_t *d_midstate;
    uint8_t  *d_block2_const;
    uint8_t  *d_header;
    int      *d_found;
    uint64_t *d_result_nonce;
    uint16_t  header_len;
};

extern "C" {

MiningCtx *mining_ctx_create(uint16_t header_len) {
    MiningCtx *ctx = (MiningCtx *)malloc(sizeof(MiningCtx));
    ctx->header_len      = header_len;
    ctx->d_midstate      = nullptr;  // unused for 122-byte path (now __constant__)
    ctx->d_block2_const  = nullptr;  // unused for 122-byte path (now __constant__)
    ctx->d_header        = nullptr;

    cudaMalloc(&ctx->d_found,        sizeof(int));
    cudaMalloc(&ctx->d_result_nonce, sizeof(uint64_t));

    if (header_len != 122) {
        cudaMalloc(&ctx->d_header, header_len);
    }
    return ctx;
}

uint16_t mining_ctx_header_len(MiningCtx *ctx) { return ctx->header_len; }

void mining_ctx_destroy(MiningCtx *ctx) {
    if (!ctx) return;
    if (ctx->d_midstate)     cudaFree(ctx->d_midstate);
    if (ctx->d_block2_const) cudaFree(ctx->d_block2_const);
    if (ctx->d_header)       cudaFree(ctx->d_header);
    cudaFree(ctx->d_found);
    cudaFree(ctx->d_result_nonce);
    free(ctx);
}

int launch_mining_kernel_ctx(
    MiningCtx      *ctx,
    const uint8_t  *header_template,
    uint8_t         difficulty_bits,
    uint64_t        start_nonce,
    int             blocks,
    int             threads,
    uint32_t        iters_per_thread,
    uint64_t       *nonce_out
) {
    int      h_found = 0;
    uint64_t h_nonce = UINT64_MAX;
    cudaMemcpy(ctx->d_found,        &h_found, sizeof(int),      cudaMemcpyHostToDevice);
    cudaMemcpy(ctx->d_result_nonce, &h_nonce, sizeof(uint64_t), cudaMemcpyHostToDevice);

    if (ctx->header_len == 122) {
        uint32_t midstate[8];
        uint8_t  block2_const[50];
        compute_midstate_64(header_template, midstate);
        for (int i = 0; i < 50; i++) block2_const[i] = header_template[64 + i];

        // Upload to __constant__ memory — L1 broadcast for all warps
        cudaMemcpyToSymbol(c_midstate,    midstate,     8 * sizeof(uint32_t));
        cudaMemcpyToSymbol(c_block2const, block2_const, 50);

        mine_kernel_122<<<blocks, threads>>>(
            difficulty_bits, start_nonce, iters_per_thread,
            ctx->d_found, ctx->d_result_nonce
        );
    } else {
        cudaMemcpy(ctx->d_header, header_template, ctx->header_len, cudaMemcpyHostToDevice);
        mine_kernel_generic<<<blocks, threads>>>(
            ctx->d_header, ctx->header_len, difficulty_bits,
            start_nonce, iters_per_thread,
            ctx->d_found, ctx->d_result_nonce
        );
    }

    cudaDeviceSynchronize();

    cudaMemcpy(&h_found, ctx->d_found,        sizeof(int),      cudaMemcpyDeviceToHost);
    cudaMemcpy(&h_nonce, ctx->d_result_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);

    if (h_found) { *nonce_out = h_nonce; return 1; }
    return 0;
}

// Legacy wrapper kept for compatibility — allocates on every call (slow).
int launch_mining_kernel(
    const uint8_t *header_template,
    uint16_t       header_len,
    uint8_t        difficulty_bits,
    uint64_t       start_nonce,
    int            blocks,
    int            threads,
    uint32_t       iters_per_thread,
    uint64_t      *nonce_out
) {
    MiningCtx *ctx = mining_ctx_create(header_len);
    int ret = launch_mining_kernel_ctx(ctx, header_template, difficulty_bits,
                                       start_nonce, blocks, threads,
                                       iters_per_thread, nonce_out);
    mining_ctx_destroy(ctx);
    return ret;
}

} // extern "C"
