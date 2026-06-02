// mine_genesis.cu — Tensorium MC genesis nonce miner
//
// Finds the nonce for the mainnet-candidate genesis block (v2 with founder allocation).
// Header is 122 bytes (3 SHA256 blocks). Searches at difficulty 40 bits.
//
// Build:
//   nvcc -O3 -arch=sm_86 -o mine_genesis mine_genesis.cu   # RTX 3060 = sm_86
//   nvcc -O3 -arch=sm_80 -o mine_genesis mine_genesis.cu   # A100 = sm_80
//
// Run:
//   ./mine_genesis
//
// Output:
//   GENESIS NONCE: <decimal>   ← paste into MC_GENESIS_NONCE in main.rs

#include <cuda_runtime.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <stdlib.h>

// ── SHA256 constants ─────────────────────────────────────────────────────────

__constant__ uint32_t K[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
};

#define ROTR(x,n) (((x)>>(n))|((x)<<(32-(n))))
#define CH(e,f,g)  (((e)&(f))^(~(e)&(g)))
#define MAJ(a,b,c) (((a)&(b))^((a)&(c))^((b)&(c)))
#define S0(a) (ROTR(a,2)^ROTR(a,13)^ROTR(a,22))
#define S1(e) (ROTR(e,6)^ROTR(e,11)^ROTR(e,25))
#define s0(x) (ROTR(x,7)^ROTR(x,18)^((x)>>3))
#define s1(x) (ROTR(x,17)^ROTR(x,19)^((x)>>10))

#define SHA256_ROUND(a,b,c,d,e,f,g,h,w,k) { \
    uint32_t T1=(h)+S1(e)+CH(e,f,g)+(k)+(w); \
    uint32_t T2=S0(a)+MAJ(a,b,c); \
    (h)=(g);(g)=(f);(f)=(e);(e)=(d)+T1; \
    (d)=(c);(c)=(b);(b)=(a);(a)=T1+T2; }

// Load big-endian u32 from 4 bytes
__device__ __forceinline__ uint32_t lbe(const uint8_t *p) {
    return ((uint32_t)p[0]<<24)|((uint32_t)p[1]<<16)|((uint32_t)p[2]<<8)|(uint32_t)p[3];
}

// ── Single SHA256 compression ─────────────────────────────────────────────────
__device__ void sha256_compress(uint32_t state[8], const uint8_t blk[64]) {
    uint32_t W[64];
    for (int i=0;i<16;i++) W[i]=lbe(blk+i*4);
    for (int i=16;i<64;i++) W[i]=s1(W[i-2])+W[i-7]+s0(W[i-15])+W[i-16];

    uint32_t a=state[0],b=state[1],c=state[2],d=state[3];
    uint32_t e=state[4],f=state[5],g=state[6],h=state[7];

    for (int i=0;i<64;i++) {
        SHA256_ROUND(a,b,c,d,e,f,g,h,W[i],K[i]);
    }
    state[0]+=a; state[1]+=b; state[2]+=c; state[3]+=d;
    state[4]+=e; state[5]+=f; state[6]+=g; state[7]+=h;
}

// ── SHA256 initial state ──────────────────────────────────────────────────────
__device__ void sha256_init(uint32_t state[8]) {
    state[0]=0x6a09e667; state[1]=0xbb67ae85;
    state[2]=0x3c6ef372; state[3]=0xa54ff53a;
    state[4]=0x510e527f; state[5]=0x9b05688c;
    state[6]=0x1f83d9ab; state[7]=0x5be0cd19;
}

