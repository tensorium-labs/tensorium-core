//! TensorHash v1 — Tensorium's memory-hard, GPU-first proof-of-work
//! algorithm. Pure-Rust reference implementation used by `tensorium-core`
//! for block validation (light verification — see crate docs in
//! `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a-design.md`).

use blake2::digest::{consts::U32, Digest};
use blake2::Blake2b;

pub const ELEMENT_SIZE: usize = 32;
pub const DATASET_N: u64 = 600_000_000;
pub const EPOCH_LENGTH: u64 = 8_192;
pub const K: usize = 32;

type Blake2b256 = Blake2b<U32>;

/// One element of the TensorHash dataset for the given epoch seed.
///
/// Computable on demand — this is what makes verification cheap (a verifier
/// recomputes only the `K` elements a given attempt touches) while mining is
/// memory-hard (a miner materializes all `DATASET_N` elements into VRAM
/// because recomputing per-attempt is ~`K`x slower).
pub fn dataset_element(epoch_seed: &[u8; 32], index: u64) -> [u8; 32] {
    let mut hasher = Blake2b256::new();
    hasher.update(epoch_seed);
    hasher.update(index.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn bytes_to_u64x4(bytes: &[u8; 32]) -> [u64; 4] {
    let mut out = [0u64; 4];
    for (i, word) in out.iter_mut().enumerate() {
        *word = u64::from_le_bytes(bytes[i * 8..(i + 1) * 8].try_into().unwrap());
    }
    out
}

fn u64x4_to_bytes(words: &[u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// TensorHash v1 proof-of-work hash.
///
/// `header_bytes` is the nonce-independent serialized header prefix
/// (`BlockHeader::pow_prefix_bytes` in `tensorium-core`). `epoch_seed` is the
/// dataset seed for the block's epoch (id-hash of the last block of the
/// previous epoch; `[0u8; 32]` for epoch 0).
///
/// Algorithm:
/// 1. `digest = Blake2b256(header_bytes || nonce_le)`
/// 2. Initialize a 4xu64 accumulator from `digest`.
/// 3. For `j in 0..K`: derive an index from `Blake2b256(digest || j_le)`,
///    look up `dataset_element(epoch_seed, idx)`, and fold it into the
///    accumulator via the TensorMix multiply-rotate-add step.
/// 4. Return `Blake2b256(header_bytes || nonce_le || acc_bytes)`.
pub fn pow_hash(header_bytes: &[u8], nonce: u64, epoch_seed: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Blake2b256::new();
    hasher.update(header_bytes);
    hasher.update(nonce.to_le_bytes());
    let digest_full = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&digest_full);

    let mut acc = bytes_to_u64x4(&digest);

    for j in 0..K as u64 {
        let mut h = Blake2b256::new();
        h.update(digest);
        h.update(j.to_le_bytes());
        let idx_seed = h.finalize();
        let idx = u64::from_le_bytes(idx_seed[0..8].try_into().unwrap()) % DATASET_N;

        let elem = bytes_to_u64x4(&dataset_element(epoch_seed, idx));
        let mut next = [0u64; 4];
        for m in 0..4 {
            next[m] = acc[m]
                .wrapping_mul(elem[m] | 1)
                .wrapping_add(elem[(m + 1) % 4].rotate_left(13));
        }
        acc = next;
    }

    let acc_bytes = u64x4_to_bytes(&acc);
    let mut h = Blake2b256::new();
    h.update(header_bytes);
    h.update(nonce.to_le_bytes());
    h.update(acc_bytes);
    let final_full = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&final_full);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_element_is_deterministic() {
        let seed = [0u8; 32];
        assert_eq!(dataset_element(&seed, 42), dataset_element(&seed, 42));
    }

    #[test]
    fn dataset_element_differs_by_index() {
        let seed = [0u8; 32];
        assert_ne!(dataset_element(&seed, 0), dataset_element(&seed, 1));
    }

    #[test]
    fn dataset_element_differs_by_seed() {
        let a = dataset_element(&[0u8; 32], 7);
        let b = dataset_element(&[1u8; 32], 7);
        assert_ne!(a, b);
    }

    #[test]
    fn dataset_element_zero_zero_known_value() {
        // Locks the exact byte layout (Blake2b-256 of 32 zero bytes ||
        // 8 zero bytes for index 0) — this is the cross-check value the
        // future CUDA implementation's --selftest must reproduce.
        let seed = [0u8; 32];
        let elem = dataset_element(&seed, 0);
        let hex = hex::encode(elem);
        println!("dataset_element([0;32], 0) = {hex}");
        // Computed by Blake2b-256("\x00"*32 || "\x00"*8):
        assert_eq!(
            hex,
            "4a1931803561f431decab002e7425f0a8531d5e456a1a47fd9998a2530c0f800"
        );
    }

    #[test]
    fn pow_hash_is_deterministic() {
        let seed = [0u8; 32];
        let header = b"test-header-bytes";
        assert_eq!(pow_hash(header, 0, &seed), pow_hash(header, 0, &seed));
    }

    #[test]
    fn pow_hash_changes_with_nonce() {
        let seed = [0u8; 32];
        let header = b"test-header-bytes";
        assert_ne!(pow_hash(header, 0, &seed), pow_hash(header, 1, &seed));
    }

    #[test]
    fn pow_hash_changes_with_epoch_seed() {
        let header = b"test-header-bytes";
        let a = pow_hash(header, 0, &[0u8; 32]);
        let b = pow_hash(header, 0, &[1u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn pow_hash_known_answer_vector() {
        // Locks down the full algorithm output for a fixed input — this is the
        // primary cross-check value the future CUDA --selftest must reproduce
        // bit-for-bit.
        let seed = [0u8; 32];
        let header = b"tensorhash-v1-kat-vector";
        let hash = pow_hash(header, 12345, &seed);
        let hex = hex::encode(hash);
        println!("pow_hash KAT = {hex}");
        assert_eq!(
            hex,
            "9eddf122dc2f33d206ef3bb7f2e32fbd049fa00f9be7cb9a98f6f7055666e47f"
        );
    }
}
