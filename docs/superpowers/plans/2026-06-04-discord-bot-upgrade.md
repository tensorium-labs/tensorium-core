# Discord Bot Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade the Tensorium Discord bot from a simple auto-role script to a full-featured mainnet bot with 8 slash commands, multi-language channels, and updated content reflecting mainnet-live status.

**Architecture:** Single Python package at `/root/txm_discord_bot/` — `bot.py` (discord.py 2.x client + `app_commands` slash commands) and `price.py` (GeckoTerminal + CoinGecko + manual fallback). A one-shot `update_channels.py` script rebuilds the server's channel structure and rewrites static content. Existing systemd service is updated in-place.

**Tech Stack:** Python 3.12, discord.py 2.3+, aiohttp 3.9+, python-dotenv, pytest + pytest-asyncio + aioresponses (tests), requests (update_channels.py one-shot script)

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `/root/txm_discord_bot/requirements.txt` | Create | Dependencies |
| `/root/txm_discord_bot/.env.example` | Create | Secret template |
| `/root/txm_discord_bot/.env` | Create on VPS | Actual secrets (not committed) |
| `/root/txm_discord_bot/price.py` | Create | GeckoTerminal price fetch + CoinGecko ETH/USD + manual fallback |
| `/root/txm_discord_bot/bot.py` | Create | Discord client, events, all 8 slash commands |
| `/root/txm_discord_bot/update_channels.py` | Create | One-shot channel rebuild + content rewrite |
| `/root/txm_discord_bot/tests/test_price.py` | Create | Unit tests for price.py |
| `/root/txm_discord_bot/tests/test_bot_helpers.py` | Create | Unit tests for format_hashrate + rpc_call |
| `/etc/systemd/system/txm-discord-bot.service` | Modify | Update ExecStart + WorkingDirectory |

---

## Task 1: Scaffold project structure

**Files:**
- Create: `/root/txm_discord_bot/requirements.txt`
- Create: `/root/txm_discord_bot/.env.example`
- Create: `/root/txm_discord_bot/tests/__init__.py`

- [ ] **Step 1: Create the project directory and files**

```bash
mkdir -p /root/txm_discord_bot/tests
touch /root/txm_discord_bot/tests/__init__.py
```

Create `/root/txm_discord_bot/requirements.txt`:
```
discord.py>=2.3.2
aiohttp>=3.9.0
python-dotenv>=1.0.0
requests>=2.31.0
pytest>=7.4.0
pytest-asyncio>=0.23.0
aioresponses>=0.7.6
```

Create `/root/txm_discord_bot/.env.example`:
```
TOKEN=your_bot_token_here
GUILD_ID=1511205971173707906
TXM_RPC_URL=https://mc-rpc.tensoriumlabs.com
EARLY_ROLE_ID=1511214814419095753
COMMUNITY_ROLE_ID=1511213800940896438
```

- [ ] **Step 2: Install dependencies**

```bash
cd /root/txm_discord_bot
pip3 install -r requirements.txt -q
```

Expected: installs discord.py, aiohttp, python-dotenv, pytest, pytest-asyncio, aioresponses with no errors.

- [ ] **Step 3: Create `.env` with real secrets**

```bash
cat > /root/txm_discord_bot/.env << 'EOF'
TOKEN=YOUR_BOT_TOKEN_HERE
GUILD_ID=1511205971173707906
TXM_RPC_URL=https://mc-rpc.tensoriumlabs.com
EARLY_ROLE_ID=1511214814419095753
COMMUNITY_ROLE_ID=1511213800940896438
EOF
chmod 600 /root/txm_discord_bot/.env
```

- [ ] **Step 4: Commit scaffold**

```bash
cd /root/.openclaw/workspace/tensorium-core
git add docs/
# Note: /root/txm_discord_bot/ is NOT in the tensorium-core repo.
# Commit is done from the bot directory after each task instead.
```

The bot files live at `/root/txm_discord_bot/` — not inside tensorium-core. Initialize a separate git repo for the bot:

```bash
cd /root/txm_discord_bot
git init
echo ".env" >> .gitignore
echo "price_manual.json" >> .gitignore
echo "__pycache__/" >> .gitignore
echo "*.pyc" >> .gitignore
git add requirements.txt .env.example .gitignore tests/
git commit -m "chore: scaffold txm-discord-bot project"
```

---

## Task 2: `price.py` — GeckoTerminal + manual fallback (TDD)

**Files:**
- Create: `/root/txm_discord_bot/price.py`
- Create: `/root/txm_discord_bot/tests/test_price.py`

- [ ] **Step 1: Write the failing tests**

Create `/root/txm_discord_bot/tests/test_price.py`:

