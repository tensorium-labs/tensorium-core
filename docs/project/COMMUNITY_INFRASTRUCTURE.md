# Community Infrastructure

Updated: `2026-06-01`

This document prepares the community operations layer for Tensorium before public opening.

It does **not** mean every public channel is already live. It means the structure, rules, and launch sequence are ready so the team can open channels without improvising under pressure.

## Current Goal

Prepare the final Phase 9E community infrastructure for:

- public Telegram access
- Discord server launch
- X/Twitter milestone announcements
- bug report and support routing
- mining/community bootstrap events

## Public Surface Map

Use these surfaces with clear roles:

| Surface | Purpose | Status |
|---|---|---|
| `tensoriumlabs.com` | homepage / canonical links | live |
| `docs.tensoriumlabs.com` | setup, RPC docs, onboarding | live |
| `explorer.tensoriumlabs.com` | chain visibility | live |
| Telegram | fast support, announcements, community chat | invite exists, public opening pending |
| Discord | structured mining/dev/support server | pending creation |
| X/Twitter | milestone broadcast, public proof of progress | pending structured rollout |
| GitHub Issues | bugs and feature requests | live |

## Telegram Setup

Recommended final posture:

- one main public Telegram group for community chat
- one pinned onboarding message
- one pinned safety message
- one linked announcement-only channel later if message volume grows

Minimum pinned items:

1. Welcome + what Tensorium is
2. Official links only:
   - website
   - docs
   - explorer
   - faucet
   - GitHub
3. Safety rules:
   - no one from the team will ask for private keys
   - no OTC trade guarantee by admins
   - verify links from pinned message only
4. Support routing:
   - mining help -> Telegram / Discord mining-help
   - bugs -> GitHub Issues
   - wallet/RPC docs -> docs site

## Discord Server Layout

Recommended categories and channels:

### 1. Info

- `#welcome`
- `#announcements`
- `#official-links`
- `#rules`
- `#faq`

### 2. Community

- `#general`
- `#introductions`
- `#memes`

### 3. Mining

- `#mining-help`
- `#pool-talk`
- `#gpu-benchmarks`
- `#share-your-rig`

### 4. Development

- `#dev-chat`
- `#rpc-api`
- `#wallet-dev`
- `#explorer-feedback`

### 5. Governance / Reports

- `#bug-reports`
- `#feature-requests`
- `#status-and-incidents`

### 6. Staff Private

- `#mod-room`
- `#incident-triage`
- `#content-calendar`

Recommended initial roles:

- `Admin`
- `Moderator`
- `Core Dev`
- `Contributor`
- `Miner`
- `Community`

## Support Routing Rules

Do not let support disappear into chat noise.

- protocol/client bugs -> GitHub Issues
- docs gaps -> GitHub Issues or docs PR
- exchange/listing inquiries -> private admin contact, then log summary in internal notes
- mining setup questions -> Telegram / Discord mining channels
- incident reports -> `#status-and-incidents` + Telegram pinned incident update

## Moderation Policy

Minimum enforcement rules:

- zero tolerance for fake admin support and private-key phishing
- no token price manipulation spam
- no impersonation of team members
- no malicious binaries or wallet downloads
- OTC trading allowed only at user risk; moderators do not escrow

Escalation ladder:

1. warn
2. mute / timeout
3. ban
4. if scam/phishing: delete + ban immediately

## Launch Sequence

Open channels in this order:

1. confirm pinned messages and official links
2. prepare first public announcement
3. open Telegram public access
4. create Discord with base structure
5. post Discord invite in Telegram and website/docs
6. publish X/Twitter milestone thread
7. run first bootstrap community event

## Content Pack To Prepare Before Public Opening

- launch announcement
- mining quickstart post
- faucet quickstart post
- wallet safety warning
- bug report instructions
- explorer/docs links

Use [templates/community-launch-announcement.md](templates/community-launch-announcement.md) as the starting point.

## First 7-Day Community Plan

Day 1:

- announce public channels
- pin onboarding links
- answer setup questions aggressively

Day 2-3:

- share mining screenshots / early stats
- collect onboarding friction

Day 4-5:

- push docs fixes based on repeated questions
- highlight explorer, faucet, SDKs

Day 6-7:

- run simple event:
  - best mining rig post
  - first wallet setup challenge
  - bug bounty mini-prompt

## Definition Of Ready

Community infrastructure is considered ready when:

- Telegram pinned messages are written
- Discord structure is defined
- moderation rules are written
- official links list is fixed
- first launch announcement draft exists
- support routing is explicit

That is the bar for "community infra ready".
