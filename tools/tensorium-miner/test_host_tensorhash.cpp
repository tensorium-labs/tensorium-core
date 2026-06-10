// tools/tensorium-miner/test_host_tensorhash.cpp
// Host-side KAT harness — runs on any x86 box, no GPU/CUDA required.
// Build+run: make test-host
#include "blake2b.cuh"
#include "host_tensorhash.h"
#include <stdio.h>
#include <string.h>

static int hex_eq(const uint8_t h[32], const char *hex) {
    char got[65];
    for (int i = 0; i < 32; i++) sprintf(got + i * 2, "%02x", h[i]);
    got[64] = '\0';
    if (strcmp(got, hex) == 0) return 1;
    fprintf(stderr, "  got      %s\n  expected %s\n", got, hex);
    return 0;
}

static int g_failures = 0;
#define CHECK(name, cond) do { \
    if (cond) printf("PASS  %s\n", name); \
    else { printf("FAIL  %s\n", name); g_failures++; } \
} while (0)

static void blake2b_vectors(void) {
    uint8_t out[32];
    th_blake2b256((const uint8_t *)"", 0, out);
    CHECK("V1 blake2b256(empty)", hex_eq(out,
        "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"));

    th_blake2b256((const uint8_t *)"abc", 3, out);
    CHECK("V2 blake2b256(abc)", hex_eq(out,
        "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"));

    uint8_t a142[142];
    memset(a142, 'a', sizeof(a142));
    th_blake2b256(a142, 142, out);  /* exercises the two-block path */
    CHECK("V3 blake2b256(142*'a')", hex_eq(out,
        "b318961b001b73c05a5cd3c224fa1468772a46b039ca9ad84ff1788a321bf49e"));
}

static void tensorhash_vectors(void) {
    uint8_t out[32];
    uint8_t zero_seed[32] = {0};

    host_dataset_element(zero_seed, 0, out);
    CHECK("V4 elem(0)", hex_eq(out,
        "4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800"));

    host_dataset_element(zero_seed, 599999999ULL, out);
    CHECK("V5 elem(N-1)", hex_eq(out,
        "b7bc37d22421db9279c262ef23d75a606372411972b589410f32b9ca22b82e81"));

    host_dataset_element(zero_seed, 123456789ULL, out);
    CHECK("V6 elem(123456789)", hex_eq(out,
        "6cb58c6796255d9e11b3db3237571be55114bc5cc3b11dc137eae82547fde646"));

    host_pow_hash((const uint8_t *)TH_KAT_POW_HEADER,
                  (uint32_t)strlen(TH_KAT_POW_HEADER),
                  TH_KAT_POW_NONCE, zero_seed, out);
    CHECK("V7 pow_hash KAT", hex_eq(out, TH_KAT_POW_HEX));

    uint8_t one_seed[32];
    memset(one_seed, 1, 32);
    uint8_t xprefix[102];
    memset(xprefix, 'x', sizeof(xprefix));
    host_pow_hash(xprefix, 102, 777, one_seed, out);
    CHECK("V8 pow_hash non-zero seed", hex_eq(out,
        "cd22f6a0e831f8d7387c59f0e620d12917a73944c7b44991722bb23452712491"));

    CHECK("kat_check() aggregate", host_tensorhash_kat_check() == 0);
}

int main(void) {
    blake2b_vectors();
    tensorhash_vectors();
    if (g_failures) { printf("\n%d FAILURE(S)\n", g_failures); return 1; }
    printf("\nall host KATs pass\n");
    return 0;
}
