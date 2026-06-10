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
}