```python
import pytest
import json
import os
from unittest.mock import patch

pytestmark = pytest.mark.asyncio

# Resolve imports relative to project root
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))


# ── helpers ──────────────────────────────────────────────────────────────────

GECKO_RESPONSE = {"data": {"attributes": {"price_usd": "0.005000"}}}
ETH_RESPONSE = {"ethereum": {"usd": 2500.0}}


async def test_fetch_uniswap_price_returns_usd_and_eth():
    from aioresponses import aioresponses
    import price

    with aioresponses() as m:
        m.get(price.GECKO_URL, payload=GECKO_RESPONSE)
        m.get(price.COINGECKO_ETH_URL, payload=ETH_RESPONSE)
        result = await price.fetch_uniswap_price()

    assert result is not None
    usd, eth = result
    assert abs(usd - 0.005) < 1e-10
    assert abs(eth - 0.005 / 2500.0) < 1e-15


async def test_fetch_uniswap_price_returns_none_on_http_error():
    from aioresponses import aioresponses
    import price

    with aioresponses() as m:
        m.get(price.GECKO_URL, status=500)
        result = await price.fetch_uniswap_price()

    assert result is None


async def test_fetch_uniswap_price_returns_none_on_exception():
    from aioresponses import aioresponses
    import price
    from aiohttp import ClientConnectionError

    with aioresponses() as m:
        m.get(price.GECKO_URL, exception=ClientConnectionError())
        result = await price.fetch_uniswap_price()

    assert result is None


def test_load_manual_price_returns_none_when_file_missing(tmp_path):
    import price
    with patch("price.MANUAL_PRICE_FILE", str(tmp_path / "nope.json")):
        assert price.load_manual_price() is None


def test_save_and_load_manual_price_roundtrip(tmp_path):
    import price
    path = str(tmp_path / "price_manual.json")
    with patch("price.MANUAL_PRICE_FILE", path):
        price.save_manual_price(0.0042)
        result = price.load_manual_price()
    assert result is not None
    assert abs(result["usd"] - 0.0042) < 1e-10
    assert "updated_at" in result


async def test_get_price_uses_uniswap_source_label():
    from aioresponses import aioresponses
    import price

    with aioresponses() as m:
        m.get(price.GECKO_URL, payload=GECKO_RESPONSE)
        m.get(price.COINGECKO_ETH_URL, payload=ETH_RESPONSE)
        with patch("price.MANUAL_PRICE_FILE", "/nonexistent/nope.json"):
            result = await price.get_price()

    assert result["source"] == "uniswap"
    assert abs(result["usd"] - 0.005) < 1e-10


async def test_get_price_falls_back_to_manual_when_uniswap_fails(tmp_path):
    from aioresponses import aioresponses
    import price

    path = str(tmp_path / "price_manual.json")
    with open(path, "w") as f:
        json.dump({"usd": 0.003, "updated_at": "2026-06-04T00:00:00+00:00"}, f)

    with aioresponses() as m:
        m.get(price.GECKO_URL, status=500)
        with patch("price.MANUAL_PRICE_FILE", path):
            result = await price.get_price()

    assert "manual" in result["source"]
    assert abs(result["usd"] - 0.003) < 1e-10


async def test_get_price_returns_unavailable_when_all_sources_fail(tmp_path):
    from aioresponses import aioresponses
    import price

    with aioresponses() as m:
        m.get(price.GECKO_URL, status=500)
        with patch("price.MANUAL_PRICE_FILE", str(tmp_path / "nope.json")):
            result = await price.get_price()

    assert result["source"] == "unavailable"
    assert result["usd"] is None
    assert result["eth"] is None
```

- [ ] **Step 2: Run tests — verify they fail (ImportError expected)**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/test_price.py -v 2>&1 | head -20
```

Expected: `ModuleNotFoundError: No module named 'price'`

- [ ] **Step 3: Implement `price.py`**

Create `/root/txm_discord_bot/price.py`:

```python
import aiohttp
import json
import os
from datetime import datetime, timezone

WTXM_ADDRESS = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e"
GECKO_URL = f"https://api.geckoterminal.com/api/v2/networks/optimism/tokens/{WTXM_ADDRESS}"
COINGECKO_ETH_URL = (
    "https://api.coingecko.com/api/v3/simple/price?ids=ethereum&vs_currencies=usd"
)
MANUAL_PRICE_FILE = os.path.join(os.path.dirname(__file__), "price_manual.json")

_TIMEOUT = aiohttp.ClientTimeout(total=8)


async def fetch_uniswap_price() -> tuple[float, float | None] | None:
    """Fetch wTXM price via GeckoTerminal + ETH/USD via CoinGecko.

    Returns (usd, eth_per_txm) on success, None on any failure.
    """
    try:
        async with aiohttp.ClientSession(timeout=_TIMEOUT) as session:
            async with session.get(GECKO_URL) as resp:
                if resp.status != 200:
                    return None
                data = await resp.json(content_type=None)
                usd = float(data["data"]["attributes"]["price_usd"])

            async with session.get(COINGECKO_ETH_URL) as eth_resp:
                if eth_resp.status != 200:
                    return usd, None
                eth_data = await eth_resp.json(content_type=None)
                eth_usd = float(eth_data["ethereum"]["usd"])
                eth_per_txm = usd / eth_usd if eth_usd else None
                return usd, eth_per_txm
    except Exception:
        return None


def load_manual_price() -> dict | None:
    """Load manually-set price from disk. Returns None if not set."""
    try:
        with open(MANUAL_PRICE_FILE) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def save_manual_price(usd: float) -> None:
    """Persist manually-set price to disk."""
    data = {"usd": usd, "updated_at": datetime.now(timezone.utc).isoformat()}
    with open(MANUAL_PRICE_FILE, "w") as f:
        json.dump(data, f)


async def get_price() -> dict:
    """Return price dict: {usd, eth, source}.

    source values:
      "uniswap"          — live from GeckoTerminal
      "manual:<iso_ts>"  — from price_manual.json
      "unavailable"      — all sources failed
    """
    result = await fetch_uniswap_price()
    if result is not None:
        usd, eth = result
        return {"usd": usd, "eth": eth, "source": "uniswap"}

    manual = load_manual_price()
    if manual:
        return {
            "usd": manual["usd"],
            "eth": None,
            "source": f"manual:{manual['updated_at']}",
        }

    return {"usd": None, "eth": None, "source": "unavailable"}
```

- [ ] **Step 4: Run tests — verify all pass**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/test_price.py -v
```

Expected output:
```
tests/test_price.py::test_fetch_uniswap_price_returns_usd_and_eth PASSED
tests/test_price.py::test_fetch_uniswap_price_returns_none_on_http_error PASSED
tests/test_price.py::test_fetch_uniswap_price_returns_none_on_exception PASSED
tests/test_price.py::test_load_manual_price_returns_none_when_file_missing PASSED
tests/test_price.py::test_save_and_load_manual_price_roundtrip PASSED
tests/test_price.py::test_get_price_uses_uniswap_source_label PASSED
tests/test_price.py::test_get_price_falls_back_to_manual_when_uniswap_fails PASSED
tests/test_price.py::test_get_price_returns_unavailable_when_all_sources_fail PASSED

8 passed
```

- [ ] **Step 5: Add `pytest.ini` for asyncio mode**

Create `/root/txm_discord_bot/pytest.ini`:
```ini
[pytest]
asyncio_mode = auto
```

Re-run to confirm still passing:
```bash
python3 -m pytest tests/test_price.py -v
```

- [ ] **Step 6: Commit**

```bash
cd /root/txm_discord_bot
git add price.py tests/test_price.py pytest.ini
git commit -m "feat: add price.py with GeckoTerminal+CoinGecko+manual fallback (8 tests)"
```

---

## Task 3: `bot.py` core — client setup, events, format_hashrate

**Files:**
- Create: `/root/txm_discord_bot/bot.py`
- Create: `/root/txm_discord_bot/tests/test_bot_helpers.py`

- [ ] **Step 1: Write failing test for `format_hashrate`**

Create `/root/txm_discord_bot/tests/test_bot_helpers.py`:

```python
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))


def test_format_hashrate_terahash():
    from bot import format_hashrate
    assert format_hashrate(5e12) == "5.00 TH/s"


def test_format_hashrate_gigahash():
    from bot import format_hashrate
    assert format_hashrate(7.64e9) == "7.64 GH/s"


def test_format_hashrate_megahash():
    from bot import format_hashrate
    assert format_hashrate(500e6) == "500.00 MH/s"


def test_format_hashrate_kilohash():
    from bot import format_hashrate
    assert format_hashrate(800e3) == "800.00 KH/s"
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/test_bot_helpers.py -v 2>&1 | head -10
```

Expected: `ModuleNotFoundError: No module named 'bot'`

- [ ] **Step 3: Create `bot.py` core**

Create `/root/txm_discord_bot/bot.py`:

```python
import discord
from discord import app_commands
import aiohttp
import os
from dotenv import load_dotenv
from price import get_price, save_manual_price

load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

TOKEN            = os.getenv("TOKEN")
GUILD_ID         = int(os.getenv("GUILD_ID",         "1511205971173707906"))
TXM_RPC_URL      = os.getenv("TXM_RPC_URL",          "https://mc-rpc.tensoriumlabs.com")
EARLY_ROLE_ID    = int(os.getenv("EARLY_ROLE_ID",    "1511214814419095753"))
COMMUNITY_ROLE_ID = int(os.getenv("COMMUNITY_ROLE_ID", "1511213800940896438"))
FOUNDER_ROLE_NAME = "👑 Founder"

intents = discord.Intents.default()
intents.members = True

client   = discord.Client(intents=intents)
tree     = app_commands.CommandTree(client)
GUILD_OBJ = discord.Object(id=GUILD_ID)


# ── Utilities ─────────────────────────────────────────────────────────────────

def format_hashrate(hr: float) -> str:
    """Format a hashrate (hashes/sec) as a human-readable string."""
    if hr >= 1e12:
        return f"{hr / 1e12:.2f} TH/s"
    if hr >= 1e9:
        return f"{hr / 1e9:.2f} GH/s"
    if hr >= 1e6:
        return f"{hr / 1e6:.2f} MH/s"
    return f"{hr / 1e3:.2f} KH/s"


async def rpc_call(method: str, params: list = None) -> object:
    """Call a Tensorium JSON-RPC method. Returns result or None on failure."""
    payload = {"jsonrpc": "2.0", "method": method, "params": params or [], "id": 1}
    timeout = aiohttp.ClientTimeout(total=8)
    try:
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.post(TXM_RPC_URL, json=payload) as resp:
                data = await resp.json(content_type=None)
                return data.get("result")
    except Exception:
        return None


# ── Events ────────────────────────────────────────────────────────────────────

@client.event
async def on_ready():
    await tree.sync(guild=GUILD_OBJ)
    guild = client.get_guild(GUILD_ID)
    print(f"[TXM Bot] Ready as {client.user} | Guild: {guild.name if guild else 'NOT FOUND'}")


@client.event
async def on_member_join(member: discord.Member):
    guild = member.guild
    try:
        early_role     = guild.get_role(EARLY_ROLE_ID)
        community_role = guild.get_role(COMMUNITY_ROLE_ID)
        roles_to_add   = [r for r in [early_role, community_role] if r]
        if roles_to_add:
            await member.add_roles(*roles_to_add, reason="Auto: Early Adopter on join")

        rules_ch   = discord.utils.get(guild.channels, name="rules")
        general_ch = discord.utils.get(guild.channels, name="general-en")
        rules_ref   = rules_ch.mention   if rules_ch   else "#rules"
        general_ref = general_ch.mention if general_ch else "#general-en"

        try:
            await member.send(
                f"👋 **Welcome to Tensorium (TXM), {member.display_name}!**\n\n"
                f"You've been given the ⭐ **Early Adopter** role — you're part of the first wave.\n\n"
                f"**Tensorium mainnet is LIVE** — SHA256d PoW, GPU-first mining.\n\n"
                f"**Get started:**\n"
                f"• Read the rules → {rules_ref}\n"
                f"• Introduce yourself → {general_ref}\n"
                f"• Install wallet → https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest\n"
                f"• Start mining → https://docs.tensoriumlabs.com\n"
                f"• Bridge TXM ↔ wTXM → https://bridge.tensoriumlabs.com\n\n"
                f"Use `/stats` for live chain stats, `/mining` for the mining guide. ⛏️"
            )
        except discord.Forbidden:
            pass  # member has DMs disabled
    except Exception as e:
        print(f"[TXM Bot] on_member_join error for {member.name}: {e}")


# Slash commands are added in subsequent tasks.

if __name__ == "__main__":
    client.run(TOKEN)
```

- [ ] **Step 4: Run tests — verify format_hashrate tests pass**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/test_bot_helpers.py::test_format_hashrate_gigahash \
                  tests/test_bot_helpers.py::test_format_hashrate_terahash \
                  tests/test_bot_helpers.py::test_format_hashrate_megahash \
                  tests/test_bot_helpers.py::test_format_hashrate_kilohash -v
```

Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
cd /root/txm_discord_bot
git add bot.py tests/test_bot_helpers.py
git commit -m "feat: add bot.py core — client, events, format_hashrate (4 tests)"
```

---

## Task 4: Add `rpc_call` tests + `/stats` + `/block` to `bot.py`

**Files:**
- Modify: `/root/txm_discord_bot/tests/test_bot_helpers.py` (add rpc_call tests)
- Modify: `/root/txm_discord_bot/bot.py` (add /stats and /block)

- [ ] **Step 1: Add rpc_call tests to `tests/test_bot_helpers.py`**

Append to the bottom of `tests/test_bot_helpers.py`:

```python
import pytest

pytestmark = pytest.mark.asyncio


async def test_rpc_call_returns_result_on_success():
    from aioresponses import aioresponses
    from bot import rpc_call

    with aioresponses() as m:
        m.post(
            "https://mc-rpc.tensoriumlabs.com",
            payload={"jsonrpc": "2.0", "result": 442, "id": 1},
        )
        result = await rpc_call("getblockcount")

    assert result == 442


async def test_rpc_call_returns_none_on_http_error():
    from aioresponses import aioresponses
    from bot import rpc_call

    with aioresponses() as m:
        m.post("https://mc-rpc.tensoriumlabs.com", status=500)
        result = await rpc_call("getblockcount")

    assert result is None


async def test_rpc_call_returns_none_on_exception():
    from aioresponses import aioresponses
    from bot import rpc_call
    from aiohttp import ClientConnectionError

    with aioresponses() as m:
        m.post("https://mc-rpc.tensoriumlabs.com", exception=ClientConnectionError())
        result = await rpc_call("getblockcount")

    assert result is None
```

