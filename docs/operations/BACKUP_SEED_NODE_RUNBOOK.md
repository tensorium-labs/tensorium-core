# Backup Seed Node Runbook

Status: draft execution runbook for the next Phase 8 infrastructure step.
Last updated: 2026-06-01

This document turns the remaining `backup seed node` blocker into an executable
rollout plan.

## Goal

Add a second mainnet-candidate seed node on a different VPS/provider without
moving the current DigitalOcean host.

Primary outcomes:

- reduce single-node risk for MC peer bootstrap,
- keep the current DO node as the first active seed,
- keep local Git + `tensorium-labs` GitHub as the source of truth,
- avoid copying founder or treasury private keys onto the backup seed host.

## Recommended Topology

Current host:

- DigitalOcean: primary MC seed node
- roles: `tensorium-mc-rpc`, `tensorium-mc-p2p`, nginx public RPC proxy,
  monitor, backups, docs-related services

New host:

- Vultr or other provider: backup MC seed node
- roles: `tensorium-mc-rpc`, `tensorium-mc-p2p`, optional nginx HTTPS proxy,
  monitoring
- no founder wallet
- no treasury wallet
- no pool payout hot wallet

## Minimum Host Spec

- 2 vCPU minimum, 4 vCPU recommended
- 4 GB RAM minimum, 8 GB recommended
- 50 GB SSD minimum
- Ubuntu 24.04 LTS recommended
- public IPv4

## Ports

Open on backup seed node:

- `22/tcp` SSH
- `80/tcp` HTTP for certbot/nginx
- `443/tcp` HTTPS if public RPC will be exposed from this host
- `33333/tcp` MC P2P

Keep node RPC bound to localhost:

- `127.0.0.1:33332`

## Source of Truth

Always use this order:

1. edit locally
2. run checks locally
3. push to `tensorium-labs`
4. deploy/sync host from GitHub
5. run smoke checks

Do not hand-edit production binaries or tracked code on the VPS and let it
drift away from GitHub.

## Preflight

Before touching the new VPS:

1. confirm the local `tensorium-core` checkout is clean
2. confirm `cargo test --workspace` passes locally
3. confirm the current primary seed node is healthy
4. confirm the new VPS has SSH access, disk, and basic firewall rules

## Install on Backup Seed Host

Clone repo:

```bash
git clone https://github.com/tensorium-labs/tensorium-core.git /root/tensorium-core
cd /root/tensorium-core
cargo build --release -p tensorium-node
install -m 0755 target/release/tensorium-node /usr/local/bin/tensorium-node
```

Prepare runtime folders:

```bash
mkdir -p /root/mc
```

Install or provide an equivalent local backup helper on the host:

```bash
install -m 0755 /path/to/your/local/tensorium-backup.sh /usr/local/bin/tensorium-backup.sh
```

Initialize MC genesis state:

```bash
TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json \
  /usr/local/bin/tensorium-node mainnet-candidate init
```

After init, expect RocksDB state at:

```bash
ls -lah /root/mc/tensorium-mc-state.db
```

Initial sync from primary seed:

```bash
TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json \
TENSORIUM_MC_PEERS=seed.tensoriumlabs.com:33333 \
  /usr/local/bin/tensorium-node mainnet-candidate sync
```

## Systemd Units

`/etc/systemd/system/tensorium-mc-rpc.service`

```ini
[Unit]
Description=Tensorium Mainnet Candidate RPC
After=network.target
Wants=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/mc
Environment=TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json
Environment=TENSORIUM_MC_MEMPOOL=/root/mc/tensorium-mc-mempool.json
Environment=TENSORIUM_MC_BANS=/root/mc/tensorium-mc-banlist.json
Environment=TENSORIUM_NO_DEFAULT_SEEDS=1
ExecStart=/usr/local/bin/tensorium-node mainnet-candidate rpc 127.0.0.1:33332
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

`/etc/systemd/system/tensorium-mc-p2p.service`

```ini
[Unit]
Description=Tensorium Mainnet Candidate P2P
After=network.target tensorium-mc-rpc.service
Wants=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/mc
Environment=TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json
Environment=TENSORIUM_MC_MEMPOOL=/root/mc/tensorium-mc-mempool.json
Environment=TENSORIUM_MC_BANS=/root/mc/tensorium-mc-banlist.json
Environment=TENSORIUM_NO_DEFAULT_SEEDS=1
ExecStart=/usr/local/bin/tensorium-node mainnet-candidate p2p-listen 0.0.0.0:33333
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Enable services:

```bash
systemctl daemon-reload
systemctl enable --now tensorium-mc-rpc tensorium-mc-p2p
```

## Optional Public RPC on Backup Node

Only do this if the backup host will also serve public RPC traffic.

Rules:

- keep node RPC on `127.0.0.1:33332`
- put nginx in front
- keep request limiting enabled

Suggested public hostname:

- `mc-rpc-backup.tensoriumlabs.com`

Minimum nginx behavior:

- proxy to `127.0.0.1:33332`
- TLS via certbot
- `limit_req` similar to the current primary host

## Monitoring

Provide equivalent local monitoring helpers:

- `/usr/local/bin/tensorium-monitor.sh`
- `/usr/local/bin/tensorium-soak.sh`

Suggested cron:

```cron
*/10 * * * * /usr/local/bin/tensorium-monitor.sh
0 * * * * /usr/local/bin/tensorium-soak.sh
```

Minimum monitor checks:

- local MC RPC `/health`
- P2P port `33333`
- disk usage
- optional HTTPS endpoint if public RPC is enabled

## Smoke Checks

Run after deployment:

```bash
systemctl is-active tensorium-mc-rpc tensorium-mc-p2p
curl -fsS http://127.0.0.1:33332/health
curl -fsS http://127.0.0.1:33332/getblockcount
ss -ltnp | egrep '33332|33333'
```

Confirm sync status:

```bash
TENSORIUM_MC_STATE=/root/mc/tensorium-mc-state.json \
  /usr/local/bin/tensorium-node mainnet-candidate status
```

Backup note:

```bash
tar -czf /root/backups/tensorium-mc-$(date +%F).tgz \
  /root/mc/tensorium-mc-state.db \
  /root/mc/tensorium-mc-mempool.json \
  /root/mc/tensorium-mc-banlist.json \
  /root/mc/*.json.migrated
```

Cross-check against primary seed:

```bash
curl -fsS https://mc-rpc.tensoriumlabs.com/getblockcount
curl -fsS http://127.0.0.1:33332/getblockcount
```

Target result:

- same chain id
- same height
- same genesis/tip hash when height is unchanged

## What Must Not Be Copied

Do not place these on the backup seed host:

- founder cold wallet
- pool treasury private key
- pool payout hot wallet unless the host is explicitly assigned pool duties
- mailbox credentials unless needed for that host

## Rollout Completion Criteria

Mark `backup seed node` complete when all of these are true:

1. second VPS is on a different provider or region
2. `tensorium-mc-rpc` and `tensorium-mc-p2p` are enabled on that host
3. node has synced from the primary seed successfully
4. smoke checks are green
5. monitoring is installed

## Next Step After Completion

After the backup seed node is live:

1. keep soak test notes running in background
2. decide whether public MC RPC should remain on the primary host only or be
   split across hosts
3. make the final public launch decision
