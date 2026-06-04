# Discord Bot Upgrade — Design Spec
**Date:** 2026-06-04  
**Status:** Approved  
**Scope:** Upgrade existing Tensorium Discord auto-role bot to full-featured mainnet bot with slash commands, multi-language channels, and updated content.

---

## Context

Tensorium mainnet is LIVE as of 2026-06-02. The current Discord bot (`txm_autorole_bot.py`) only auto-assigns roles on member join and sends a welcome DM. All channel content (rules, faq, announcements) still references "mainnet-candidate soak test" and testnet faucet — both outdated. The Discord server needs a full upgrade to reflect mainnet status.

**Existing assets:**
- Bot token: stored in `/root/txm_autorole_bot.py` (to be moved to `.env`)
- Guild ID: `1511205971173707906`
- Bot user ID: `1511210602054422588`
- Systemd service: `txm-discord-bot.service` on VPS `157.230.44.162`
- RPC endpoint: `https://mc-rpc.tensoriumlabs.com` (MC mainnet, port 33332 proxied via nginx)
- Uniswap V4 pool: wTXM/ETH on OP Mainnet, PoolManager `0x9a13F98Cb987694C9F086b1F5eB990EeA8264Ec3`
- wTXM contract: `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`

---

## Architecture

**Approach:** Single-file Python bot upgrade (Option A). Extend existing discord.py to v2.x with `app_commands` for slash commands. Add a small `price.py` helper for Uniswap price fetching with manual fallback.

```
/root/txm_discord_bot/
├── bot.py                  # Main bot: events + all slash commands
├── price.py                # Price helper: Uniswap OP fetch + manual fallback
├── .env                    # Secrets: TOKEN, GUILD_ID, OP_RPC_URL (not committed)
├── price_manual.json       # Persisted manual price (updated via /setprice)
└── requirements.txt        # discord.py>=2.3, aiohttp, python-dotenv
```

The existing systemd service only needs its `ExecStart` updated to point to the new path. No additional services required.

**Key technical choices:**
- discord.py 2.x `app_commands.CommandTree` for slash commands
- `aiohttp` for async HTTP calls to RPC and Uniswap
- Uniswap price via `eth_call` on OP Mainnet: V4 has no individual pool contract — price queried via `PoolManager.getSlot0(poolKey)` where poolKey encodes (currency0=ETH, currency1=wTXM, fee, tickSpacing, hooks). sqrtPriceX96 → ETH per TXM ratio. ETH/USD from CoinGecko free API → final USD price
- `price_manual.json` stores `{"usd": float, "updated_at": str}` — written by `/setprice`, read as fallback when Uniswap fetch fails

---

## Channel Structure

Full channel rebuild via update script. Old channels replaced with:

```
📋 INFO
  #rules          (read-only)
  #announcements  (read-only)
  #faq            (read-only)

💬 COMMUNITY — ENGLISH
  #general-en
  #introductions
  #off-topic

💬 CHAT — LANGUAGES
  #indonesia  🇮🇩
  #chinese    🇨🇳
  #russian    🇷🇺
  #spanish    🇪🇸
  #french     🇫🇷
  #german     🇩🇪

⛏️ MINING
  #mining-general
  #mining-support
  #pool-mining
  #gpu-benchmarks

🔧 DEVELOPMENT
  #dev-general
  #wallet
  #sdk-api
  #bug-reports

📊 TRADING & ECOSYSTEM
  #price-talk
  #otc-trading
  #bridge-wtxm

🌐 NETWORK
  #node-operators
  #mainnet

🔊 VOICE
  General Talk
  Mining Talk
  Dev Talk
```

**Notes:**
- `#testnet` and `#mainnet-candidate` channels are removed (mainnet is live)
- `#mainnet` replaces both — covers mainnet node ops, chain status, announcements
- Language channels under their own category for discoverability
- All language channels allow free posting (no read-only restriction)

---

## Slash Commands

All commands registered as global guild slash commands.

### `/stats`
- **Description:** Show current mainnet chain stats
- **Data:** RPC call `getblockcount` + `getblocktemplate` (for difficulty) to `https://mc-rpc.tensoriumlabs.com`
- **Output:** Embed with block height, difficulty (bits), estimated network hashrate (derived from diff + avg block time), chain ID
- **Error handling:** If RPC unreachable, show "Node temporarily unreachable, check https://explorer.tensoriumlabs.com"

