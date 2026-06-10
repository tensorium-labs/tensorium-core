// tools/tensorium-miner/host_tensorhash.cpp
#include "host_tensorhash.h"
#include "tensorhash_params.h"
#include "blake2b.cuh"
#include <string.h>

static void le64_store(uint8_t *b, uint64_t v) {
    for (int i = 0; i < 8; i++) { b[i] = (uint8_t)v; v >>= 8; }
}
static uint64_t le64_load(const uint8_t *b) {
    uint64_t v = 0;
    for (int i = 7; i >= 0; i--) v = (v << 8) | b[i];
    return v;
}
static uint64_t rotl64(uint64_t x, int n) { return (x << n) | (x >> (64 - n)); }

void host_dataset_element(const uint8_t seed[32], uint64_t index, uint8_t out[32]) {
    uint8_t buf[40];
    memcpy(buf, seed, 32);
    le64_store(buf + 32, index);
    th_blake2b256(buf, 40, out);
}

void host_pow_hash(const uint8_t *prefix, uint32_t prefix_len, uint64_t nonce,
                   const uint8_t seed[32], uint8_t out[32]) {
    uint8_t buf[TH_PREFIX_MAX + 8 + 32];
    memcpy(buf, prefix, prefix_len);
    le64_store(buf + prefix_len, nonce);

    uint8_t digest[32];
    th_blake2b256(buf, prefix_len + 8, digest);

    uint64_t acc[4];
    for (int m = 0; m < 4; m++) acc[m] = le64_load(digest + m * 8);

    uint8_t ibuf[40], iseed[32], elem_bytes[32];
    memcpy(ibuf, digest, 32);
    for (uint64_t j = 0; j < TH_K; j++) {
        le64_store(ibuf + 32, j);
        th_blake2b256(ibuf, 40, iseed);
        uint64_t idx = le64_load(iseed) % TH_DATASET_N;

        host_dataset_element(seed, idx, elem_bytes);
        uint64_t elem[4];
        for (int m = 0; m < 4; m++) elem[m] = le64_load(elem_bytes + m * 8);

        uint64_t next[4];
        for (int m = 0; m < 4; m++)
            next[m] = acc[m] * (elem[m] | 1ULL) + rotl64(elem[(m + 1) & 3], 13);
        for (int m = 0; m < 4; m++) acc[m] = next[m];
    }

    /* final hash input: prefix || nonce_le || acc_bytes (buf already holds
       prefix||nonce — append the accumulator) */
    for (int m = 0; m < 4; m++) le64_store(buf + prefix_len + 8 + m * 8, acc[m]);
    th_blake2b256(buf, prefix_len + 8 + 32, out);
}

int host_leading_zero_bits(const uint8_t hash[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (hash[i] == 0) { bits += 8; continue; }
        unsigned x = hash[i];
        while (!(x & 0x80)) { bits++; x <<= 1; }
        break;
    }
    return bits;
}

const char    *TH_KAT_POW_HEADER = "tensorhash-v1-kat-vector";
const uint64_t TH_KAT_POW_NONCE  = 12345;
const char    *TH_KAT_POW_HEX =
    "9eddf122dc2f33d206ef3bb7f2e32fbd049fa00f9be7cb9a98f6f7055666e47f";

int th_hex32_to_bytes(const char *hex, uint8_t out[32]) {
    for (int i = 0; i < 32; i++) {
        int hi = hex[i * 2], lo = hex[i * 2 + 1];
        hi = (hi >= 'a') ? hi - 'a' + 10 : (hi >= 'A') ? hi - 'A' + 10 : hi - '0';
        lo = (lo >= 'a') ? lo - 'a' + 10 : (lo >= 'A') ? lo - 'A' + 10 : lo - '0';
        if (hi < 0 || hi > 15 || lo < 0 || lo > 15) return 0;
        out[i] = (uint8_t)((hi << 4) | lo);
    }
    return 1;
}

int host_tensorhash_kat_check(void) {
    uint8_t out[32], expect[32];
    uint8_t zero_seed[32] = {0};

    /* 1: blake2b two-block path */
    uint8_t a142[142];
    memset(a142, 'a', sizeof(a142));
    th_blake2b256(a142, 142, out);
    th_hex32_to_bytes("b318961b001b73c05a5cd3c224fa1468772a46b039ca9ad84ff1788a321bf49e", expect);
    if (memcmp(out, expect, 32) != 0) return 1;

    /* 2: dataset element 0 */
    host_dataset_element(zero_seed, 0, out);
    th_hex32_to_bytes("4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800", expect);
    if (memcmp(out, expect, 32) != 0) return 2;

    /* 3: dataset element N-1 */
    host_dataset_element(zero_seed, TH_DATASET_N - 1, out);
    th_hex32_to_bytes("b7bc37d22421db9279c262ef23d75a606372411972b589410f32b9ca22b82e81", expect);
    if (memcmp(out, expect, 32) != 0) return 3;

    /* 4: full pow_hash KAT (zero seed) */
    host_pow_hash((const uint8_t *)TH_KAT_POW_HEADER,
                  (uint32_t)strlen(TH_KAT_POW_HEADER),
                  TH_KAT_POW_NONCE, zero_seed, out);
    th_hex32_to_bytes(TH_KAT_POW_HEX, expect);
    if (memcmp(out, expect, 32) != 0) return 4;

    /* 5: pow_hash with non-zero seed + real 102-byte prefix length */
    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    host_pow_hash(xprefix, 102, 777, one_seed, out);
    th_hex32_to_bytes("cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491", expect);
    if (memcmp(out, expect, 32) != 0) return 5;

    return 0;
}
