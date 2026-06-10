// tools/tensorium-miner/tensorhash_params.h
// TensorHash v1 consensus parameters — MUST match crates/tensorium-tensorhash.
#pragma once
#include <stdint.h>

#define TH_ELEMENT_SIZE   32
#define TH_DATASET_N      600000000ULL
#define TH_DATASET_BYTES  (TH_DATASET_N * (uint64_t)TH_ELEMENT_SIZE)  /* 19.2 GB */
#define TH_EPOCH_LENGTH   8192ULL
#define TH_K              32
#define TH_PREFIX_MAX     184   /* HEADER_MAX(192) - 8 nonce bytes */
