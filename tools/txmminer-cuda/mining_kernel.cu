// mining_kernel.cu — CUDA SHA256d mining kernel for Tensorium
// Called from main.cu via launch_mining_kernel()

#include "sha256d.cuh"
#include <stdint.h>

// ── Kernel ────────────────────────────────────────────────────────────────────
//
// Each thread tries nonces: start_nonce + (blockIdx.x * blockDim.x + threadIdx.x)
// stepping by total_threads each iteration, for `iters` iterations.
//
// Outputs:
//   found[0]      = 1 if a valid nonce was found
//   result_nonce  = the winning nonce (u64)

__global__ void mine_kernel(
    const uint32_t midstate[8],   // SHA256 state after block1 (constant)
    const uint8_t  block2_prefix[40], // block2 bytes [0..39] (before nonce)
    uint8_t        difficulty_bits,   // required leading zero bits
    uint64_t       start_nonce,       // global starting nonce
    uint32_t       iters,             // nonce attempts per thread
    int           *found,             // output: 1 if found
    uint64_t      *result_nonce       // output: winning nonce
) {
    if (*found) return;  // early exit if already found

    uint32_t gid = blockIdx.x * blockDim.x + threadIdx.x;
    uint32_t stride = gridDim.x * blockDim.x;
    uint64_t nonce = start_nonce + (uint64_t)gid;

    uint8_t block2[64];
    uint8_t hash[32];

    // Copy fixed prefix (block2[0..39])
    for (int i = 0; i < 40; i++) block2[i] = block2_prefix[i];

    // SHA256 padding for 112-byte input:
    block2[48] = 0x80;
    for (int i = 49; i < 56; i++) block2[i] = 0x00;
    // bit length = 112 * 8 = 896 = 0x380  (big-endian u64)
    block2[56] = 0x00; block2[57] = 0x00; block2[58] = 0x00; block2[59] = 0x00;
    block2[60] = 0x00; block2[61] = 0x00; block2[62] = 0x03; block2[63] = 0x80;

    for (uint32_t i = 0; i < iters; i++) {
        if (*found) return;

        // Write nonce as little-endian u64 at block2[40..47]
        block2[40] = (uint8_t)(nonce);
        block2[41] = (uint8_t)(nonce >> 8);
        block2[42] = (uint8_t)(nonce >> 16);
        block2[43] = (uint8_t)(nonce >> 24);
        block2[44] = (uint8_t)(nonce >> 32);
        block2[45] = (uint8_t)(nonce >> 40);
        block2[46] = (uint8_t)(nonce >> 48);
        block2[47] = (uint8_t)(nonce >> 56);

        sha256d_header(midstate, block2, hash);

        if (leading_zero_bits(hash) >= (int)difficulty_bits) {
            if (atomicCAS(found, 0, 1) == 0) {
                *result_nonce = nonce;
            }
            return;
        }

        nonce += (uint64_t)stride;
    }
}

