// tools/tensorium-miner/host_tensorhash.h
// Host CPU reference for TensorHash v1 — cheap verification (recomputes only
// the K=32 touched dataset elements per attempt; the full dataset lives only
// in GPU VRAM). Used for share/block pre-verification and selftest oracles.
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

void host_dataset_element(const uint8_t seed[32], uint64_t index, uint8_t out[32]);

/* prefix = serialized header bytes WITHOUT the trailing 8 nonce bytes.
   prefix_len <= TH_PREFIX_MAX. */
void host_pow_hash(const uint8_t *prefix, uint32_t prefix_len, uint64_t nonce,
                   const uint8_t seed[32], uint8_t out[32]);

int host_leading_zero_bits(const uint8_t hash[32]);

/* Runs every hardcoded KAT vector; returns 0 on full pass, else the 1-based
   index of the first failing vector. Selftest layer 1. */
int host_tensorhash_kat_check(void);

/* The full-pipeline KAT (Rust: pow_hash_known_answer_vector) — shared with
   the GPU selftest so the kernel path checks the same vector. */
extern const char    *TH_KAT_POW_HEADER;   /* "tensorhash-v1-kat-vector" */
extern const uint64_t TH_KAT_POW_NONCE;    /* 12345 */
extern const char    *TH_KAT_POW_HEX;      /* expected pow-hash hex */

/* 64-char hex -> 32 bytes; returns 1 on success. */
int th_hex32_to_bytes(const char *hex, uint8_t out[32]);

#ifdef __cplusplus
}
#endif