- [ ] **Step 2: Run rpc_call tests — verify they pass**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/test_bot_helpers.py -v
```

Expected: 7 passed (4 format_hashrate + 3 rpc_call)

- [ ] **Step 3: Add `/stats` and `/block` commands to `bot.py`**

Add the following block to `bot.py` BEFORE the `if __name__ == "__main__":` line:

```python
# ── /stats ────────────────────────────────────────────────────────────────────

@tree.command(name="stats", description="Show Tensorium mainnet chain stats", guild=GUILD_OBJ)
async def cmd_stats(interaction: discord.Interaction):
    await interaction.response.defer()

    height = await rpc_call("getblockcount")
    if height is None:
        await interaction.followup.send(
            "⚠️ Node temporarily unreachable. Check https://explorer.tensoriumlabs.com"
        )
        return

    block_hash = await rpc_call("getblockhash", [height])
    block      = await rpc_call("getblock", [block_hash]) if block_hash else None

    difficulty_bits = block.get("difficulty_bits", 40) if isinstance(block, dict) else 40
    hashrate        = (2 ** difficulty_bits) / 144  # ~2.4 min target block time

    embed = discord.Embed(title="⛓️ Tensorium Mainnet Stats", color=0x00D4AA)
    embed.add_field(name="Block Height",         value=f"`{height:,}`",              inline=True)
    embed.add_field(name="Difficulty",           value=f"`{difficulty_bits} bits`",  inline=True)
    embed.add_field(name="Est. Network Hashrate", value=f"`{format_hashrate(hashrate)}`", inline=True)
    embed.add_field(name="Chain ID",             value="`tensorium-mainnet-candidate-0`", inline=False)
    embed.set_footer(text="tensoriumlabs.com  •  explorer.tensoriumlabs.com")
    await interaction.followup.send(embed=embed)


# ── /block ────────────────────────────────────────────────────────────────────

@tree.command(name="block", description="Show latest block details", guild=GUILD_OBJ)
async def cmd_block(interaction: discord.Interaction):
    await interaction.response.defer()

    height = await rpc_call("getblockcount")
    if height is None:
        await interaction.followup.send(
            "⚠️ Node temporarily unreachable. Check https://explorer.tensoriumlabs.com"
        )
        return

    block_hash = await rpc_call("getblockhash", [height])
    block      = await rpc_call("getblock", [block_hash]) if block_hash else None

    if not isinstance(block, dict):
        await interaction.followup.send("⚠️ Could not fetch block data.")
        return

    from datetime import datetime, timezone
    ts      = block.get("timestamp", 0)
    dt_str  = datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%d %H:%M UTC") if ts else "Unknown"
    tx_count = len(block.get("transactions", []))
    hash_str = str(block_hash)[:24] + "…" if block_hash else "Unknown"

    embed = discord.Embed(title=f"📦 Block #{height:,}", color=0x5865F2)
    embed.add_field(name="Hash",         value=f"`{hash_str}`",  inline=False)
    embed.add_field(name="Time",         value=f"`{dt_str}`",    inline=True)
    embed.add_field(name="Transactions", value=f"`{tx_count}`",  inline=True)
    embed.add_field(name="Miner Reward", value="`11.9027 TXM`",  inline=True)
    embed.set_footer(text="https://explorer.tensoriumlabs.com")
    await interaction.followup.send(embed=embed)
```

- [ ] **Step 4: Confirm all tests still pass**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/ -v
```

Expected: 15 passed (8 price + 7 bot_helpers)

- [ ] **Step 5: Commit**

```bash
cd /root/txm_discord_bot
git add bot.py tests/test_bot_helpers.py
git commit -m "feat: add /stats and /block slash commands with rpc_call helper (7 tests)"
```

---

## Task 5: Add `/price` and `/setprice` to `bot.py`

**Files:**
- Modify: `/root/txm_discord_bot/bot.py`

- [ ] **Step 1: Add `/price` and `/setprice` commands**

Add the following block to `bot.py` BEFORE the `if __name__ == "__main__":` line:

```python
# ── /price ────────────────────────────────────────────────────────────────────

@tree.command(name="price", description="Show TXM price in USD and ETH", guild=GUILD_OBJ)
async def cmd_price(interaction: discord.Interaction):
    await interaction.response.defer()
    data = await get_price()

    if data["source"] == "unavailable":
        embed = discord.Embed(
            title="💹 TXM Price",
            description=(
                "Price data not available yet.\n"
                "Trade OTC: https://otc.tensoriumlabs.com\n"
                "Bridge to wTXM: https://bridge.tensoriumlabs.com"
            ),
            color=0xFF6B6B,
        )
    else:
        usd_str = f"${data['usd']:.6f}" if data["usd"] is not None else "N/A"
        eth_str = f"{data['eth']:.8f} ETH" if data["eth"] is not None else "N/A"

        if data["source"] == "uniswap":
            source_text = "Uniswap V4 · OP Mainnet"
            color = 0xFF007A
        else:
            ts_part = data["source"].split(":", 1)[1][:10] if ":" in data["source"] else ""
            source_text = f"Manual (set by Founder{', ' + ts_part if ts_part else ''})"
            color = 0xFFA500

        embed = discord.Embed(title="💹 TXM Price", color=color)
        embed.add_field(name="USD",    value=f"`{usd_str}`", inline=True)
        embed.add_field(name="ETH",    value=f"`{eth_str}`", inline=True)
        embed.add_field(name="Source", value=source_text,    inline=False)
        embed.add_field(
            name="Trade",
            value="[Uniswap V4 · OP Mainnet](https://app.uniswap.org)  |  [OTC Board](https://otc.tensoriumlabs.com)",
            inline=False,
        )

    await interaction.followup.send(embed=embed)


# ── /setprice ─────────────────────────────────────────────────────────────────

@tree.command(name="setprice", description="Manually set TXM price — Founder only", guild=GUILD_OBJ)
@app_commands.describe(usd="TXM price in USD (e.g. 0.005)")
async def cmd_setprice(interaction: discord.Interaction, usd: float):
    is_founder = any(r.name == FOUNDER_ROLE_NAME for r in interaction.user.roles)
    if not is_founder:
        await interaction.response.send_message(
            "❌ Requires 👑 Founder role.", ephemeral=True
        )
        return
    save_manual_price(usd)
    await interaction.response.send_message(
        f"✅ Manual TXM price set to **${usd:.6f}** USD.", ephemeral=True
    )
```

