//! TensorHash v1 — Tensorium's memory-hard, GPU-first proof-of-work
//! algorithm. Pure-Rust reference implementation used by `tensorium-core`
//! for block validation (light verification — see crate docs in
//! `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-a-design.md`).

pub const ELEMENT_SIZE: usize = 32;
pub const DATASET_N: u64 = 600_000_000;
pub const EPOCH_LENGTH: u64 = 8_192;
pub const K: usize = 32;
