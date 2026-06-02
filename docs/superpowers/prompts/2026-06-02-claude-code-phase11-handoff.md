# Claude Code Handoff Prompt — Post Phase 10

Use this prompt in Claude Code to continue Tensorium after Phase 10 closure.

---

You are continuing work on the `tensorium-core` and `tensorium-explorer` repositories after **Phase 10 has been completed on 2026-06-02**.

First, read these files to rebuild exact context:

1. `MAINNET_READINESS.md`
2. `docs/superpowers/plans/2026-06-02-post-launch-mainnet-phases.md`
3. `CANONICAL_ASSET_METADATA.md`
4. `PUBLIC_RPC_HARDENING_RUNBOOK.md`
5. `RESTORE_RUNBOOK.md`
6. `POOL_PAYOUT_RUNBOOK.md`
7. `CEX_LISTING_PACKAGE.md`

Then inspect current git status before changing anything.

Important context:

- Mainnet is already live.
- RocksDB migration is complete in node/core/wallet paths.
- Explorer no longer rescans full chain per address/tx request and now persists `txindex.json`.
- Backup/restore runbook exists and has already been drill-tested.
- Pool treasury vs payout hot wallet separation is documented and partially surfaced in runtime metadata.
- Public RPC posture is localhost-only node RPC behind nginx, with a committed hardening template and incident runbook.
- Canonical integrator/listing metadata now lives in `CANONICAL_ASSET_METADATA.md`.

Your task:

1. Audit the repo for any remaining operator-facing or integrator-facing references that can drift from `CANONICAL_ASSET_METADATA.md`.
2. Normalize the remaining active docs so they reference the canonical metadata file instead of duplicating critical values unnecessarily.
3. Identify the best **Phase 11** candidate and implement the first concrete slice of it, not just a plan.

Constraints:

- Do not revert unrelated existing worktree changes.
- Prefer fixing real operational/documentation drift over adding speculative architecture.
- Keep changes production-minded and mainnet-relevant.
- Run verification commands for whatever you touch and report exact results.

Success criteria:

- Phase 10 remains cleanly closed.
- No obvious active metadata drift remains in main operator/integrator docs.
- A credible Phase 11 next step is started with code and/or docs, plus verification.

When you finish, provide:

- what you changed
- what you verified
- remaining risks or follow-ups

---

Suggested Phase 11 direction if no stronger issue is found:

- operator-facing observability/alerting automation
- explorer reorg behavior test coverage
- public API consumer docs / SLA cleanup
- bridge relayer productionization audit