- [ ] **Step 2: Run full test suite**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/ -v
```

Expected: 15 passed (no regressions)

- [ ] **Step 3: Commit**

```bash
cd /root/txm_discord_bot
git add bot.py
git commit -m "feat: add /price and /setprice slash commands"
```

---

## Task 6: Add static slash commands `/mining` `/wallet` `/bridge` `/otc`

**Files:**
- Modify: `/root/txm_discord_bot/bot.py`

- [ ] **Step 1: Add all four static commands**

Add the following block to `bot.py` BEFORE the `if __name__ == "__main__":` line:

```python
# ── /mining ───────────────────────────────────────────────────────────────────

@tree.command(name="mining", description="Quick start guide for mining TXM", guild=GUILD_OBJ)
async def cmd_mining(interaction: discord.Interaction):
    embed = discord.Embed(title="⛏️ Mine TXM — Quick Start", color=0xFF7700)
    embed.add_field(
        name="Requirements",
        value="NVIDIA GPU — RTX 3060 or better\nCUDA drivers installed\nMainnet difficulty: **40 bits**",
        inline=False,
    )
    embed.add_field(
        name="Download",
        value=(
            "[tensorium-core releases](https://github.com/tensorium-labs/tensorium-core/releases/latest)\n"
            "Pick `tensorium-miner-linux-x86_64-sm<arch>` for your GPU"
        ),
        inline=False,
    )
    embed.add_field(
        name="Solo Mining (fee-free)",
        value=(
            "```\ntensorium-node mainnet-candidate rpc\n"
            "tensorium-miner --mode solo \\\n"
            "  --rpc http://127.0.0.1:33332 \\\n"
            "  --wallet YOUR_ADDRESS --gpu all\n```"
        ),
        inline=False,
    )
    embed.add_field(
        name="Pool Mining (5% fee)",
        value=(
            "```\ntensorium-miner --mode pool \\\n"
            "  --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 \\\n"
            "  --wallet YOUR_ADDRESS\n```"
        ),
        inline=False,
    )
    embed.add_field(name="Full Guide", value="https://docs.tensoriumlabs.com", inline=False)
    await interaction.response.send_message(embed=embed)


# ── /wallet ───────────────────────────────────────────────────────────────────

@tree.command(name="wallet", description="Download and install Tensorium wallet", guild=GUILD_OBJ)
async def cmd_wallet(interaction: discord.Interaction):
    embed = discord.Embed(title="👛 Tensorium Wallet", color=0x57F287)
    embed.add_field(
        name="Chrome Extension",
        value=(
            "[Download latest release](https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest)\n"
            "Install: chrome://extensions → Enable Developer Mode → Load Unpacked"
        ),
        inline=False,
    )
    embed.add_field(
        name="CLI Wallet (txmwallet)",
        value=(
            "Included in the node release.\n"
            "`txmwallet new-wallet` to create · `txmwallet send` to send TXM"
        ),
        inline=False,
    )
    embed.add_field(
        name="Networks",
        value=(
            "**Mainnet:** `mc-rpc.tensoriumlabs.com:33332`\n"
            "**Testnet:** `rpc.tensoriumlabs.com:23332` (dev / CPU-minable)"
        ),
        inline=False,
    )
    embed.add_field(name="Docs", value="https://docs.tensoriumlabs.com", inline=False)
    await interaction.response.send_message(embed=embed)


# ── /bridge ───────────────────────────────────────────────────────────────────

@tree.command(name="bridge", description="Bridge TXM ↔ wTXM on Optimism", guild=GUILD_OBJ)
async def cmd_bridge(interaction: discord.Interaction):
    embed = discord.Embed(title="🌉 TXM Bridge — Optimism", color=0xFF0420)
    embed.add_field(name="Bridge UI", value="https://bridge.tensoriumlabs.com", inline=False)
    embed.add_field(
        name="TXM → wTXM (Deposit)",
        value=(
            "1. Send TXM to the custody address shown on the bridge page\n"
            "2. Submit txid + OP wallet address in #bridge-wtxm\n"
            "3. Automated relayer mints wTXM on Optimism (~1–2 min)"
        ),
        inline=False,
    )
    embed.add_field(
        name="wTXM → TXM (Withdraw)",
        value=(
            "1. Burn wTXM via bridge contract\n"
            "2. Relayer releases TXM to your mainnet address"
        ),
        inline=False,
    )
    embed.add_field(
        name="wTXM Contract (OP Mainnet)",
        value="`0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e`",
        inline=False,
    )
    embed.add_field(
        name="Trade",
        value="[Uniswap V4 · OP Mainnet](https://app.uniswap.org) — wTXM/ETH pool",
        inline=False,
    )
    await interaction.response.send_message(embed=embed)


# ── /otc ──────────────────────────────────────────────────────────────────────

@tree.command(name="otc", description="OTC peer-to-peer trading info", guild=GUILD_OBJ)
async def cmd_otc(interaction: discord.Interaction):
    embed = discord.Embed(title="🤝 OTC Trading", color=0xEB459E)
    embed.add_field(name="OTC Board", value="https://otc.tensoriumlabs.com", inline=False)
    embed.add_field(
        name="How to trade",
        value=(
            "1. Post your offer on the OTC board or in #otc-trading\n"
            "2. Agree on price and amount with counterparty\n"
            "3. Trade in stages or use escrow for safety\n"
            "4. Verify on https://explorer.tensoriumlabs.com"
        ),
        inline=False,
    )
    embed.add_field(
        name="⚠️ Safety",
        value="**Never share your private key or wallet passphrase.**\nThe team will never DM you asking for funds.",
        inline=False,
    )
    await interaction.response.send_message(embed=embed)
```

- [ ] **Step 2: Run full test suite**

```bash
cd /root/txm_discord_bot
python3 -m pytest tests/ -v
```

Expected: 15 passed

- [ ] **Step 3: Commit**

```bash
cd /root/txm_discord_bot
git add bot.py
git commit -m "feat: add /mining /wallet /bridge /otc static slash commands"
```

---

## Task 7: `update_channels.py` — channel rebuild + content rewrite

**Files:**
- Create: `/root/txm_discord_bot/update_channels.py`

This is a one-shot maintenance script. Run it once on the VPS after deploying the bot. It is idempotent: it checks existence before creating/editing.

- [ ] **Step 1: Create `update_channels.py`**

Create `/root/txm_discord_bot/update_channels.py`:

```python
"""
One-shot Discord channel rebuild for Tensorium mainnet.
Run ONCE on VPS: python3 update_channels.py
Idempotent: checks existing channels before creating/updating.
"""
import requests, time, os, json
from dotenv import load_dotenv

