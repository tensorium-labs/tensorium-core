# Documentation Layout

The repository keeps top-level project entrypoints lean and stores supporting documentation under `docs/`.

## Directories

- `docs/operations/` — operator runbooks, recovery procedures, payout flows, and public RPC posture
- `docs/integrations/` — canonical metadata packets and exchange/listing handoff material
- `docs/bridge/phase9a/` — bridge architecture, custody, policy, and execution artifacts from Phase 9A
- `docs/project/` — project-wide references such as risk disclosure, migration notes, and known issues
- `docs/superpowers/` — internal execution plans, prompts, and worker-oriented artifacts

## Root Kept Intentionally Small

These stay in the repository root because they are standard entrypoints for GitHub visitors and tooling:

- `README.md`
- `MAINNET_READINESS.md`
- `CHANGELOG.md`
- `Cargo.toml`
- `install.sh`