// ── SHA256d on 122-byte header ────────────────────────────────────────────────
// Header bytes 0..121, nonce at [114..121] (8 bytes LE).
// 122 bytes → 3 SHA256 blocks (192 bytes padded).
//
// block1 = header[0..63]
// block2 = header[64..121] (58 bytes) + 0x80 + 5 zeros
// block3 = 56 zeros + big-endian u64 message bit length (976 = 0x3D0)
//
// Midstate optimization: block1 is constant → precomputed on CPU and passed as uniform.
__device__ void sha256d_122(
    const uint32_t midstate[8],      // SHA256 state after block1
    const uint8_t  block2_const[50], // header[64..113] — constant
    uint64_t       nonce,            // varies per thread
    uint8_t        hash_out[32]
) {
    // ── First SHA256: compress block2 and block3 ─────────────────────────────
    uint8_t blk2[64], blk3[64];

    // block2: bytes 0..49 = header[64..113], bytes 50..57 = nonce LE
    for (int i=0;i<50;i++) blk2[i]=block2_const[i];
    blk2[50]=(uint8_t)(nonce);
    blk2[51]=(uint8_t)(nonce>>8);
    blk2[52]=(uint8_t)(nonce>>16);
    blk2[53]=(uint8_t)(nonce>>24);
    blk2[54]=(uint8_t)(nonce>>32);
    blk2[55]=(uint8_t)(nonce>>40);
    blk2[56]=(uint8_t)(nonce>>48);
    blk2[57]=(uint8_t)(nonce>>56);
    // padding start + zeros
    blk2[58]=0x80;
    for (int i=59;i<64;i++) blk2[i]=0x00;

    // block3: 56 zeros + 8-byte big-endian bit length
    // bit length = 122 * 8 = 976 = 0x000000000000_03D0
    for (int i=0;i<56;i++) blk3[i]=0x00;
    blk3[56]=0x00; blk3[57]=0x00; blk3[58]=0x00; blk3[59]=0x00;
    blk3[60]=0x00; blk3[61]=0x00; blk3[62]=0x03; blk3[63]=0xD0;

    uint32_t state[8];
    for (int i=0;i<8;i++) state[i]=midstate[i];
    sha256_compress(state, blk2);
    sha256_compress(state, blk3);

    // state → intermediate hash (big-endian)
    uint8_t h1[32];
    for (int i=0;i<8;i++) {
        h1[i*4+0]=(uint8_t)(state[i]>>24);
        h1[i*4+1]=(uint8_t)(state[i]>>16);
        h1[i*4+2]=(uint8_t)(state[i]>>8);
        h1[i*4+3]=(uint8_t)(state[i]);
    }

    // ── Second SHA256: hash of 32-byte intermediate ──────────────────────────
    // 32-byte message → 1 SHA256 block (64 bytes padded)
    // block = h1[0..31] + 0x80 + 23 zeros + 8-byte length (256 = 0x100)
    uint8_t blk4[64];
    for (int i=0;i<32;i++) blk4[i]=h1[i];
    blk4[32]=0x80;
    for (int i=33;i<56;i++) blk4[i]=0x00;
    blk4[56]=0x00; blk4[57]=0x00; blk4[58]=0x00; blk4[59]=0x00;
    blk4[60]=0x00; blk4[61]=0x00; blk4[62]=0x01; blk4[63]=0x00;

    uint32_t state2[8];
    sha256_init(state2);
    sha256_compress(state2, blk4);

    for (int i=0;i<8;i++) {
        hash_out[i*4+0]=(uint8_t)(state2[i]>>24);
        hash_out[i*4+1]=(uint8_t)(state2[i]>>16);
        hash_out[i*4+2]=(uint8_t)(state2[i]>>8);
        hash_out[i*4+3]=(uint8_t)(state2[i]);
    }
}

// Count leading zero bits in 32-byte hash (big-endian)
__device__ int leading_zeros(const uint8_t h[32]) {
    int n=0;
    for (int i=0;i<32;i++) {
        if (h[i]==0) { n+=8; continue; }
        uint8_t b=h[i];
        while (!(b&0x80)) { n++; b<<=1; }
        break;
    }
    return n;
}

// ── Kernel ────────────────────────────────────────────────────────────────────
__global__ void genesis_kernel(
    const uint32_t midstate[8],
    const uint8_t  block2_const[50],
    uint8_t        target_bits,
    uint64_t       start_nonce,
    uint32_t       iters,
    int           *found,
    uint64_t      *result_nonce
) {
    if (*found) return;
    uint32_t gid = blockIdx.x * blockDim.x + threadIdx.x;
    uint64_t stride = (uint64_t)gridDim.x * blockDim.x;
    uint64_t nonce = start_nonce + gid;

    uint8_t hash[32];
    for (uint32_t i=0; i<iters; i++) {
        if (*found) return;
        sha256d_122(midstate, block2_const, nonce, hash);
        if (leading_zeros(hash) >= (int)target_bits) {
            if (atomicCAS(found, 0, 1) == 0) *result_nonce = nonce;
            return;
        }
        nonce += stride;
    }
}

// ── CPU: build header and compute midstate ────────────────────────────────────

