You are continuing Tensorium after the mainnet-only cutover on 2026-06-02.

Read these files first:

1. `README.md`
2. `MAINNET_READINESS.md`
3. `CHANGELOG.md`
4. `docs/integrations/CANONICAL_ASSET_METADATA.md`
5. `docs/operations/PUBLIC_RPC_POSTURE.md`
6. `docs/superpowers/prompts/2026-06-02-claude-code-mainnet-only-handoff.md`

Then inspect current git status before changing anything.

Current verified state:

- Mainnet is live at `tensorium-mainnet-candidate-0`.
- Public explorer is live and branded correctly at `https://explorer.tensoriumlabs.com`.
- Public RPC is `https://mc-rpc.tensoriumlabs.com`.
- VPS `157.230.44.162` is now mainnet-only operationally:
  - `tensorium-mc-rpc.service` active on `33332`
  - `tensorium-mc-p2p.service` active on `33333`
  - legacy `tensorium-rpc.service`, `tensorium-p2p.service`, and `tensorium-automine.service` are disabled/inactive
- Explorer frontend asset mismatch was fixed and `Cache-Control: no-store` was added for HTML/API/utils.js.
- GPU miner (`tensorium-miner`) on the rental RTX 5090 host is running via `tmux` and has already produced a visible block reward to:
  - `txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck`

Operational conclusion:

- `tensorium-miner` is now the real mainnet mining path.
- `txmminer` CPU should no longer be presented as an active production mining path.
- However, do not delete `txmminer` blindly unless you can prove it is safe to retire or clearly reduce it to dev/diagnostic scope.

Your task:

1. Audit the repo for all places where `txmminer` CPU is still presented as an operator-facing or production mining path.
2. Reposition `txmminer` to `dev/diagnostic/fallback` scope only.
3. Make `tensorium-miner` the explicit default mining path for:
   - solo mining docs
   - pool/operator docs
   - install/runtime guidance
   - user-facing examples where production/mainnet is implied
4. Identify whether `txmminer` CPU is still needed as a maintained binary:
   - if yes, justify its retained scope clearly
   - if not, prepare the safest first slice toward retirement without breaking tooling/tests
5. Run verification for every file you touch and report exact results.

Constraints:

- Do not revert unrelated worktree changes.
- Do not remove binaries or commands casually if tests/dev workflows still rely on them.
- Prefer reducing operator ambiguity over aggressive deletion.
- If you touch live operational assumptions, call that out explicitly.

Success criteria:

- Mainnet mining guidance is GPU-only by default.
- `txmminer` CPU is no longer described as the normal production miner.
- Any retained CPU-miner scope is explicit and narrow.
- No live mainnet path is regressed.

When you finish, provide:

- what you changed
- what you verified
- what you decided about `txmminer`
- what still remains