load_dotenv(os.path.join(os.path.dirname(__file__), ".env"))

TOKEN  = os.getenv("TOKEN")
GUILD  = os.getenv("GUILD_ID", "1511205971173707906")
BASE   = "https://discord.com/api/v10"
H      = {"Authorization": f"Bot {TOKEN}", "Content-Type": "application/json"}

# ── API helper ────────────────────────────────────────────────────────────────

def api(method, path, **kw):
    r = requests.request(method, f"{BASE}{path}", headers=H, **kw)
    if r.status_code == 429:
        wait = r.json().get("retry_after", 1)
        print(f"  [rate limit] {wait:.1f}s")
        time.sleep(wait + 0.2)
        return api(method, path, **kw)
    time.sleep(0.45)
    return r

def purge_and_post(ch_id, content):
    """Delete existing messages and post new content (handles 2000-char Discord limit)."""
    msgs = api("GET", f"/channels/{ch_id}/messages?limit=10").json()
    if isinstance(msgs, list) and msgs:
        ids = [m["id"] for m in msgs]
        if len(ids) == 1:
            api("DELETE", f"/channels/{ch_id}/messages/{ids[0]}")
        else:
            api("POST", f"/channels/{ch_id}/messages/bulk-delete", json={"messages": ids})
        time.sleep(0.6)

    # Split content into ≤1990-char chunks at newline boundaries
    while content:
        chunk = content[:1990]
        if len(content) > 1990:
            cut = chunk.rfind("\n")
            if cut > 1000:
                chunk = content[:cut]
        api("POST", f"/channels/{ch_id}/messages", json={"content": chunk})
        content = content[len(chunk):]
        time.sleep(0.5)

# ── Target structure ──────────────────────────────────────────────────────────
# Each entry: (category_name, [channel_dicts])
# channel_dict keys: name, readonly (bool, optional), topic (str)

STRUCTURE = [
    ("📋 INFO", [
        {"name": "rules",         "readonly": True,  "topic": "📋 Read before participating. Violations → mute or ban."},
        {"name": "announcements", "readonly": True,  "topic": "📢 Official Tensorium announcements."},
        {"name": "faq",           "readonly": True,  "topic": "❓ Common questions about TXM, mining, and the wallet."},
    ]),
    ("💬 COMMUNITY — ENGLISH", [
        {"name": "general-en",    "topic": "💬 General chat about Tensorium."},
        {"name": "introductions", "topic": "👋 New here? Tell us who you are."},
        {"name": "off-topic",     "topic": "🎲 Anything goes — keep it civil."},
    ]),
    ("💬 CHAT — LANGUAGES", [
        {"name": "indonesia", "topic": "🇮🇩 Chat dalam Bahasa Indonesia."},
        {"name": "chinese",   "topic": "🇨🇳 普通话/中文交流。"},
        {"name": "russian",   "topic": "🇷🇺 Общение на русском языке."},
        {"name": "spanish",   "topic": "🇪🇸 Chat en español."},
        {"name": "french",    "topic": "🇫🇷 Chat en français."},
        {"name": "german",    "topic": "🇩🇪 Chat auf Deutsch."},
    ]),
    ("⛏️ MINING", [
        {"name": "mining-general", "topic": "⛏️ GPU mining discussion, tips, and strategies."},
        {"name": "mining-support", "topic": "🆘 Stuck? Post GPU model + OS + driver version."},
        {"name": "pool-mining",    "topic": "🏊 Official pool: https://pooltxm.tensoriumlabs.com | 5% fee | Solo mining is free."},
        {"name": "gpu-benchmarks", "topic": "📊 Share: GPU model | hashrate | power (W) | driver."},
    ]),
    ("🔧 DEVELOPMENT", [
        {"name": "dev-general", "topic": "🔧 Protocol and tooling discussion."},
        {"name": "wallet",      "topic": "👛 Chrome extension & txmwallet CLI — installs, bugs, requests."},
        {"name": "sdk-api",     "topic": "📦 tensorium-sdk and RPC API. https://docs.tensoriumlabs.com"},
        {"name": "bug-reports", "topic": "🐛 Format: [version] description + steps to reproduce + OS."},
    ]),
    ("📊 TRADING & ECOSYSTEM", [
        {"name": "price-talk",  "topic": "💹 TXM price discussion. Not financial advice. No pump/dump."},
        {"name": "otc-trading", "topic": "🤝 Peer-to-peer: https://otc.tensoriumlabs.com"},
        {"name": "bridge-wtxm", "topic": "🌉 Bridge TXM ↔ wTXM: https://bridge.tensoriumlabs.com"},
    ]),
    ("🌐 NETWORK", [
        {"name": "node-operators", "topic": "🖥️ Running a node? Share config, sync status, peer count."},
        {"name": "mainnet",        "topic": "🚀 Tensorium mainnet (40-bit, GPU). Explorer: https://explorer.tensoriumlabs.com"},
    ]),
]

VOICE_NAMES     = ["General Talk", "Mining Talk", "Dev Talk"]
DELETE_BY_NAME  = {"testnet", "mainnet-candidate"}  # channels to remove

# ── Static content ────────────────────────────────────────────────────────────

RULES_CONTENT = """**Welcome to the official Tensorium (TXM) Discord.**
https://tensoriumlabs.com

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
**📋  SERVER RULES**
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**1. Be respectful.**
No harassment, hate speech, personal attacks, or discrimination.

**2. Stay on topic.**
Use the right channel. Mining questions → #mining-support. Bugs → #bug-reports.

**3. No spam or self-promotion.**
Don't flood channels or advertise unrelated projects without permission.

**4. No scam links or phishing.**
The team will never DM you asking for your private key or funds.

**5. No financial advice.**
Price speculation belongs in #price-talk. Nothing here is investment advice.

**6. Never share your private key.**
Not in any channel, DM, or screenshot — ever.

**7. Follow Discord Terms of Service.**
https://discord.com/terms

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
**🔗  USEFUL LINKS**
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🌐 Website   →  https://tensoriumlabs.com
📖 Docs      →  https://docs.tensoriumlabs.com
🔭 Explorer  →  https://explorer.tensoriumlabs.com
⛏️ Pool      →  https://pooltxm.tensoriumlabs.com
🌉 Bridge    →  https://bridge.tensoriumlabs.com
📦 GitHub    →  https://github.com/tensorium-labs/tensorium-core
👛 Wallet    →  https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Violations → mute or permanent ban at moderator discretion."""

