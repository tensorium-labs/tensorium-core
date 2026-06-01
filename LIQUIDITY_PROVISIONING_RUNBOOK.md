# Tensorium wTXM/WETH Liquidity Provisioning — Runbook

**Target:** Create a Uniswap v3 wTXM/WETH pool on Optimism mainnet and add initial liquidity.

**Current state (2026-06-01):**
- wTXM deployed at `0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e` (Optimism mainnet)
- BridgeController at `0x4b31C557AD64609B975610812273BF82F1475384` (Optimism mainnet)
- wTXM total supply: 0 (no one has bridged yet)
- Uniswap v3 pool: does not exist yet
- Deployer wallet ETH: ~0.0013 ETH — **not enough, needs top-up**

---

## Step 0 — Decisions to make before running

| Parameter | Description | Suggested starting value |
|---|---|---|
| `INITIAL_PRICE_ETH_PER_WTXM` | Price of 1 wTXM in ETH | `0.0001` (~$0.30 at ETH=$3000) |
| `WTXM_LIQUIDITY_AMOUNT` | wTXM to deposit | `50000` |
| ETH to pair | Must = `WTXM_LIQUIDITY_AMOUNT × price` | `5.0` ETH (at 0.0001 price) |
| `FEE_TIER` | Uniswap fee tier | `10000` (1%, best for new tokens) |

**Important:** Initial price is up to you. It doesn't have to reflect a "fair" market price since there is no market yet. Pick a round number that you're comfortable defending publicly.

---

## Step 1 — Fund the deployer wallet on Optimism

Deployer address: `0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB`

Required ETH on Optimism mainnet:
- **ETH for the pool** — amount you want to provide as liquidity (e.g. 5 ETH)
- **Gas budget** — ~0.005 ETH covers all 5 transactions

Options to get ETH on Optimism:
```
1. Bridge ETH from Ethereum mainnet via https://app.optimism.io/bridge
2. Buy ETH directly on Optimism via Coinbase or Binance withdrawal
3. Use Across Protocol, Synapse, or Stargate for fast bridge
```

Check current balance:
```bash
curl -s -X POST https://mainnet.optimism.io \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0x15a8A0A259417ba0fFE92488FF09D458BE6ef9EB","latest"]}' \
  | python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(int(r,16)/1e18, 'ETH')"
```

---

## Step 2 — Set up .env

In `tensorium-core/contracts/.env`:
```bash
DEPLOYER_PRIVATE_KEY=<your private key — never commit this>
OP_MAINNET_RPC_URL=https://mainnet.optimism.io

# Liquidity parameters (override defaults)
INITIAL_PRICE_ETH_PER_WTXM=0.0001
WTXM_LIQUIDITY_AMOUNT=50000
FEE_TIER=10000
```

---

## Step 3 — Run the provisioning script

```bash
cd tensorium-core/contracts
npx hardhat run scripts/provision-liquidity.js --network op-mainnet
```

The script will:
1. Check wTXM balance; if 0, mint via bridge controller (you are the operator)
2. Check if pool exists; if not, create it
3. Initialize pool with the specified price
4. Wrap ETH → WETH
5. Approve wTXM + WETH on NonfungiblePositionManager
6. Mint full-range LP position

---

## Step 4 — Verify on-chain

After running:

**Uniswap pool:**
```
https://app.uniswap.org/explore/tokens/optimism/0x2e71fd45530fae75b6b427f3e71a0cdeb146c20e
```

**Your LP position (NFT):**
```
https://app.uniswap.org/pools
```

**Pool on Optimistic Etherscan:**
```
https://optimistic.etherscan.io/address/<pool_address>
```

---

## Step 5 — Update bridge.tensoriumlabs.com

After pool is live, update the bridge page to show the Uniswap link so users can swap wTXM:
- Add "Trade on Uniswap" button with pool URL
- Show current price feed (if available)

---

## What the script does NOT do

- Does not manage the LP position over time (adding/removing liquidity)
- Does not set up a price oracle
- Does not make the pool "concentrated" (uses full range ±∞ which is less capital efficient)
- Does not verify the operator key is the correct multisig signer

## Risks to communicate publicly

- Initial liquidity is small; price will be volatile until more liquidity joins
- Uniswap v3 full-range LP earns fees but has high impermanent loss exposure
- wTXM has no price peg — price is discovered by the market
- Testnet TXM has no value; only mainnet-candidate TXM bridged to wTXM has context

---

## Troubleshooting

**"is not an operator" error:**
The deployer address is not set as operator on the bridge controller. Call:
```solidity
controller.setOperator(deployer, true);  // owner only
```

**Insufficient ETH error:**
Add more ETH to the deployer wallet on Optimism.

**Pool already initialized at wrong price:**
Cannot change price once initialized without removing all liquidity. If pool was initialized incorrectly, remove LP position and recreate pool with different fee tier.

**maxPerTx exceeded:**
The current `maxPerTx` is 10,000 wTXM. For larger amounts, the owner must call `setMaxPerTx(newMax)` first. The script mints in one call, so amount must be ≤ maxPerTx.
