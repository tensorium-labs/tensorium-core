# Public RPC Posture

Status: current operational decision for Tensorium public RPC exposure.
Last updated: 2026-06-02

This document defines which host serves public RPC now, which host stays
seed-only, and what conditions justify activating public RPC on the backup host.

## Current Decision

For the current mainnet stage:

- DigitalOcean remains the primary public RPC host.
- Vultr `txm-mc-seed-1` remains a backup seed node, not a public RPC host yet.
- Node RPC stays bound to localhost on both hosts.
- Any public RPC exposure must sit behind nginx with rate limiting.

## Current Host Roles

### DigitalOcean `157.230.44.162`

Roles:

- primary MC seed node
- public MC RPC at `https://mc-rpc.tensoriumlabs.com`
- docs/web/pool/faucet/status-related services

Reason:

- already has working nginx + TLS + rate limiting
- already serves the Chrome extension endpoints
- keeps public traffic concentrated on the host that is already configured for it

### Vultr `139.180.137.144`

Roles:

- backup MC seed node
- private local MC RPC at `127.0.0.1:33332`
- public MC P2P at `0.0.0.0:33333`
- local monitoring + soak logging

Reason:

- backup seed redundancy is the first requirement
- public RPC is optional on this host until traffic or failover needs justify it

## Why Not Expose Backup Public RPC Immediately

Current MC chain height is still minimal and public traffic is low.

Keeping Vultr seed-only for now has advantages:

- fewer moving parts before launch
- no extra DNS/certbot work yet
- simpler debugging if a sync or peer issue appears
- clearer separation between primary public ingress and backup network redundancy

## Activation Conditions For Backup Public RPC

Enable public RPC on Vultr only when at least one of these is true:

1. primary public RPC latency becomes a real user issue
2. Chrome extension or wallet traffic grows enough to justify splitting ingress
3. failover coverage is needed before launch
4. launch review decides MC RPC should be served from two hosts

## Activation Plan

If backup public RPC is enabled later:

1. install nginx + certbot on Vultr
2. keep `tensorium-node` bound to `127.0.0.1:33332`
3. create a new hostname such as `mc-rpc-backup.tensoriumlabs.com`
4. proxy nginx `443 -> 127.0.0.1:33332`
5. apply the same `limit_req` posture used on DigitalOcean via `templates/nginx-public-rpc.conf`
6. add CORS headers equivalent to current public RPC
7. add HTTPS endpoint monitoring to the backup host
8. decide whether clients use:
   - manual fallback,
   - DNS rotation,
   - active/standby failover

Operational companion artifacts:

- `PUBLIC_RPC_HARDENING_RUNBOOK.md`
- `templates/nginx-public-rpc.conf`

## Recommended Near-Term Posture

Near-term recommendation:

- keep DO as primary public RPC
- keep Vultr as backup seed only
- use Vultr for public RPC later only if traffic, latency, or failover needs justify it

This keeps launch-chain infrastructure simple while preserving an upgrade path.