FAQ_CONTENT = """**❓  FREQUENTLY ASKED QUESTIONS**

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
**What is Tensorium (TXM)?**
A Proof-of-Work Layer 1 blockchain — SHA256d hashing, UTXO model, GPU-first mining, open source (Apache-2.0).

**What is the max supply?**
33,000,000 TXM — 8,000,000 pre-mint (founder + liquidity + bridge + ecosystem) + 25,000,000 mined over 10 halving eras (~20 years).

**What is the block reward?**
11.9027 TXM/block. Halving every ~2 years.

**What GPU do I need to mine?**
NVIDIA RTX 3060 or better. Mainnet difficulty is **40 bits** — GPU required.

**How do I start mining?**
→ https://docs.tensoriumlabs.com
Download `tensorium-miner`, run in solo or pool mode. Use `/mining` in any channel for a quick guide.

**What is the pool fee?**
The official pool at https://pooltxm.tensoriumlabs.com charges **5%**. Solo mining directly to your node is always fee-free.

**Where do I get a wallet?**
Chrome extension: https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest
CLI wallet (`txmwallet`): included in the node release package.

**Is Tensorium mainnet live?**
**Yes — mainnet is LIVE.**
Genesis block mined 2026-06-02. Chain ID: `tensorium-mainnet-candidate-0`.

**Is TXM tradeable?**
Yes. Bridge TXM → wTXM on Optimism via https://bridge.tensoriumlabs.com, then trade on Uniswap V4 (OP Mainnet).
OTC peer-to-peer: https://otc.tensoriumlabs.com

**What is the mainnet RPC endpoint?**
`mc-rpc.tensoriumlabs.com:33332` (HTTPS proxy)

**Where is the source code?**
https://github.com/tensorium-labs/tensorium-core (Apache-2.0)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Still have questions? Ask in #general-en or #mining-support."""

ANNOUNCEMENT_CONTENT = """📢  **TENSORIUM MAINNET IS LIVE**

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

After months of development, testing, and auditing — **Tensorium (TXM) mainnet is now live.**

**Genesis block:**
Hash: `0000000000007076b8daa7e605fcbdbeec5ad8f4dcedbfec762ae47a19ae18431b`
Mined: 2026-06-02 · RTX 5090 · ~4.64 GH/s

**Chain specs:**
• SHA256d Proof-of-Work, UTXO model
• Difficulty: **40 bits** (GPU required)
• Block reward: **11.9027 TXM**
• Max supply: **33,000,000 TXM**
• Chain ID: `tensorium-mainnet-candidate-0`

**Scripting layer:**
• P2PKH, multisig (m-of-n), CLTV, HTLC — live on mainnet

**Get started:**
1. Install wallet → https://github.com/tensorium-labs/tensorium-wallet-extension/releases/latest
2. Download node + miner → https://github.com/tensorium-labs/tensorium-core/releases/latest
3. Start mining → https://docs.tensoriumlabs.com
4. Bridge TXM ↔ wTXM (Optimism) → https://bridge.tensoriumlabs.com

**Explorer:** https://explorer.tensoriumlabs.com
**Pool:** https://pooltxm.tensoriumlabs.com (5% fee)
**OTC:** https://otc.tensoriumlabs.com

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
GitHub: https://github.com/tensorium-labs/tensorium-core
Docs: https://docs.tensoriumlabs.com"""

# ── Main execution ────────────────────────────────────────────────────────────

def main():
    print("[1] Fetching current server state...")
    all_channels = api("GET", f"/guilds/{GUILD}/channels").json()
    if not isinstance(all_channels, list):
        print(f"  ERROR: {all_channels}")
        return

    cats_by_name = {c["name"]: c for c in all_channels if c["type"] == 4}
    chs_by_name  = {c["name"]: c for c in all_channels if c["type"] == 0}
    voice_by_name = {c["name"]: c for c in all_channels if c["type"] == 2}

    print(f"  Existing categories: {list(cats_by_name.keys())}")
    print(f"  Existing text channels: {list(chs_by_name.keys())}")

    # ── Delete outdated channels ──────────────────────────────────────────────
    print("\n[2] Removing outdated channels...")
    for name in DELETE_BY_NAME:
        if name in chs_by_name:
            api("DELETE", f"/channels/{chs_by_name[name]['id']}")
            print(f"  deleted #{name}")

    # ── Create/update categories + channels ───────────────────────────────────
    print("\n[3] Creating/updating categories and channels...")
    everyone_id = GUILD  # @everyone role id == guild id

    for cat_name, channels in STRUCTURE:
        # Find or create category
        if cat_name in cats_by_name:
            cat_id = cats_by_name[cat_name]["id"]
            print(f"  category exists: {cat_name}")
        else:
            r = api("POST", f"/guilds/{GUILD}/channels", json={"name": cat_name, "type": 4})
            cat_id = r.json()["id"]
            print(f"  + created category: {cat_name}")

        for ch in channels:
            ch_name = ch["name"]
            topic   = ch.get("topic", "")
            readonly = ch.get("readonly", False)

            overwrites = []
            if readonly:
                overwrites.append({
                    "id": everyone_id, "type": 0,
                    "allow": "1024",  # VIEW_CHANNEL
                    "deny": "2048",   # SEND_MESSAGES
                })

            if ch_name in chs_by_name:
                # Update existing channel: topic + parent + overwrites
                api("PATCH", f"/channels/{chs_by_name[ch_name]['id']}", json={
                    "topic": topic,
                    "parent_id": cat_id,
                    "permission_overwrites": overwrites,
                })
                print(f"    updated #{ch_name}")
            else:
                payload = {
                    "name": ch_name,
                    "type": 0,
                    "parent_id": cat_id,
                    "topic": topic,
                    "permission_overwrites": overwrites,
                }
                r = api("POST", f"/guilds/{GUILD}/channels", json=payload)
                new_ch = r.json()
                chs_by_name[ch_name] = new_ch  # track for content step
                print(f"    + created #{ch_name}")

    # ── Voice channels ────────────────────────────────────────────────────────
    print("\n[4] Creating missing voice channels...")
    voice_cat = cats_by_name.get("🔊 VOICE")
    if not voice_cat:
        r = api("POST", f"/guilds/{GUILD}/channels", json={"name": "🔊 VOICE", "type": 4})
        voice_cat = r.json()

    for vname in VOICE_NAMES:
        if vname not in voice_by_name:
            api("POST", f"/guilds/{GUILD}/channels", json={
                "name": vname, "type": 2, "parent_id": voice_cat["id"]
            })
            print(f"  + created voice: {vname}")
        else:
            print(f"  voice exists: {vname}")

    # ── Rewrite static content ────────────────────────────────────────────────
    print("\n[5] Rewriting static channel content...")

    # Re-fetch channel list to get any newly created IDs
    all_channels = api("GET", f"/guilds/{GUILD}/channels").json()
    chs_by_name  = {c["name"]: c for c in all_channels if c["type"] == 0}

    for ch_name, content in [
        ("rules",         RULES_CONTENT),
        ("faq",           FAQ_CONTENT),
        ("announcements", ANNOUNCEMENT_CONTENT),
    ]:
        if ch_name in chs_by_name:
            purge_and_post(chs_by_name[ch_name]["id"], content)
            print(f"  rewrote #{ch_name}")
        else:
            print(f"  WARNING: #{ch_name} not found, skipping content")

    print("\n✅  Channel update complete!")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Commit**