### `/block`
- **Description:** Show latest block details
- **Data:** RPC `getblockcount` → `getblockhash(height)` → `getblock(hash)`
- **Output:** Embed with height, hash (truncated), timestamp, tx count, miner reward (11.9027 TXM)
- **Error handling:** Same as `/stats`

### `/price`
- **Description:** Show TXM price in USD and ETH
- **Data:** Primary = Uniswap V4 slot0 on OP Mainnet via `OP_RPC_URL`; Secondary = ETH/USD from CoinGecko free API; Fallback = `price_manual.json`
- **Output:** Embed with TXM/USD, TXM/ETH, source label ("Uniswap V4 · OP Mainnet" or "Last set manually by Founder")
- **Error handling:** If both Uniswap and CoinGecko fail, show manual price with timestamp; if no manual price set, show "Price not available yet"

### `/setprice`
- **Description:** Manually set TXM price (Founder only)
- **Permission:** Restricted to role `👑 Founder` (checked via `interaction.user.get_role`)
- **Args:** `usd: float`
- **Behavior:** Writes to `price_manual.json`, confirms with ephemeral reply
- **Error handling:** Non-Founder gets ephemeral "You don't have permission to use this command"

### `/mining`
- **Description:** Quick start guide for mining TXM
- **Data:** Static embed
- **Output:** GPU requirements (RTX 3060+), download link, solo vs pool, docs link `https://docs.tensoriumlabs.com`

### `/wallet`
- **Description:** Download and install Tensorium wallet
- **Data:** Static embed
- **Output:** Chrome extension release link, CLI wallet info, mainnet vs testnet network selector note

### `/bridge`
- **Description:** Bridge TXM ↔ wTXM on Optimism
- **Data:** Static embed
- **Output:** Bridge URL `https://bridge.tensoriumlabs.com`, custody address, wTXM contract on OP, brief flow description

### `/otc`
- **Description:** OTC peer-to-peer trading info
- **Data:** Static embed
- **Output:** OTC URL `https://otc.tensoriumlabs.com`, #otc-trading channel link, safety reminder (never share private keys)

---

## Content Updates

### #rules (full rewrite)
- Remove all testnet/soak-test references
- Keep: respect, stay on topic, no spam, no scam links, no financial advice, no private keys, Discord ToS
- Update useful links section: remove faucet, add Uniswap pool link

### #faq (full rewrite)
- Block reward: **11.9027 TXM/block** (corrected from old 15.23 figure)
- Max supply: **33,000,000 TXM** (8M pre-mint + 25M mining)
- Mainnet status: **LIVE** — remove "soak test" language
- Remove faucet Q&A entirely
- Add: "Is TXM tradeable?" → Yes, wTXM on Uniswap V4 OP Mainnet via bridge

### #announcements (new post)
- Post **Mainnet Live** announcement: genesis hash, block reward, mining guide, bridge, wallet

### Welcome DM (on_member_join)
- Remove testnet faucet link
- Update to mainnet messaging: wallet install, #general-en, mining guide, bridge

---

## Deployment

1. Create `/root/txm_discord_bot/` with new files
2. Move token to `.env` (chmod 600)
3. Update systemd `ExecStart` to `python3 /root/txm_discord_bot/bot.py`
4. Run `python3 update_channels.py` once (separate one-shot script, idempotent: checks existing channels before creating/editing)
5. Restart service (`systemctl restart txm-discord-bot`)

**Bot permissions required (already granted):**
- `GUILD_MEMBERS` intent (privileged — already enabled for auto-role)
- `MANAGE_CHANNELS` — to create/update channels
- `MANAGE_ROLES` — to assign roles
- `SEND_MESSAGES`, `EMBED_LINKS` — for slash command responses

---

## Error Handling & Resilience

- All RPC/HTTP calls use `asyncio.wait_for` with 8s timeout
- Price fetch failures are silent to user (fallback shown instead)
- Bot reconnects automatically via discord.py's built-in reconnect logic
- Systemd `Restart=always` with `RestartSec=10`

---

## Out of Scope

- Stratum pool integration (pool is separate service on port 23336)
- Automatic block announcement in a channel (future enhancement)
- CMC/CoinGecko listing price feed (not listed yet)
- Testnet faucet command (`/faucet` removed per user decision)