// ── Host-side launch wrapper ──────────────────────────────────────────────────
extern "C" {

// Build the SHA256 midstate of block1 on the CPU.
// block1 = header[0..63]
void compute_midstate(const uint8_t header112[112], uint32_t midstate_out[8]) {
    // SHA256 initial state
    midstate_out[0] = 0x6a09e667;
    midstate_out[1] = 0xbb67ae85;
    midstate_out[2] = 0x3c6ef372;
    midstate_out[3] = 0xa54ff53a;
    midstate_out[4] = 0x510e527f;
    midstate_out[5] = 0x9b05688c;
    midstate_out[6] = 0x1f83d9ab;
    midstate_out[7] = 0x5be0cd19;

    // Load block1 as 16 big-endian u32s
    uint32_t W[16];
    for (int i = 0; i < 16; i++) {
        const uint8_t *p = header112 + i * 4;
        W[i] = ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
               ((uint32_t)p[2] <<  8) |  (uint32_t)p[3];
    }

    // SHA256 compression (CPU version — same logic as device)
    uint32_t Ks[64] = {
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

    uint32_t Wexp[64];
    for (int i = 0; i < 16; i++) Wexp[i] = W[i];
    for (int i = 16; i < 64; i++) {
        uint32_t s0 = (Wexp[i-15] >> 7 | Wexp[i-15] << 25) ^
                      (Wexp[i-15] >> 18 | Wexp[i-15] << 14) ^ (Wexp[i-15] >> 3);
        uint32_t s1 = (Wexp[i-2] >> 17 | Wexp[i-2] << 15) ^
                      (Wexp[i-2] >> 19 | Wexp[i-2] << 13) ^ (Wexp[i-2] >> 10);
        Wexp[i] = Wexp[i-16] + s0 + Wexp[i-7] + s1;
    }

    uint32_t a = midstate_out[0], b = midstate_out[1];
    uint32_t c = midstate_out[2], d = midstate_out[3];
    uint32_t e = midstate_out[4], f = midstate_out[5];
    uint32_t g = midstate_out[6], h = midstate_out[7];

    for (int i = 0; i < 64; i++) {
        uint32_t S1 = (e >> 6 | e << 26) ^ (e >> 11 | e << 21) ^ (e >> 25 | e << 7);
        uint32_t ch = (e & f) ^ (~e & g);
        uint32_t T1 = h + S1 + ch + Ks[i] + Wexp[i];
        uint32_t S0 = (a >> 2 | a << 30) ^ (a >> 13 | a << 19) ^ (a >> 22 | a << 10);
        uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint32_t T2 = S0 + maj;
        h=g; g=f; f=e; e=d+T1; d=c; c=b; b=a; a=T1+T2;
    }

    midstate_out[0] += a; midstate_out[1] += b;
    midstate_out[2] += c; midstate_out[3] += d;
    midstate_out[4] += e; midstate_out[5] += f;
    midstate_out[6] += g; midstate_out[7] += h;
}

// Returns 1 if a valid nonce was found (stored in *nonce_out), 0 otherwise.
int launch_mining_kernel(
    const uint8_t  header112[112],  // full 112-byte header (nonce field ignored)
    uint8_t        difficulty_bits,
    uint64_t       start_nonce,
    int            blocks,          // CUDA grid: number of blocks
    int            threads,         // CUDA block: threads per block
    uint32_t       iters_per_thread,
    uint64_t      *nonce_out
) {
    // Build midstate and block2 prefix on CPU
    uint32_t midstate[8];
    compute_midstate(header112, midstate);

    // block2_prefix = header[64..103] (40 bytes before nonce)
    uint8_t block2_prefix[40];
    for (int i = 0; i < 40; i++) block2_prefix[i] = header112[64 + i];

    // Allocate device memory
    uint32_t *d_midstate;
    uint8_t  *d_block2_prefix;
    int      *d_found;
    uint64_t *d_result_nonce;

    cudaMalloc(&d_midstate,      8 * sizeof(uint32_t));
    cudaMalloc(&d_block2_prefix, 40);
    cudaMalloc(&d_found,         sizeof(int));
    cudaMalloc(&d_result_nonce,  sizeof(uint64_t));

    cudaMemcpy(d_midstate,      midstate,      8 * sizeof(uint32_t), cudaMemcpyHostToDevice);
    cudaMemcpy(d_block2_prefix, block2_prefix, 40,                   cudaMemcpyHostToDevice);

    int h_found = 0;
    uint64_t h_nonce = UINT64_MAX;
    cudaMemcpy(d_found,        &h_found, sizeof(int),      cudaMemcpyHostToDevice);
    cudaMemcpy(d_result_nonce, &h_nonce, sizeof(uint64_t), cudaMemcpyHostToDevice);

    // Launch kernel
    mine_kernel<<<blocks, threads>>>(
        d_midstate, d_block2_prefix, difficulty_bits,
        start_nonce, iters_per_thread,
        d_found, d_result_nonce
    );
    cudaDeviceSynchronize();

    // Retrieve results
    cudaMemcpy(&h_found, d_found,        sizeof(int),      cudaMemcpyDeviceToHost);
    cudaMemcpy(&h_nonce, d_result_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);

    cudaFree(d_midstate);
    cudaFree(d_block2_prefix);
    cudaFree(d_found);
    cudaFree(d_result_nonce);

    if (h_found) { *nonce_out = h_nonce; return 1; }
    return 0;
}

} // extern "C"
