# Phase A2 — GPU Runtime Validation Notes (2026-06-10)

**Hardware:** vast.ai rental — NVIDIA GeForce RTX 5090 (32 GB, Blackwell
`sm_120`), driver 580.126.09, CUDA 13.0, 566 GB RAM.
**Tree:** `main` @ `aad3cbe` (Phase A2 + two live-path fixes found during
this session).

## Results

| Check | Result |
|---|---|
| `make test-host` (host KATs V1–V8) | ✅ all pass |
| `make` (auto-detected `ARCH=sm_120`) | ✅ builds clean |
| `--selftest` (layers 1, 2, 3, 4a, 4b) | ✅ **ALL PASS — GPU implementation is consensus-equivalent, bit-for-bit** |
| Dataset generation (19.2 GB, 600M elements) | **0.14 s** |
| `--benchmark 120` sustained hashrate | **220.31 MH/s** (blocks=8192, threads=256) |
| Devnet live-path (template → mine → submit) | ✅ 8 blocks accepted in one 90 s run; chain reached height 11 |
| Genesis dry-run (`--mode genesis`, placeholder ts 1780272000, **36 bits**) | ✅ nonce `101230883021`, host-verified, **9 m 55 s** (E≈5.2 m, 1.5× variance) |

The dry-run used 36 bits (2^36 attempts ≈ 5 min expected) to validate the
pipeline without burning ~5.5 GPU-hours; the nonce is NOT committed anywhere
(it is neither 42-bit nor for the final launch timestamp). `verify-genesis`'s
VALID path therefore remains exercised only by the real launch-time re-mine;
its INVALID path and the prefix/mine/host-verify loop are fully validated.

## Bugs found and fixed by the live-path test

Both pre-existing solo-mode bugs in `tools/tensorium-miner/solo_client.cpp`
(v2-era code), invisible at SHA256d-mainnet share rates but immediately fatal
at devnet's ~170 shares/s:

1. **`b97fe4a`** — `replace_header_nonce` resumed printing from the
   header-only buffer copy after splicing the nonce, dropping
   `"transactions"` from the submitted block JSON. Every solo submit was
   rejected with `EOF while parsing an object`.
2. **`aad3cbe`** — shares were submitted without checking `job_id` against
   the current job (stale nonces spliced into new templates), and the 10 s
   template refresh overwrote the cached template JSON without republishing
   the job (GPU mined the old timestamp, submits used the new one). Both
   paths produced `block proof-of-work is invalid` on every submission after
   the first.

## Difficulty calibration (decision input)

Measured: RTX 5090 = **220 MH/s**. At the shipped initial difficulty of
**42 bits** (2^42 ≈ 4.4×10¹² expected attempts):

- One RTX 5090 solo: expected **~5.5 hours per block** — ~330× slower than
  the 60 s target.
- 60 s blocks at 42 bits require ~73 GH/s of network hashrate ≈ **330×
  RTX 5090s** — far beyond any realistic launch fleet.
- With retargeting at ±1 bit per 60-block window, a single-GPU launch would
  take **weeks per retarget step** to walk the difficulty down.

This exceeds the agreed ">4× off → recalibrate" threshold by two orders of
magnitude. Reference points for `initial_leading_zero_bits` (60 s target):

| Launch fleet | Hashrate | Balanced bits |
|---|---|---|
| 1× 5090 | 220 MH/s | ~33.6 |
| 5× 5090-class | 1.1 GH/s | ~36.0 |
| 20× 5090-class | 4.4 GH/s | ~38.0 |

**Recommendation:** `initial_leading_zero_bits: 36` (min 28 / max 52,
preserving the −8/+16 spread). One 5090 then averages ~5 min/block at
genesis and retargeting corrects toward 60 s as miners join. The genesis
re-mine itself at 36 bits takes minutes instead of GPU-hours.

**Decision (2026-06-10): keep 42 bits — no recalibration.** Accepted
implications: the launch-time genesis re-mine takes ~5.5 expected GPU-hours
on one RTX 5090 (rent more/bigger GPUs to shorten), and early-chain block
times will be well above 60 s until network hashrate grows or retargeting
(−1 bit per 60-block window, floor 34) walks the difficulty down.

## Pre-launch checklist addition (Phase A3, 2026-06-10)

Pool↔GPU live test — bundle with the genesis re-mine rental: devnet node +
`tensorium-pool` + `tensorium-miner --mode pool`; verify the miner receives
`epoch_seed` in `mining.notify`, shares are accepted at the vardiff target
rate, a found block is submitted and accepted by the node, and the PPLNS
split is recorded in the ledger.
