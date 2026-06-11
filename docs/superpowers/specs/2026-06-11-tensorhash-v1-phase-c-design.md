# TensorHash v1 Relaunch — Phase C Design (Consumer + Core Chain-Correctness)

Date: 2026-06-11
Status: Approved (brainstorming) — pending spec review → writing-plans

## Goal & Boundary

Make every consumer surface and the core `tensorium-node` CLI correct for the
relaunched chain (`chain_id = "tensorium-mainnet"`, PoW = TensorHash v1,
difficulty 42 bits). **Code + config only.** The `tensorium-docs` SHA256d /
branding rewrite and tokenomics prose are deferred to Phase E. The VPS
redeploy, DNS/nginx cutover, and genesis mine are Phase D.

Key finding from the audit: **the RPC/JSON shape did not change with
TensorHash.** Consumers parse the same block-header fields (`chain_id`,
`difficulty_bits`/`leading_zero_bits`, `pow_hash`/`hash`, height, timestamp),
so nothing is *functionally* broken by the new algorithm. The remaining gaps
are stale `mainnet-candidate` strings, a duplicated legacy `mc` CLI namespace
in core, and deployment-coupled endpoints/env-var names.

### Decisions locked during brainstorming
- **Scope:** code+config only; docs (tensorium-docs branding/SHA256d) → Phase E.
- **Core rename:** full rename, **no backward-compat aliases**.
- **RPC endpoint:** consolidate to `rpc.tensoriumlabs.com`; retire
  `mc-rpc.tensoriumlabs.com` (DNS/nginx cutover is a Phase D task).
- Ports unchanged: RPC `33332`, P2P `33333`.

## Component 1 — core (`tensorium-node`): delete the legacy `mc` namespace

The canonical mainnet entry point is **already** the top-level commands
(`init`, `status`, `mine-once`, `rpc`, `p2p-listen`, `p2p-connect`, `daemon`,
`sync`). They use `state_path_from_env()` → `TENSORIUM_STATE` →
`tensorium-mainnet-state.json`, operating on `&MAINNET`.

The `"mainnet-candidate" | "mc"` match arm (`crates/tensorium-node/src/main.rs`
~line 234) is a **full duplicate** of these top-level commands, differing only
by `MC_`-prefixed env vars, constants, and helper functions. "Full rename, no
aliases" therefore means **deleting dead duplication**, not renaming:

- Delete the entire `"mainnet-candidate" | "mc"` match arm and its nested
  subcommands (`init`, `mine-genesis`, `rpc`, `p2p-listen`, `daemon`, `sync`,
  `status`).
- Delete its exclusive helpers: `mc_state_path_from_env`,
  `mc_mempool_path_from_env`, `mc_ban_path_from_env`.
- Delete its exclusive constants: `DEFAULT_MC_STATE_PATH`,
  `DEFAULT_MC_MEMPOOL_PATH`, `DEFAULT_MC_BAN_PATH`, `DEFAULT_MC_RPC_BIND`,
  `DEFAULT_MC_P2P_BIND`, and any MC-only seed-node list.
- Delete the `TENSORIUM_MC_*` env-var reads.
- Rename leftover legacy identifiers used by the *canonical* commands:
  `init_mainnet_candidate_state` → `init_mainnet_state`; comments referencing
  "mainnet-candidate genesis" → "mainnet genesis".
- Strip the `mainnet-candidate` / `mc` section and `TENSORIUM_MC_*` lines from
  `print_help` / usage output.
- Remove the `tensorium-mc-state.db` working-directory artifact from the repo
  root.

**Guard before deleting (must verify, not assume):** confirm every genesis
subcommand the launch needs (`init`, genesis mine, `verify-genesis`,
`print-genesis-prefix`) has a working top-level equivalent. If any exists
*only* under the `mc` arm, **migrate it to a top-level command** rather than
delete it. The launch checklist in
`docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md` is the
source of truth for which subcommands must survive.

**Verification:** the existing 226-test workspace must stay green; `cargo build`
clean; `tensorium-node help` smoke shows no `mc`/`mainnet-candidate` residue;
each surviving genesis subcommand reachable from the canonical namespace.

## Component 2 — `tensorium-bridge-relayer` (private repo)

Files: `index.js`, `api-server.js`, `txm-client.js`, `withdrawal-watcher.js`,
`.env.example`.

