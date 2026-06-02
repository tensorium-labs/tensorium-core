# Public RPC Hardening Runbook

Status: Phase 10D operational runbook
Last updated: 2026-06-02

> **Canonical RPC URLs and chain identifiers:** see `../integrations/CANONICAL_ASSET_METADATA.md`.

This runbook turns the current public RPC posture into an operator checklist.

## Scope

Applies to:

- primary public RPC host
- backup seed node if promoted to public RPC
- explorer host when it depends on the same local RPC

Does not apply to:

- founder cold wallet handling
- pool payout hot wallet procedure
- bridge signer custody

## Required Role Split

Minimum service ownership boundary:

- node host:
  - `tensorium-node` RPC
  - `tensorium-node` P2P
  - RocksDB chain state
- explorer host/process:
  - read-only RPC client
  - explorer web/API
  - persisted `txindex.json`
- pool host/process:
  - pool API / miner ingress
  - payout scheduler
  - payout hot wallet only

Hard rule:

- public RPC hosts must not hold founder cold-wallet keys
- public RPC hosts must not hold treasury reserve keys

## Public RPC Baseline

Required baseline before exposing RPC publicly:

1. Keep node RPC bound to `127.0.0.1`.
2. Put nginx in front of it.
3. Allow only `GET`, `POST`, and `OPTIONS`.
4. Enforce request rate limit and connection cap per client IP.
5. Keep `tensorium-node` single-purpose: RPC and P2P only.
6. Monitor disk growth for RocksDB and log growth for nginx/journald.

Reference template:

- `templates/nginx-public-rpc.conf`

## Alert Thresholds

Treat these as page-worthy operator thresholds:

| Signal | Warning | Critical | Operator meaning |
| --- | --- | --- | --- |
| RPC `/health` | 1 failed check | 3 consecutive failed checks | local node or proxy unhealthy |
| Height advance | no new block for 10 min | no new block for 20 min | possible chain stall or peer isolation |
| Explorer divergence | explorer height lags RPC by 2 blocks | explorer height lags RPC by 5+ blocks for 10 min | indexer stuck or replay loop |
| Disk usage | `>= 80%` | `>= 90%` | RocksDB/log pressure |
| Backup age | latest backup older than 26h | latest backup older than 36h | backup job failed or retention broke |
| Public RPC latency | p95 `> 1s` | p95 `> 3s` sustained 10 min | abuse, disk I/O saturation, or proxy issue |

## Smoke Checks

Run after deploy, restart, or failover:

```bash
curl -fsS http://127.0.0.1:33332/health
curl -fsS http://127.0.0.1:33332/getblockcount
curl -fsS https://mc-rpc.tensoriumlabs.com/health
curl -fsS https://mc-rpc.tensoriumlabs.com/getblockcount
ss -ltnp | egrep '33332|33333|443'
systemctl is-active tensorium-mc-rpc tensorium-mc-p2p nginx
```

If explorer is on the same host or same rollout window:

```bash
curl -fsS https://explorer.tensoriumlabs.com/api/indexer/status
```

## Incident Checklists

### Chain Stall

Symptoms:

- `/getblockcount` stops moving
- both public RPC and local RPC show the same frozen height

First response:

1. Compare local height against backup seed and explorer.
2. Check `systemctl status tensorium-mc-rpc tensorium-mc-p2p`.
3. Inspect recent logs: `journalctl -u tensorium-mc-rpc -u tensorium-mc-p2p -n 200 --no-pager`.
4. Check peer reachability on `33333/tcp`.
5. If local-only issue, restart `tensorium-mc-p2p` first, then `tensorium-mc-rpc`.
6. If chain-wide issue, do not restore from backup blindly; capture logs and compare cumulative work on both seeds first.

### Peer Isolation

Symptoms:

- local node height stops while another seed keeps advancing
- P2P port open but sync does not progress

First response:

1. Run local `curl -fsS http://127.0.0.1:33332/getblockcount`.
2. Cross-check with backup seed or primary seed height.
3. Verify firewall/UFW changes and recent bans.
4. Review `banlist.json` and inbound/outbound peer logs.
5. Run one manual sync against the healthy seed.
6. If sync succeeds, restart only P2P service and keep RPC up.

### Explorer Divergence

Symptoms:

- explorer height behind RPC
- `/api/tx/:txid` missing recently accepted transactions
- `/api/indexer/status` stops advancing

First response:

1. Compare explorer reported height against local RPC `/getblockcount`.
2. Confirm `txindex.json` is writable and not full-disk blocked.
3. Restart explorer process only.
4. Watch `/api/indexer/status` for `loaded_from_disk` and advancing tip.
5. If divergence persists, rebuild explorer index from RPC and keep node untouched.

### RPC Abuse / Proxy Saturation

Symptoms:

- nginx 429 spike
- high p95 latency on `/getblock` or `/getblocktemplate`
- local RPC healthy but public RPC timing out

First response:

1. Confirm local RPC answers on `127.0.0.1`.
2. Inspect nginx access/error logs for abusive IPs or endpoint patterns.
3. Tighten `limit_req` / `limit_conn` temporarily if needed.
4. Block the worst source IPs at nginx or firewall.
5. Keep node RPC localhost-only; do not bypass nginx to “fix” public access.

### Backup Failure

Symptoms:

- latest archive missing
- latest archive older than retention target

First response:

1. Run the backup script manually.
2. Check free disk space in backup destination.
3. Check cron / scheduler logs and script exit status.
4. Confirm RocksDB directory paths still match current deployment.
5. If backup archive is created manually, leave automation disabled only long enough to fix the root cause.

### Disk Pressure

Symptoms:

- disk warning/critical threshold crossed
- RocksDB compaction or nginx logs growing too fast

First response:

1. Measure usage: `df -h`, `du -sh` on state, logs, backups.
2. Rotate or prune old backups according to retention.
3. Confirm journald/nginx log retention is still active.
4. Expand disk before usage crosses `90%`.
5. Do not delete live RocksDB files while services are running.

## Failover Rule

If the primary public RPC host is degraded but chain state is healthy:

1. keep the backup seed node on localhost RPC first
2. only then enable nginx public RPC on backup using `templates/nginx-public-rpc.conf`
3. update DNS / client fallback after HTTPS checks pass

## Validation Record

Phase 10D kickoff changes completed on 2026-06-02:

- installer systemd RPC bind changed back to `127.0.0.1`
- nginx public RPC template added to repo
- readiness docs updated to reference this runbook