static void sha256_init_cpu(uint32_t s[8]) {
    s[0]=0x6a09e667; s[1]=0xbb67ae85; s[2]=0x3c6ef372; s[3]=0xa54ff53a;
    s[4]=0x510e527f; s[5]=0x9b05688c; s[6]=0x1f83d9ab; s[7]=0x5be0cd19;
}

static uint32_t lbe_cpu(const uint8_t *p) {
    return ((uint32_t)p[0]<<24)|((uint32_t)p[1]<<16)|((uint32_t)p[2]<<8)|(uint32_t)p[3];
}

static void sha256_compress_cpu(uint32_t s[8], const uint8_t blk[64]) {
    static const uint32_t Ks[64] = {
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
    };
    uint32_t W[64];
    for (int i = 0; i < 16; i++) W[i] = lbe_cpu(blk + i*4);
    for (int i = 16; i < 64; i++) {
        uint32_t x = W[i-15], y = W[i-2];
        uint32_t s0 = ((x>>7)|(x<<25)) ^ ((x>>18)|(x<<14)) ^ (x>>3);
        uint32_t s1 = ((y>>17)|(y<<15)) ^ ((y>>19)|(y<<13)) ^ (y>>10);
        W[i] = s0 + W[i-16] + s1 + W[i-7];
    }
    uint32_t a=s[0], b=s[1], c=s[2], d=s[3];
    uint32_t e=s[4], f=s[5], g=s[6], h=s[7];
    for (int i = 0; i < 64; i++) {
        uint32_t S1 = ((e>>6)|(e<<26)) ^ ((e>>11)|(e<<21)) ^ ((e>>25)|(e<<7));
        uint32_t ch = (e&f) ^ (~e&g);
        uint32_t T1 = h + S1 + ch + Ks[i] + W[i];
        uint32_t S0 = ((a>>2)|(a<<30)) ^ ((a>>13)|(a<<19)) ^ ((a>>22)|(a<<10));
        uint32_t maj = (a&b) ^ (a&c) ^ (b&c);
        uint32_t T2 = S0 + maj;
        h=g; g=f; f=e; e=d+T1;
        d=c; c=b; b=a; a=T1+T2;
    }
    s[0]+=a; s[1]+=b; s[2]+=c; s[3]+=d;
    s[4]+=e; s[5]+=f; s[6]+=g; s[7]+=h;
}

// Build the 122-byte genesis header (nonce=0) and extract midstate + block2_const
static void build_genesis_header(uint8_t hdr[122]) {
    // MC genesis parameters
    const char *chain_id = "tensorium-mainnet-candidate-0"; // 29 bytes
    uint64_t height      = 0;
    uint64_t timestamp   = 1780272000ULL;
    uint8_t  diff_bits   = 40;

    // Merkle root — post-S1 serialisation (script_pubkey coinbase, 2026-06-02)
    // Verified from: tensorium-node mainnet-candidate mine-genesis 1
    // f555b26269c9a7c3e0454c4ff27f7887925c6f8a46111fefa3ad3425eeb21001
    const uint8_t merkle[32] = {
        0xf5,0x55,0xb2,0x62,0x69,0xc9,0xa7,0xc3,
        0xe0,0x45,0x4c,0x4f,0xf2,0x7f,0x78,0x87,
        0x92,0x5c,0x6f,0x8a,0x46,0x11,0x1f,0xef,
        0xa3,0xad,0x34,0x25,0xee,0xb2,0x10,0x01,
    };

    int p = 0;

    // version (u32 LE) = 1
    hdr[p++]=1; hdr[p++]=0; hdr[p++]=0; hdr[p++]=0;

    // chain_id (29 bytes)
    for (int i=0;i<29;i++) hdr[p++]=(uint8_t)chain_id[i];

    // height (u64 LE)
    for (int i=0;i<8;i++) hdr[p++]=(uint8_t)(height>>(i*8));

    // previous_hash (32 zeros)
    for (int i=0;i<32;i++) hdr[p++]=0;

    // merkle_root
    for (int i=0;i<32;i++) hdr[p++]=merkle[i];

    // timestamp (u64 LE)
    for (int i=0;i<8;i++) hdr[p++]=(uint8_t)(timestamp>>(i*8));

    // difficulty_bits
    hdr[p++]=diff_bits;

    // nonce (u64 LE) — 8 zeros, will be replaced by GPU
    for (int i=0;i<8;i++) hdr[p++]=0;

    // p == 122
}

