# TensorHash v1 Phase C Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the core `tensorium-node` CLI and all consumer repos correct for the relaunched chain (`chain_id = "tensorium-mainnet"`, TensorHash v1, 42-bit) by removing the duplicated legacy `mc` namespace and stale `mainnet-candidate` strings/endpoints — code+config only.

**Architecture:** The canonical mainnet entry point is already the top-level `tensorium-node` commands (`state_path_from_env` / `TENSORIUM_STATE`). The `"mainnet-candidate" | "mc"` arm is a full duplicate using `MC_`-prefixed env/consts/helpers. We migrate the two subcommands that exist *only* under `mc` (`daemon`, `mine-genesis`) up to top-level, converge P2P peer routing onto `TENSORIUM_PEERS`, then delete the entire `mc` namespace. Consumer repos get find/replace edits for the stale chain id and the `mc-rpc.tensoriumlabs.com → rpc.tensoriumlabs.com` endpoint.

**Tech Stack:** Rust (tensorium-node, cargo), Node.js (bridge-relayer, sdk-js, pool-website, wallet-extension — vitest where present), Python (sdk-py, pytest), static HTML (explorer), bash (install.sh).

**Repos touched (each is its own git repo / branch / commit set):**
`tensorium-core` (Tasks 1–4), `tensorium-bridge-relayer` (5), `tensorium-explorer` (6), `tensorium-sdk-js` (7), `tensorium-sdk-py` (8), `tensorium-pool-website` (9), `tensorium-wallet-extension` (10). All under `/root/.openclaw/workspace/`.

**Out of scope (do NOT do here):** nginx/DNS `mc-rpc→rpc` cutover, VPS redeploy, genesis mine (Phase D); tensorium-docs SHA256d/branding rewrite and the explorer "K hashes/block" copy (Phase E).

---

## Task 1: Core — converge P2P peer routing onto `TENSORIUM_PEERS`

`peers_for()` currently routes mainnet broadcast through `configured_mc_peers()` (`TENSORIUM_MC_PEERS`). Since only the canonical namespace survives, mainnet must read `TENSORIUM_PEERS`. There is a regression test that pins the old behavior — rewrite it first (TDD).

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-core/crates/tensorium-node/src/main.rs` (test `peers_for_mainnet_candidate_uses_mc_peer_list` ~line 2485; `peers_for` ~line 1432; `configured_mc_peers` ~line 1438; `MC_DEFAULT_SEEDS` ~line 1401)

- [ ] **Step 1: Rewrite the regression test to pin the new behavior**

Replace the whole `peers_for_mainnet_candidate_uses_mc_peer_list` test with:

```rust
    #[test]
    fn peers_for_mainnet_uses_tensorium_peers() {
        // After the mc-namespace removal there is a single peer env var.
        // peers_for(&MAINNET) must read TENSORIUM_PEERS (the canonical list the
        // top-level rpc/p2p/daemon commands and install.sh configure).
        env::set_var("TENSORIUM_PEERS", "10.0.0.2:33333");

        let peers = peers_for(&MAINNET);
        assert_eq!(
            peers,
            vec!["10.0.0.2:33333".to_owned()],
            "mainnet broadcast must use TENSORIUM_PEERS"
        );

        env::remove_var("TENSORIUM_PEERS");
    }
```

- [ ] **Step 2: Run the test, verify it FAILS**

Run: `cd /root/.openclaw/workspace/tensorium-core && cargo test -p tensorium-node peers_for_mainnet_uses_tensorium_peers -- --nocapture`
Expected: FAIL — `peers_for` still returns the `TENSORIUM_MC_PEERS` value (empty/seed list), assertion mismatch.

- [ ] **Step 3: Converge `peers_for` and delete the MC peer helper + seed list**

In `peers_for`, change the mainnet branch to use the canonical list:

```rust
fn peers_for(params: &ConsensusParams) -> Vec<String> {
    // Single canonical peer list for every chain now that the mc namespace is gone.
    let _ = params;
    configured_peers()
}
```

Delete the entire `configured_mc_peers` function (~lines 1438–1453) and the `MC_DEFAULT_SEEDS` const block with its doc comment (~lines 1397–1404).

- [ ] **Step 4: Run the test, verify it PASSES**

Run: `cd /root/.openclaw/workspace/tensorium-core && cargo test -p tensorium-node peers_for_mainnet_uses_tensorium_peers`
Expected: PASS (1 passed).

- [ ] **Step 5: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add crates/tensorium-node/src/main.rs
git commit -m "refactor(node): route mainnet P2P peers via TENSORIUM_PEERS (drop mc peer list)"
```

