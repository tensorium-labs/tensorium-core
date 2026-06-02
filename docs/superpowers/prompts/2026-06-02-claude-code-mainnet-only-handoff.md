You are continuing work on `tensorium-core` and related Tensorium repos after the mainnet-only cutover work completed on 2026-06-02.

Read these files first:

1. `README.md`
2. `MAINNET_READINESS.md`
3. `CHANGELOG.md`
4. `docs/integrations/CANONICAL_ASSET_METADATA.md`
5. `docs/operations/PUBLIC_RPC_POSTURE.md`
6. `docs/superpowers/prompts/2026-06-02-claude-code-phase11-handoff.md`

Then inspect current git status before changing anything.

Current verified state:

- Mainnet chain is live at `tensorium-mainnet-candidate-0`.
- Public explorer is live at `https://explorer.tensoriumlabs.com` and now shows mainnet branding.
- Public RPC is `https://mc-rpc.tensoriumlabs.com`.
- Seed is `seed.tensoriumlabs.com:33333`.
- VPS `157.230.44.162` has already been switched to mainnet-only operations:
  - `tensorium-mc-rpc.service` active on `33332`
  - `tensorium-mc-p2p.service` active on `33333`
  - legacy `tensorium-rpc.service`, `tensorium-p2p.service`, and `tensorium-automine.service` have been disabled/stopped
- Explorer frontend patch has already been deployed to the VPS via `pm2 restart tensorium-explorer`.
- CUDA miner patch for 122-byte mainnet headers is already applied in `tools/txmminer-cuda`.
- GPU miner on the rental RTX 5090 host was started successfully and produced a visible block reward to:
  - `txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck`

What has already been cleaned:

- Installer, monitor, runtime defaults, README, known issues, risk docs, and many ops/integration docs were switched to mainnet-first.
- Explorer repo local code was switched from `23332`/`testnet` to `33332`/`mainnet`.
- Large parts of `docs/superpowers/*` were cleaned so they no longer present testnet as the default active path.

What still remains:

- Some code-level testnet support still exists intentionally in `crates/tensorium-core/src/chain.rs` and related tests.
- There may still be historical or embedded references in legacy planning/spec docs.
- There may be opportunities to simplify or retire testnet-specific branches, names, comments, and ignored files without breaking dev/test scaffolding.

Your task:

1. Audit the repo for remaining testnet-specific references and separate them into:
   - safe-to-remove legacy wording/examples
   - keep-but-relabel dev/test scaffolding
   - risky consensus/runtime removals that should not be changed casually
2. Implement the next safe cleanup slice so the repo becomes more clearly mainnet-only without breaking the remaining development/test harness.
3. Update docs where needed so the repository no longer presents testnet as an active production path.
4. Run concrete verification for every file you touch and report exact results.

Constraints:

- Do not revert unrelated worktree changes.
- Do not casually remove consensus/test scaffolding unless you can prove it is unused and safe.
- Prefer operational clarity and production correctness over aggressive deletion.
- If a change affects live operational assumptions, state that explicitly.

Success criteria:

- The repository is even more clearly mainnet-only to operators and integrators.
- Remaining testnet references are either removed or consciously justified.
- No live mainnet path is regressed.

When you finish, provide:

- what you changed
- what you verified
- what still remains and why