```bash
cd /root/txm_discord_bot
git add update_channels.py
git commit -m "feat: add update_channels.py — channel rebuild + mainnet content"
```

---

## Task 8: Deploy to VPS

**Files:**
- Modify: `/etc/systemd/system/txm-discord-bot.service` (on VPS)
- The bot directory is created directly on the VPS; it is not inside tensorium-core.

All steps in this task run **on VPS `157.230.44.162`**.

- [ ] **Step 1: Copy bot files to VPS**

From your local machine / this workspace:

```bash
# Create directory on VPS
ssh root@157.230.44.162 "mkdir -p /root/txm_discord_bot/tests"

# Copy all files
scp /root/txm_discord_bot/bot.py           root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/price.py         root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/update_channels.py root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/requirements.txt root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/.env.example     root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/pytest.ini       root@157.230.44.162:/root/txm_discord_bot/
scp /root/txm_discord_bot/tests/test_price.py      root@157.230.44.162:/root/txm_discord_bot/tests/
scp /root/txm_discord_bot/tests/test_bot_helpers.py root@157.230.44.162:/root/txm_discord_bot/tests/
scp /root/txm_discord_bot/tests/__init__.py root@157.230.44.162:/root/txm_discord_bot/tests/
```

- [ ] **Step 2: Create `.env` on VPS**

```bash
ssh root@157.230.44.162 "cat > /root/txm_discord_bot/.env << 'EOF'
TOKEN=YOUR_BOT_TOKEN_HERE
GUILD_ID=1511205971173707906
TXM_RPC_URL=https://mc-rpc.tensoriumlabs.com
EARLY_ROLE_ID=1511214814419095753
COMMUNITY_ROLE_ID=1511213800940896438
EOF
chmod 600 /root/txm_discord_bot/.env"
```

- [ ] **Step 3: Install dependencies on VPS**

```bash
ssh root@157.230.44.162 "cd /root/txm_discord_bot && pip3 install -r requirements.txt -q"
```

Expected: no errors

- [ ] **Step 4: Run tests on VPS**

```bash
ssh root@157.230.44.162 "cd /root/txm_discord_bot && python3 -m pytest tests/ -v 2>&1 | tail -5"
```

Expected: `15 passed`

- [ ] **Step 5: Stop the old bot service**

```bash
ssh root@157.230.44.162 "systemctl stop txm-discord-bot"
```

- [ ] **Step 6: Update the systemd service**

```bash
ssh root@157.230.44.162 "cat > /etc/systemd/system/txm-discord-bot.service << 'EOF'
[Unit]
Description=TXM Discord Bot
After=network.target

[Service]
ExecStart=/usr/bin/python3 /root/txm_discord_bot/bot.py
WorkingDirectory=/root/txm_discord_bot
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF"

ssh root@157.230.44.162 "systemctl daemon-reload && systemctl enable txm-discord-bot"
```

- [ ] **Step 7: Run `update_channels.py` once**

```bash
ssh root@157.230.44.162 "cd /root/txm_discord_bot && python3 update_channels.py"
```

Expected: output shows categories and channels created/updated, then `✅  Channel update complete!`

- [ ] **Step 8: Start the bot service**

```bash
ssh root@157.230.44.162 "systemctl start txm-discord-bot && sleep 3 && systemctl status txm-discord-bot --no-pager | head -15"
```

Expected: `Active: active (running)`

- [ ] **Step 9: Verify slash commands appear in Discord**

1. Open Discord → Tensorium server
2. Type `/` in any channel — confirm all 8 commands appear: `stats`, `block`, `price`, `setprice`, `mining`, `wallet`, `bridge`, `otc`
3. Run `/stats` — confirm embed with block height, difficulty, hashrate
4. Run `/block` — confirm embed with latest block info
5. Run `/price` — confirm price embed (shows "Price not available yet" if GeckoTerminal doesn't list wTXM yet, or live price if it does)
6. Run `/mining` — confirm mining guide embed
7. Run `/wallet` — confirm wallet download embed
8. Run `/bridge` — confirm bridge info embed
9. Run `/otc` — confirm OTC embed

- [ ] **Step 10: Verify channel structure**

In Discord server, confirm:
- Categories: 📋 INFO, 💬 COMMUNITY — ENGLISH, 💬 CHAT — LANGUAGES, ⛏️ MINING, 🔧 DEVELOPMENT, 📊 TRADING & ECOSYSTEM, 🌐 NETWORK, 🔊 VOICE
- Language channels: #indonesia, #chinese, #russian, #spanish, #french, #german
- #testnet and #mainnet-candidate are gone
- #rules, #faq, #announcements have updated mainnet content

- [ ] **Step 11: Tail logs to confirm no errors**

```bash
ssh root@157.230.44.162 "journalctl -u txm-discord-bot -f --no-pager -n 20"
```

Expected: `[TXM Bot] Ready as TXM Bot#XXXX | Guild: Tensorium` with no exceptions.