---

## Task 2: Core — migrate `daemon` and `mine-genesis` to top-level commands

These two subcommands exist **only** under the `mc` arm. Add top-level equivalents using the canonical helpers (`state_path_from_env`, `mempool_path_from_env`, `ban_path_from_env`, `DEFAULT_RPC_BIND`, `DEFAULT_P2P_BIND`) BEFORE deleting the arm in Task 3.

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-core/crates/tensorium-node/src/main.rs` (top-level `match command` block, after the `"sync"` arm ~line 148; `print_help` ~line 793)

- [ ] **Step 1: Add the top-level `daemon` arm**

Insert after the `"sync" => { ... }` arm (before `"peers" =>`):

```rust
        "daemon" => {
            // Run RPC + P2P in one process so they share the same DB path without
            // fighting over the RocksDB exclusive lock (open_rocksdb retries handle
            // the brief simultaneous-open window).
            let rpc_bind = args.get(2).map(String::as_str).unwrap_or(DEFAULT_RPC_BIND).to_owned();
            let p2p_bind = args.get(3).map(String::as_str).unwrap_or(DEFAULT_P2P_BIND).to_owned();
            let mempool_path = mempool_path_from_env();
            let ban_path = ban_path_from_env();

            println!("tensorium mainnet daemon  rpc={rpc_bind}  p2p={p2p_bind}");

            let rpc_state = state_path.clone();
            let rpc_mempool = mempool_path.clone();
            let rpc_handle = thread::spawn(move || {
                serve_rpc(&rpc_bind, rpc_state, rpc_mempool, &MAINNET)
            });

            serve_p2p(&p2p_bind, state_path, mempool_path, ban_path, &MAINNET)?;
            rpc_handle.join().map_err(|_| "RPC thread panicked".to_owned())??;
        }
