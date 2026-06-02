# Restore Runbook

> **Canonical service URLs, chain IDs, and RPC endpoints:** see `../integrations/CANONICAL_ASSET_METADATA.md`.
> If any value here disagrees with that file, update this file.

This runbook describes how to restore a Tensorium node from a backup created by `tensorium-backup.sh`.

## Scope

Applies to:

- testnet node data
- mainnet-candidate node data
- explorer instances that depend on node RPC after the node state is restored

Does not cover:

- founder cold-wallet recovery
- pool payout hot-wallet incident recovery
- bridge signer recovery

## Backup Artifact Layout

Expected archive contents:

- `state.db/` or `tensorium-mc-state.db/`
- `mempool.json` or `tensorium-mc-mempool.json`
- `banlist.json` or `tensorium-mc-banlist.json`
- optional `*.json.migrated`

## Before You Start

1. Confirm which host you are restoring:
   - testnet
   - mainnet-candidate
   - backup seed
2. Confirm the target data directory:
   - example testnet: `/root/.tensorium`
   - example mainnet-candidate: `/root/mc`
3. Confirm the backup archive timestamp you want to restore.
4. Make sure you have shell access and sudo privileges.

## Service Stop Order

Stop anything that reads or mutates chain state before touching files:

```bash
systemctl stop tensorium-explorer || true
systemctl stop tensorium-pool || true
systemctl stop tensorium-mc-p2p || true
systemctl stop tensorium-mc-rpc || true
systemctl stop tensorium-p2p || true
systemctl stop tensorium-rpc || true
```

If your deployment uses different unit names, adapt them before proceeding.

## Safety Snapshot Of Current Failed State

Before overwriting anything, preserve the current files:

```bash
mkdir -p /root/restore-preflight
tar -czf /root/restore-preflight/pre-restore-$(date -u +%F-%H%M%S).tgz \
  /root/.tensorium \
  /root/mc \
  2>/dev/null || true
```

## Restore Procedure

### Testnet Example

```bash
mkdir -p /root/.tensorium
tar -xzf /root/backups/tensorium-backup-YYYY-MM-DD-HHMMSS.tgz -C /
chown -R root:root /root/.tensorium
find /root/.tensorium -type d -name '*.db' -exec chmod 700 {} \;
find /root/.tensorium -type f -exec chmod 600 {} \;
```

### Mainnet-Candidate Example

```bash
mkdir -p /root/mc
tar -xzf /root/backups/tensorium-backup-YYYY-MM-DD-HHMMSS.tgz -C /
chown -R root:root /root/mc
find /root/mc -type d -name '*.db' -exec chmod 700 {} \;
find /root/mc -type f -exec chmod 600 {} \;
```

## When To Use `*.json.migrated`

Use the RocksDB directory first.

Only fall back to `*.json.migrated` when:

- the restored RocksDB directory is missing
- RocksDB files are corrupt or incomplete
- the node cannot reopen the DB after restore

Fallback flow:

1. Move the broken `*.db/` out of the way.
2. Rename `*.json.migrated` back to `*.json`.
3. Start the node with the same `TENSORIUM_STATE` or `TENSORIUM_MC_STATE`.
4. Let the node auto-migrate the JSON file back into a fresh `*.db/`.

Example:

```bash
mv /root/mc/tensorium-mc-state.db /root/mc/tensorium-mc-state.db.bad
cp /root/mc/tensorium-mc-state.json.migrated /root/mc/tensorium-mc-state.json
TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json /usr/local/bin/tensorium-node mainnet-candidate status
```

## Service Start Order

Bring chain services up first, then readers:

```bash
systemctl start tensorium-rpc || true
systemctl start tensorium-p2p || true
systemctl start tensorium-mc-rpc || true
systemctl start tensorium-mc-p2p || true
sleep 3
systemctl start tensorium-explorer || true
systemctl start tensorium-pool || true
```

## Verification Checklist

### Testnet

```bash
curl -fsS http://127.0.0.1:23332/health
curl -fsS http://127.0.0.1:23332/getblockcount
curl -fsS http://127.0.0.1:23332/getdifficulty
```

### Mainnet-Candidate

```bash
curl -fsS http://127.0.0.1:33332/health
curl -fsS http://127.0.0.1:33332/getblockcount
curl -fsS https://mc-rpc.tensoriumlabs.com/getblockcount
```

### Explorer

```bash
curl -fsS http://127.0.0.1:3000/api/indexer/status
```

Expected checks:

- RPC responds successfully
- local height is sane
- explorer indexer status responds
- local mainnet-candidate height is close to or equal to the public reference

## Failure Branches

### Node starts but height is wrong

- verify you restored the expected backup archive
- compare against public seed height
- if badly behind, run sync after restore

### Node fails to open RocksDB

- check file ownership
- check available disk space
- inspect `journalctl -u tensorium-rpc -n 100` or MC unit equivalent
- if DB is unrecoverable, retry with `*.json.migrated`

### Explorer returns stale or empty address/tx data

- restart explorer after node is healthy
- check `/api/indexer/status`
- if needed, fully restart explorer so it rebuilds the in-memory index from RPC

## Post-Restore Notes

- Record:
  - archive used
  - restore start/end time
  - final chain height
  - any manual deviations
- If the incident exposed a gap in the runbook, update this file immediately after recovery.

## Status

Documented on 2026-06-02 after RocksDB migration and backup-script introduction.

## Restore Drill Log — 2026-06-02

Temp drill environment:

- base dir: `/tmp/tmp.65EDyf3YOO`
- restored archive: `tensorium-backup-2026-06-02-085342.tgz`

Measured results:

- init + mine one block: `53.766s`
- backup creation: `0.034s`
- restore extract: `0.018s`
- chain height before restore: `1`
- chain height after restore: `1`

Post-restore checks passed:

- `tensorium-node status`
- RPC `/health`
- RPC `/getblockcount`
- explorer `/api/indexer/status`
- explorer `/api/tx/<restored-txid>`

Observed gotchas:

- Most elapsed time in the drill came from `cargo run`, not from backup or restore itself.
- Restoring as root preserved a usable RocksDB directory immediately, but the explicit ownership/permission fixup steps should remain in the runbook for real operators and mixed-user deployments.
- Explorer recovered cleanly after restart because its in-memory index rebuilt from restored RPC state.
- This drill validated the primary `*.db/` path only; `*.json.migrated` fallback was documented but not exercised here.

Recommended next step:

- exercise the documented `*.json.migrated` fallback path once in a temp environment, or proceed to Phase 10B if primary RocksDB recovery is considered sufficient for current operations.