int main(void) {
    printf("Tensorium MC Genesis Miner (v2 — with founder allocation)\n");
    printf("chain_id:  tensorium-mainnet-candidate-0\n");
    printf("timestamp: 1780272000 (2026-06-01 00:00:00 UTC)\n");
    printf("diff:      40 bits\n");
    printf("founder:   txm18c3t652j0x0sanux3dhse8fqgrqpsdzx97358d\n\n");

    // Build header and compute midstate
    uint8_t hdr[122];
    build_genesis_header(hdr);

    // Print header hex for verification
    printf("Header bytes [0..121]: ");
    for (int i=0;i<122;i++) printf("%02x",hdr[i]);
    printf("\n");
    printf("Merkle root: ");
    for (int i=73;i<105;i++) printf("%02x",hdr[i]);
    printf("\n\n");

    // Compute midstate = SHA256 of block1 = header[0..63]
    uint32_t midstate[8];
    sha256_init_cpu(midstate);
    sha256_compress_cpu(midstate, hdr);  // block1 = hdr[0..63]

    // block2_const = header[64..113] = 50 bytes
    uint8_t block2_const[50];
    for (int i=0;i<50;i++) block2_const[i]=hdr[64+i];

    // GPU config: RTX 5090 — 21760 CUDA cores = 170 SMs × 128 cores
    const int  CUDA_BLOCKS  = 8192;
    const int  CUDA_THREADS = 256;
    const uint32_t ITERS    = 1 << 19;  // ~500k iters per thread per batch
    const uint64_t BATCH    = (uint64_t)CUDA_BLOCKS * CUDA_THREADS * ITERS;

    // Allocate GPU memory
    uint32_t *d_midstate; uint8_t *d_b2c;
    int *d_found; uint64_t *d_nonce;

    cudaMalloc(&d_midstate, 8*4);
    cudaMalloc(&d_b2c,      50);
    cudaMalloc(&d_found,    4);
    cudaMalloc(&d_nonce,    8);

    cudaMemcpy(d_midstate, midstate,      8*4, cudaMemcpyHostToDevice);
    cudaMemcpy(d_b2c,      block2_const,  50,  cudaMemcpyHostToDevice);

    int    h_found = 0;
    uint64_t h_nonce = 0;

    uint64_t start_nonce = 0;
    uint64_t total_hashes = 0;
    time_t t0 = time(NULL);
    time_t last_print = t0;

    printf("Mining... (RTX 5090, ~3+ GH/s expected, ~5 min at diff 40)\n");
    fflush(stdout);

    while (!h_found) {
        cudaMemset(d_found, 0, 4);
        cudaMemset(d_nonce, 0, 8);

        genesis_kernel<<<CUDA_BLOCKS, CUDA_THREADS>>>(
            d_midstate, d_b2c, 40,
            start_nonce, ITERS,
            d_found, d_nonce
        );
        cudaDeviceSynchronize();

        cudaMemcpy(&h_found, d_found, 4, cudaMemcpyDeviceToHost);
        cudaMemcpy(&h_nonce, d_nonce, 8, cudaMemcpyDeviceToHost);

        start_nonce += BATCH;
        total_hashes += BATCH;

        time_t now = time(NULL);
        if (now - last_print >= 5) {
            double elapsed = (double)(now - t0);
            double mhs = (double)total_hashes / elapsed / 1e6;
            uint64_t expected = (1ULL << 40);
            double eta = (elapsed * (double)expected) / (double)total_hashes - elapsed;
            printf("  %.1f GH/s  |  %.2f B hashes  |  ETA ~%.0f s\n",
                   mhs/1000.0,
                   (double)total_hashes/1e9,
                   eta > 0 ? eta : 0.0);
            fflush(stdout);
            last_print = now;
        }
    }

    time_t t1 = time(NULL);
    double elapsed = (double)(t1 - t0);
    double ghash = (double)total_hashes / elapsed / 1e9;

    printf("\n");
    printf("===================================================\n");
    printf("  GENESIS NONCE: %llu\n", (unsigned long long)h_nonce);
    printf("  Time:   %.1f s at %.2f GH/s\n", elapsed, ghash);
    printf("===================================================\n");
    printf("\nUpdate main.rs:\n");
    printf("  const MC_GENESIS_NONCE: u64 = %llu;\n\n", (unsigned long long)h_nonce);
    fflush(stdout);

    cudaFree(d_midstate); cudaFree(d_b2c);
    cudaFree(d_found);    cudaFree(d_nonce);
    return 0;
}