```

- [ ] **Step 2: Add the top-level `mine-genesis` arm**

Insert immediately after the `"daemon"` arm:

```rust
        "mine-genesis" => {
            let threads = args
                .get(2)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or_else(|| thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
            println!(
                "Mining mainnet genesis: diff={} bits, threads={}, timestamp={}",
                MAINNET.initial_leading_zero_bits, threads, MAINNET_GENESIS_TIMESTAMP
            );
            println!("This may take hours — use tensorium-miner for GPU acceleration.");
            let nonce = mine_genesis_multithreaded(threads)?;
            let state = init_mainnet_state(&state_path, nonce)?;
            println!("GENESIS NONCE: {nonce}  (hardcode this in node binary for v1 release)");
            print_status(&state, &MAINNET);
        }
```

Note: `init_mainnet_state` is the renamed `init_mainnet_candidate_state` (done in Task 3 Step 2). If executing tasks strictly in order, this arm references a name that exists only after Task 3's rename — so do Task 2 and Task 3 as one continuous edit/build cycle, OR temporarily call `init_mainnet_candidate_state` here and let Task 3's rename sweep update it. Either way the build is verified at Task 3 Step 5.

- [ ] **Step 3: Add both commands to `print_help`**

In `print_help`, add these lines alongside the existing top-level command descriptions (near the `rpc`/`p2p-listen` lines):

```rust
    println!("  daemon [rpc_bind] [p2p_bind]  start RPC + P2P in one process (recommended)");
    println!("  mine-genesis [threads]      CPU-mine the genesis nonce (prefer tensorium-miner GPU)");
```

- [ ] **Step 4: Build**

Run: `cd /root/.openclaw/workspace/tensorium-core && cargo build -p tensorium-node`
Expected: compiles (warnings about now-unused mc helpers are OK until Task 3).

- [ ] **Step 5: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add crates/tensorium-node/src/main.rs
git commit -m "feat(node): add top-level daemon + mine-genesis commands"
```

---

## Task 3: Core — delete the `mc` namespace + MC consts/helpers/env

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-core/crates/tensorium-node/src/main.rs`

- [ ] **Step 1: Delete the `"mainnet-candidate" | "mc"` match arm**

Remove the entire arm (`"mainnet-candidate" | "mc" => { ... }`, ~lines 234–345) including its nested `init/mine-genesis/rpc/p2p-listen/daemon/sync/status` subcommands and the `_ => print_help_mc()` fallback.

- [ ] **Step 2: Rename `init_mainnet_candidate_state` → `init_mainnet_state` everywhere**

Run: `cd /root/.openclaw/workspace/tensorium-core && grep -rn 'init_mainnet_candidate_state' crates/tensorium-node/src/main.rs`
Rename the function definition and its remaining caller (top-level `"init"` arm ~line 103) to `init_mainnet_state`.

- [ ] **Step 3: Delete MC constants, helpers, and the `print_help_mc` function**

Delete these items from `main.rs`:
- consts `DEFAULT_MC_STATE_PATH`, `DEFAULT_MC_MEMPOOL_PATH`, `DEFAULT_MC_BAN_PATH` (~lines 33–35)
- consts `DEFAULT_MC_RPC_BIND`, `DEFAULT_MC_P2P_BIND` (~lines 36–37)
- functions `mc_state_path_from_env`, `mc_mempool_path_from_env`, `mc_ban_path_from_env` (~lines 387–402)
- function `print_help_mc` (~lines 831–851)

- [ ] **Step 4: Scrub remaining mc/MC references in help + comments**

Run: `cd /root/.openclaw/workspace/tensorium-core && grep -rniE 'mainnet-candidate|TENSORIUM_MC_|DEFAULT_MC_|_mc_|print_help_mc| mc ' crates/tensorium-node/src/main.rs`
For each hit:
- In `print_help`: delete the line `"  mainnet-candidate    explicit alias for the same mainnet chain"` and the three `TENSORIUM_MC_STATE/MEMPOOL/BANS` lines and the `TENSORIUM_MC_PEERS` line and the "both generic and mc aliases" note (replace that note's text with `"  TENSORIUM_NO_DEFAULT_SEEDS=1  disable built-in mainnet seed list"` if a duplicate doesn't already exist).
- In code comments (e.g. the `DEFAULT_SEEDS` doc, the `peers_for` doc, "Built-in mainnet-candidate seed nodes"): reword "mainnet-candidate" → "mainnet".
Expected after fixing: the grep returns no hits.

- [ ] **Step 5: Build + full workspace test suite**

Run: `cd /root/.openclaw/workspace/tensorium-core && cargo build && cargo test 2>&1 | tail -30`
Expected: clean build (no unused-symbol warnings for mc items); all tests pass (≥226, accounting for the one rewritten test from Task 1).

- [ ] **Step 6: Smoke-test the CLI help has no mc residue and lists migrated commands**

Run: `cd /root/.openclaw/workspace/tensorium-core && ./target/debug/tensorium-node help | grep -iE 'daemon|mine-genesis|mainnet-candidate|mc '`
Expected: lines for `daemon` and `mine-genesis` present; NO `mainnet-candidate` / standalone `mc` lines.

- [ ] **Step 7: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add crates/tensorium-node/src/main.rs
git commit -m "refactor(node): remove legacy mainnet-candidate/mc namespace and MC_* env/consts"
```

---

## Task 4: Core — update `install.sh` and remove the stale state-db artifact

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-core/install.sh`
- Delete: `/root/.openclaw/workspace/tensorium-core/tensorium-mc-state.db/` (working-dir artifact)

- [ ] **Step 1: Update CHAIN_ID and all mc invocations/env in install.sh**

Apply these exact replacements in `install.sh`:
- Line 14: `CHAIN_ID="tensorium-mainnet-candidate-0"` → `CHAIN_ID="tensorium-mainnet"`
- All `TENSORIUM_MC_STATE` → `TENSORIUM_STATE`, `TENSORIUM_MC_MEMPOOL` → `TENSORIUM_MEMPOOL`, `TENSORIUM_MC_BANS` → `TENSORIUM_BANS`, `TENSORIUM_MC_PEERS` → `TENSORIUM_PEERS` (lines 170–172, 176–179, 216–219, 239–241, 280–281).
- All `tensorium-node mainnet-candidate init` → `tensorium-node init` (line 173)
- `tensorium-node mainnet-candidate sync` → `tensorium-node sync` (line 180, including the message text)
- ExecStart line 220: `tensorium-node mainnet-candidate rpc 127.0.0.1:${RPC_PORT}` → `tensorium-node rpc 127.0.0.1:${RPC_PORT}`
- ExecStart line 242: `tensorium-node mainnet-candidate p2p-listen 0.0.0.0:${P2P_PORT}` → `tensorium-node p2p-listen 0.0.0.0:${P2P_PORT}`
- Lines 280–281 echo examples: `mainnet-candidate rpc`/`mainnet-candidate p2p-listen` → `rpc`/`p2p-listen`.

- [ ] **Step 2: Verify no mc residue remains in install.sh**

Run: `cd /root/.openclaw/workspace/tensorium-core && grep -nE 'mainnet-candidate|TENSORIUM_MC_|tensorium-mc' install.sh`
Expected: no output.

- [ ] **Step 3: Syntax-check the script**

Run: `cd /root/.openclaw/workspace/tensorium-core && bash -n install.sh && echo OK`
Expected: `OK`.

- [ ] **Step 4: Remove the stale state-db working artifact**

Run: `cd /root/.openclaw/workspace/tensorium-core && git rm -r --cached tensorium-mc-state.db 2>/dev/null; rm -rf tensorium-mc-state.db; git status --short | grep mc-state || echo "removed"`
Expected: the dir is gone (it is a local DB artifact; if it was never tracked, the `rm -rf` is sufficient).

- [ ] **Step 5: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add install.sh
git commit -m "chore(install): use canonical mainnet commands + TENSORIUM_* env; drop mc artifact"
```

---

## Task 5: Bridge — rename `TENSORIUM_MC_*` env, endpoint, and state default

No test runner in this repo; verify with `node --check`.

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-bridge-relayer/{index.js,api-server.js,txm-client.js,withdrawal-watcher.js,.env.example}`

- [ ] **Step 1: Apply the env/endpoint/state replacements**

- `index.js` line 15: in the required-env list, `"TENSORIUM_MC_STATE"` → `"TENSORIUM_STATE"`.
- `api-server.js` line 72: `process.env.TENSORIUM_MC_RPC || "https://mc-rpc.tensoriumlabs.com"` → `process.env.TENSORIUM_RPC || "https://rpc.tensoriumlabs.com"`.
- `txm-client.js` line 1: `const MC_RPC = process.env.TENSORIUM_MC_RPC || "https://mc-rpc.tensoriumlabs.com";` → `const MC_RPC = process.env.TENSORIUM_RPC || "https://rpc.tensoriumlabs.com";` (keep the local `MC_RPC` identifier name to avoid churn, or rename to `RPC` and update its usages in the file — your choice; if renaming, run `grep -n MC_RPC txm-client.js` and update all).
- `withdrawal-watcher.js` line 16: `process.env.TENSORIUM_MC_RPC_LOCAL || "127.0.0.1:33332"` → `process.env.TENSORIUM_RPC_LOCAL || "127.0.0.1:33332"`.
- `withdrawal-watcher.js` line 21: `TENSORIUM_STATE: process.env.TENSORIUM_MC_STATE,` → `TENSORIUM_STATE: process.env.TENSORIUM_STATE,`.
- `.env.example`: `TENSORIUM_MC_RPC=https://mc-rpc.tensoriumlabs.com` → `TENSORIUM_RPC=https://rpc.tensoriumlabs.com`; `TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json` → `TENSORIUM_STATE=/root/tensorium/tensorium-mainnet-state.json`; `TENSORIUM_MC_RPC_LOCAL=127.0.0.1:33332` → `TENSORIUM_RPC_LOCAL=127.0.0.1:33332`.

- [ ] **Step 2: Verify no mc residue + syntax**

Run:
```bash
cd /root/.openclaw/workspace/tensorium-bridge-relayer
grep -rnE 'TENSORIUM_MC_|mc-rpc\.tensoriumlabs|tensorium-mc-state' index.js api-server.js txm-client.js withdrawal-watcher.js .env.example
for f in index.js api-server.js txm-client.js withdrawal-watcher.js; do node --check "$f" && echo "$f OK"; done
```
Expected: grep prints nothing; each file reports `OK`.

- [ ] **Step 3: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-bridge-relayer
git add index.js api-server.js txm-client.js withdrawal-watcher.js .env.example
git commit -m "chore: rename TENSORIUM_MC_* -> TENSORIUM_*, point RPC at rpc.tensoriumlabs.com"
```

---

## Task 6: Explorer — fix the hardcoded chain-id fallback + README

The live UI reads `chain_id` from RPC and self-corrects; only the static fallback string and README are stale.

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-explorer/public/index.html` (line 147), `/root/.openclaw/workspace/tensorium-explorer/README.md` (line 25)

- [ ] **Step 1: Replace the stale chain id in both files**

- `public/index.html` line 147: `tensorium-mainnet-candidate-0` → `tensorium-mainnet`.
- `README.md` line 25: `` `tensorium-mainnet-candidate-0` `` → `` `tensorium-mainnet` ``.

- [ ] **Step 2: Verify no residue**

Run: `cd /root/.openclaw/workspace/tensorium-explorer && grep -rnE 'mainnet-candidate' public/ README.md`
Expected: no output. (Do NOT touch the hashrate/difficulty estimation logic — that is Phase E.)

- [ ] **Step 3: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-explorer
git add public/index.html README.md
git commit -m "chore: update chain id fallback to tensorium-mainnet"
```

---

## Task 7: SDK-JS — endpoint + test fixture

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-sdk-js/README.md` (lines 16, 76), `/root/.openclaw/workspace/tensorium-sdk-js/test/sdk.test.ts` (line 120)

- [ ] **Step 1: Apply replacements**

- `README.md` line 16: `new TxmRPC('https://mc-rpc.tensoriumlabs.com')` → `new TxmRPC('https://rpc.tensoriumlabs.com')`.
- `README.md` line 76: `` `https://mc-rpc.tensoriumlabs.com` `` → `` `https://rpc.tensoriumlabs.com` ``.
- `test/sdk.test.ts` line 120: `chain_id: 'tensorium-mainnet-candidate-0'` → `chain_id: 'tensorium-mainnet'`.

- [ ] **Step 2: Run the test suite**

Run: `cd /root/.openclaw/workspace/tensorium-sdk-js && npm test`
Expected: vitest passes (the chain_id assertion, if any, now matches `tensorium-mainnet`).

- [ ] **Step 3: Verify no residue + commit**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-js
grep -rnE 'mainnet-candidate|mc-rpc\.tensoriumlabs' README.md test/ src/ 2>/dev/null
git add README.md test/sdk.test.ts
git commit -m "chore: point SDK at rpc.tensoriumlabs.com and tensorium-mainnet chain id"
```
Expected: grep prints nothing before commit.

---

## Task 8: SDK-Py — test fixture

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-sdk-py/tests/test_sdk.py` (line 85)

- [ ] **Step 1: Apply replacement**

`tests/test_sdk.py` line 85: `"chain_id": "tensorium-mainnet-candidate-0"` → `"chain_id": "tensorium-mainnet"`.

- [ ] **Step 2: Run the tests**

Run: `cd /root/.openclaw/workspace/tensorium-sdk-py && (python -m pytest -q || pytest -q)`
Expected: tests pass.

- [ ] **Step 3: Verify no residue + commit**

```bash
cd /root/.openclaw/workspace/tensorium-sdk-py
grep -rnE 'mainnet-candidate' tensorium_sdk tests 2>/dev/null
git add tests/test_sdk.py
git commit -m "chore(test): update fixture chain id to tensorium-mainnet"
```
Expected: grep prints nothing before commit.

---

## Task 9: Pool-website — chain-name strings

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-pool-website/{README.md,app/page.tsx,app/blocks/page.tsx}`

- [ ] **Step 1: Apply replacements**

- `README.md` line 18: `NEXT_PUBLIC_CHAIN_NAME=Tensorium mainnet pool (tensorium-mainnet-candidate-0)` → `NEXT_PUBLIC_CHAIN_NAME=Tensorium mainnet pool`.
- `README.md` line 36: reword `That VPS is the temporary mainnet-candidate host until a` → `That VPS is the mainnet host` (drop the "candidate/temporary" framing; keep the rest of the sentence sensible — read the surrounding lines and tidy).
- `app/page.tsx` line 56: `"Tensorium mainnet pool (tensorium-mainnet-candidate-0)"` → `"Tensorium mainnet pool"`.
- `app/blocks/page.tsx` line 16: `"Tensorium mainnet pool (tensorium-mainnet-candidate-0)"` → `"Tensorium mainnet pool"`.

- [ ] **Step 2: Verify no residue + typecheck/build if quick**

Run:
```bash
cd /root/.openclaw/workspace/tensorium-pool-website
grep -rnE 'mainnet-candidate' README.md app/ 2>/dev/null
```
Expected: no output. (A full `next build` is optional; these are string-literal edits.)

- [ ] **Step 3: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-pool-website
git add README.md app/page.tsx app/blocks/page.tsx
git commit -m "chore: drop mainnet-candidate label from pool chain name"
```

---

## Task 10: Wallet-extension — test/submission fixtures

Runtime code is already chain-agnostic; only mock fixtures carry the stale id.

**Files:**
- Modify: `/root/.openclaw/workspace/tensorium-wallet-extension/src/__tests__/rpc.test.ts` (lines 12, 14), `/root/.openclaw/workspace/tensorium-wallet-extension/submission/cws-preview.html` (lines 82, 96, 108, 109, 143)

- [ ] **Step 1: Replace every `tensorium-mainnet-candidate-0` → `tensorium-mainnet`**

Run: `cd /root/.openclaw/workspace/tensorium-wallet-extension && sed -i 's/tensorium-mainnet-candidate-0/tensorium-mainnet/g' src/__tests__/rpc.test.ts submission/cws-preview.html`

- [ ] **Step 2: Run the test suite + verify no residue**

Run:
```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
npm test
grep -rnE 'mainnet-candidate' src/ submission/ 2>/dev/null
```
Expected: vitest passes; grep prints nothing.

- [ ] **Step 3: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-wallet-extension
git add src/__tests__/rpc.test.ts submission/cws-preview.html
git commit -m "chore(test): update mock chain id to tensorium-mainnet"
```

---

## Final verification (after all tasks)

- [ ] **Core**: `cd /root/.openclaw/workspace/tensorium-core && cargo test 2>&1 | tail -5` → all pass; `grep -rniE 'mainnet-candidate|TENSORIUM_MC_|mc_state_path' crates/ install.sh` → no output.
- [ ] **All consumers**: from `/root/.openclaw/workspace`, `grep -rlE 'mainnet-candidate-0|mc-rpc\.tensoriumlabs' tensorium-bridge-relayer tensorium-explorer tensorium-sdk-js tensorium-sdk-py tensorium-pool-website tensorium-wallet-extension --exclude-dir=node_modules --exclude-dir=.git` → no output.
- [ ] Each repo has its Phase C commit(s); none pushed yet (pushing + deploy is Phase D).
