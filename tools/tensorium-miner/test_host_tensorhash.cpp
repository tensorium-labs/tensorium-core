// tools/tensorium-miner/test_host_tensorhash.cpp
// Host-side KAT harness — runs on any x86 box, no GPU/CUDA required.
// Build+run: make test-host
#include "blake2b.cuh"
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

int main(void) {
    blake2b_vectors();
    if (g_failures) { printf("\n%d FAILURE(S)\n", g_failures); return 1; }
    printf("\nall host KATs pass\n");
    return 0;
}
