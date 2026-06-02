// mining_kernel.cu — CUDA SHA256d mining kernel for Tensorium
// Called from main.cu via launch_mining_kernel()

#include "sha256d.cuh"
#include <stdint.h>

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

__device__ void sha256d_122(
    const uint32_t midstate[8],
    const uint8_t  block2_const[50],
    uint64_t       nonce,
    uint8_t        hash_out[32]
) {
    uint8_t blk2[64], blk3[64], blk4[64], h1[32];
    uint32_t block2_words[16], block3_words[16], block4_words[16];

    for (int i = 0; i < 50; i++) blk2[i] = block2_const[i];
    blk2[50] = (uint8_t)(nonce);
    blk2[51] = (uint8_t)(nonce >> 8);
    blk2[52] = (uint8_t)(nonce >> 16);
    blk2[53] = (uint8_t)(nonce >> 24);
    blk2[54] = (uint8_t)(nonce >> 32);
    blk2[55] = (uint8_t)(nonce >> 40);
    blk2[56] = (uint8_t)(nonce >> 48);
    blk2[57] = (uint8_t)(nonce >> 56);
    blk2[58] = 0x80;
    for (int i = 59; i < 64; i++) blk2[i] = 0x00;

    for (int i = 0; i < 56; i++) blk3[i] = 0x00;
    blk3[56] = 0x00; blk3[57] = 0x00; blk3[58] = 0x00; blk3[59] = 0x00;
    blk3[60] = 0x00; blk3[61] = 0x00; blk3[62] = 0x03; blk3[63] = 0xD0;

    uint32_t state[8];
    for (int i = 0; i < 8; i++) state[i] = midstate[i];

    for (int i = 0; i < 16; i++) {
        block2_words[i] = load_be32(blk2 + i * 4);
        block3_words[i] = load_be32(blk3 + i * 4);
    }
    sha256_compress(state, block2_words);
    sha256_compress(state, block3_words);

    for (int i = 0; i < 8; i++) {
        h1[i * 4 + 0] = (uint8_t)(state[i] >> 24);
        h1[i * 4 + 1] = (uint8_t)(state[i] >> 16);
        h1[i * 4 + 2] = (uint8_t)(state[i] >> 8);
        h1[i * 4 + 3] = (uint8_t)(state[i]);
    }

    for (int i = 0; i < 32; i++) blk4[i] = h1[i];
    blk4[32] = 0x80;
    for (int i = 33; i < 56; i++) blk4[i] = 0x00;
    blk4[56] = 0x00; blk4[57] = 0x00; blk4[58] = 0x00; blk4[59] = 0x00;
    blk4[60] = 0x00; blk4[61] = 0x00; blk4[62] = 0x01; blk4[63] = 0x00;

    uint32_t state2[8] = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };
    for (int i = 0; i < 16; i++) block4_words[i] = load_be32(blk4 + i * 4);
    sha256_compress(state2, block4_words);

    for (int i = 0; i < 8; i++) {
        hash_out[i * 4 + 0] = (uint8_t)(state2[i] >> 24);
        hash_out[i * 4 + 1] = (uint8_t)(state2[i] >> 16);
        hash_out[i * 4 + 2] = (uint8_t)(state2[i] >> 8);
        hash_out[i * 4 + 3] = (uint8_t)(state2[i]);
    }
}

__global__ void mine_kernel_122(
    const uint32_t *midstate,
    const uint8_t  *block2_const,
    uint8_t         difficulty_bits,
    uint64_t        start_nonce,
    uint32_t        iters,
    int            *found,
    uint64_t       *result_nonce
) {
    if (*found) return;

    uint32_t gid = blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint64_t nonce = start_nonce + gid;
    uint8_t hash[32];

    for (uint32_t i = 0; i < iters; i++) {
        if (*found) return;
        sha256d_122(midstate, block2_const, nonce, hash);
        if (leading_zero_bits(hash) >= (int)difficulty_bits) {
            if (atomicCAS(found, 0, 1) == 0) {
                *result_nonce = nonce;
            }
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

extern "C" {

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
    uint8_t  *d_header_template = nullptr;
    uint32_t *d_midstate = nullptr;
    uint8_t  *d_block2_const = nullptr;
    int      *d_found = nullptr;
    uint64_t *d_result_nonce = nullptr;

    cudaMalloc(&d_found, sizeof(int));
    cudaMalloc(&d_result_nonce, sizeof(uint64_t));

    int h_found = 0;
    uint64_t h_nonce = UINT64_MAX;
    cudaMemcpy(d_found, &h_found, sizeof(int), cudaMemcpyHostToDevice);
    cudaMemcpy(d_result_nonce, &h_nonce, sizeof(uint64_t), cudaMemcpyHostToDevice);

    if (header_len == 122) {
        uint32_t midstate[8];
        uint8_t block2_const[50];
        compute_midstate_64(header_template, midstate);
        for (int i = 0; i < 50; i++) block2_const[i] = header_template[64 + i];

        cudaMalloc(&d_midstate, 8 * sizeof(uint32_t));
        cudaMalloc(&d_block2_const, 50);
        cudaMemcpy(d_midstate, midstate, 8 * sizeof(uint32_t), cudaMemcpyHostToDevice);
        cudaMemcpy(d_block2_const, block2_const, 50, cudaMemcpyHostToDevice);

        mine_kernel_122<<<blocks, threads>>>(
            d_midstate, d_block2_const, difficulty_bits,
            start_nonce, iters_per_thread,
            d_found, d_result_nonce
        );
    } else {
        cudaMalloc(&d_header_template, header_len);
        cudaMemcpy(d_header_template, header_template, header_len, cudaMemcpyHostToDevice);
        mine_kernel_generic<<<blocks, threads>>>(
            d_header_template, header_len, difficulty_bits,
            start_nonce, iters_per_thread,
            d_found, d_result_nonce
        );
    }

    cudaDeviceSynchronize();

    cudaMemcpy(&h_found, d_found, sizeof(int), cudaMemcpyDeviceToHost);
    cudaMemcpy(&h_nonce, d_result_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);

    if (d_header_template) cudaFree(d_header_template);
    if (d_midstate) cudaFree(d_midstate);
    if (d_block2_const) cudaFree(d_block2_const);
    cudaFree(d_found);
    cudaFree(d_result_nonce);

    if (h_found) {
        *nonce_out = h_nonce;
        return 1;
    }
    return 0;
}

} // extern "C"