- Env rename: `TENSORIUM_MC_RPC` → `TENSORIUM_RPC`,
  `TENSORIUM_MC_STATE` → `TENSORIUM_STATE`,
  `TENSORIUM_MC_RPC_LOCAL` → `TENSORIUM_RPC_LOCAL`.
- Endpoint value `mc-rpc.tensoriumlabs.com` → `rpc.tensoriumlabs.com`.
- State-path default → `tensorium-mainnet-state.json`.
- Ports `33332` / `33333` unchanged.
- Must match the canonical env-var names chosen in Component 1.

## Component 3 — explorer / pool-website / sdk-js / sdk-py

- Replace stale `tensorium-mainnet-candidate-0` → `tensorium-mainnet`:
  - `tensorium-explorer`: `public/index.html` footer fallback + `README.md`.
  - `tensorium-sdk-js`: `README.md`, `test/sdk.test.ts`.
  - `tensorium-sdk-py`: `tests/test_sdk.py`.
  - `tensorium-pool-website`: `README.md`, any `app/*.tsx` hardcodes.
- Endpoint `mc-rpc.tensoriumlabs.com` → `rpc.tensoriumlabs.com` in sdk-js,
  sdk-py, pool-website.
- Explorer: the live `chain_id` is RPC-driven (`server.js` reads it from the
  node) and self-corrects — fix only the hardcoded HTML fallback. Do **not**
  touch the hashrate/difficulty estimation logic.
- **Flagged, deferred to Phase E (cosmetic):** the `block.html` difficulty copy
  `≈ ${2^difficulty_bits/1000}K hashes/block` reads absurdly at 42 bits. Not a
  chain-correctness issue; bundle with the Phase E branding pass.

## Component 4 — `tensorium-wallet-extension`

- Update stale `tensorium-mainnet-candidate-0` → `tensorium-mainnet` in
  `src/__tests__/rpc.test.ts` and `submission/cws-preview.html` (store-submission
  mock fixtures only). Runtime code is already chain-agnostic.
- The actual Chrome Web Store resubmission is deferred to launch time, not
  Phase C.

## Sequencing

1. **core** (`tensorium-core`) — defines the canonical env-var names.
2. **bridge** (`tensorium-bridge-relayer`) — follows core's env names.
3. **explorer / sdk-js / sdk-py / pool-website** — independent string/endpoint
   edits.
4. **wallet-extension** — fixture edits.

Each repo is its own branch and commit. Steps 3 and 4 are independent of each
other and of bridge, but all consumer env/endpoint values must match the
canonical names settled in step 1.

## Verification per repo

- **core:** `cargo test` (workspace, ≥226 pass) + `cargo build` clean + manual
  `help` smoke + reachability check of surviving genesis subcommands.
- **bridge:** lint/load `.env.example`; run any existing node tests.
- **sdk-js:** existing test suite updated and green.
- **sdk-py:** existing test suite updated and green.
- **wallet-extension:** `npm test` green.

## Out of Scope

- **Phase D (deploy):** nginx/DNS `mc-rpc` → `rpc` cutover; VPS redeploy on
  DO + Vultr replacing the candidate chain; genesis mine; server-side state-file
  path change (fresh genesis → no state migration needed).
- **Phase E (docs/branding):** `tensorium-docs` SHA256d → TensorHash rewrite;
  `TOKENOMICS.md`, `MINING.md`; explorer difficulty/hashrate copy; "~7.8558
  TXM/block, halving ~4 yr" branding.
- Launch-time steps (separate from C/D/E): pick final
  `MAINNET_GENESIS_TIMESTAMP`, rent GPU (~5.5 GPU-h at 42 bits on one 5090),
  run `print-genesis-prefix` → genesis mine → `verify-genesis` → commit nonce,
  plus the pool↔GPU live test.

## References

- `docs/superpowers/specs/2026-06-10-tensorhash-v1-phase-b-design.md` (mechanical
  rename already done; this phase finishes what B intentionally deferred).
- `docs/superpowers/specs/2026-06-10-phase-a2-gpu-validation-notes.md` (genesis
  subcommand list + pool live-test checklist).
- New chain constant: `crates/tensorium-core/src/chain.rs` →
  `MAINNET.chain_id = "tensorium-mainnet"`.
