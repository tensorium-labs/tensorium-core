/**
 * create-v4-pool.js
 *
 * Creates and initializes a Uniswap v4 wTXM/ETH pool on Optimism mainnet.
 * This script ONLY creates + initializes the pool price.
 * Liquidity is added separately via the Uniswap UI or a second script.
 *
 * Pool design:
 *   currency0 = native ETH (address(0), lower address)
 *   currency1 = wTXM (0x2e71..., higher address)
 *   fee       = 10000 (1%) — standard for new/illiquid tokens
 *   tickSpacing = 200
 *   hooks     = address(0) — no custom hooks
 *
 * Initial price: 1 TXM = $0.0005 USD (fetched live from Chainlink ETH/USD)
 *
 * Usage:
 *   npx hardhat run scripts/create-v4-pool.js --network op-mainnet
 *
 * Required .env:
 *   DEPLOYER_PRIVATE_KEY
 *   OP_MAINNET_RPC_URL (optional, defaults to public endpoint)
 */

import hardhat from "hardhat";
const { ethers, network } = hardhat;

// ── Addresses ─────────────────────────────────────────────────────────────────
const WTXM            = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const ETH_NATIVE      = "0x0000000000000000000000000000000000000000";
const POSITION_MANAGER = "0x7C5f5A4bBd8fD63184577525326123B519429bDc";
const POOL_MANAGER    = "0x498581fF718922c3f8e6A244956aF099B2652b2b";
const CHAINLINK_ETH_USD = "0x13e3Ee699D1909E989722E753853AE30b17e08c5"; // Optimism

// ── Pool config ───────────────────────────────────────────────────────────────
const FEE          = 10000;   // 1% — protects LPs from arb on low-liquidity pool
const TICK_SPACING = 200;     // matches 1% fee tier in v4
const HOOKS        = "0x0000000000000000000000000000000000000000";
const TXM_USD_TARGET = 0.0005; // $0.0005 per TXM

// ── ABIs ──────────────────────────────────────────────────────────────────────
const PM_ABI = [
  // IPoolInitializer_v4
  `function initializePool(
    tuple(
      address currency0,
      address currency1,
      uint24  fee,
      int24   tickSpacing,
      address hooks
    ) key,
    uint160 sqrtPriceX96
  ) external payable returns (int24 tick)`,

  // IPoolManager.getSlot0 (via extsload through PositionManager)
  `function poolManager() external view returns (address)`,
];

const POOL_MGR_ABI = [
  `function getSlot0(bytes32 poolId) external view returns (
    uint160 sqrtPriceX96,
    int24   tick,
    uint24  protocolFee,
    uint24  lpFee
  )`,
];

const CHAINLINK_ABI = [
  "function latestAnswer() external view returns (int256)",
  "function decimals() external view returns (uint8)",
];

// ── Helpers ───────────────────────────────────────────────────────────────────

async function getEthUsd(provider) {
  const feed = new ethers.Contract(CHAINLINK_ETH_USD, CHAINLINK_ABI, provider);
  const [answer, decimals] = await Promise.all([feed.latestAnswer(), feed.decimals()]);
  return Number(answer) / 10 ** Number(decimals);
}

function calcSqrtPriceX96(wTxmPerEth) {
  // price = currency1 amount / currency0 amount
  // currency0 = ETH, currency1 = wTXM
  // price = wTXM per ETH = wTxmPerEth
  // both 18 decimals so raw ratio = wTxmPerEth
  // sqrtPriceX96 = sqrt(wTxmPerEth) * 2^96
  const Q96 = 2n ** 96n;
  // Use high-precision integer sqrt via BigInt
  // price scaled up by 1e18 for precision
  const SCALE = 10n ** 18n;
  const priceScaled = BigInt(Math.round(wTxmPerEth * 1e9)) * (SCALE / 1000000000n);
  // sqrtPriceX96 = sqrt(priceScaled) * Q96 / sqrt(SCALE)
  const target = priceScaled * Q96 * Q96 / SCALE;
  let x = BigInt(Math.ceil(Math.sqrt(Number(target))));
  // Newton refinement
  for (let i = 0; i < 20; i++) {
    if (x === 0n) break;
    const next = (x + target / x) / 2n;
    if (next >= x) break;
    x = next;
  }
  return x;
}

function poolKeyHash(currency0, currency1, fee, tickSpacing, hooks) {
  return ethers.keccak256(
    ethers.AbiCoder.defaultAbiCoder().encode(
      ["address", "address", "uint24", "int24", "address"],
      [currency0, currency1, fee, tickSpacing, hooks]
    )
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const [deployer] = await ethers.getSigners();
  const addr = deployer.address;
  const balance = await ethers.provider.getBalance(addr);

  console.log("\n══════════════════════════════════════════════════════");
  console.log("  Tensorium — Uniswap v4 wTXM/ETH Pool Initialization");
  console.log("══════════════════════════════════════════════════════\n");
  console.log("Network:      ", network.name);
  console.log("Deployer:     ", addr);
  console.log("ETH balance:  ", ethers.formatEther(balance), "ETH");

  if (balance < ethers.parseEther("0.002")) {
    throw new Error("Insufficient ETH for gas. Need at least 0.002 ETH.");
  }

  // ── Fetch live ETH price ───────────────────────────────────────────────────
  const ethUsd = await getEthUsd(ethers.provider);
  const wTxmPerEth = ethUsd / TXM_USD_TARGET;
  const sqrtPriceX96 = calcSqrtPriceX96(wTxmPerEth);

  console.log("\n── Price ────────────────────────────────────────────");
  console.log(`ETH/USD (Chainlink): $${ethUsd.toFixed(2)}`);
  console.log(`TXM target:          $${TXM_USD_TARGET}`);
  console.log(`wTXM per ETH:        ${wTxmPerEth.toFixed(0)}`);
  console.log(`sqrtPriceX96:        ${sqrtPriceX96.toString()}`);

  // ── Build pool key ────────────────────────────────────────────────────────
  // In v4: currency0 < currency1 (by address uint value)
  // ETH native = address(0) = 0x0000...0000 which is always < any ERC20
  const poolKey = {
    currency0:   ETH_NATIVE,
    currency1:   WTXM,
    fee:         FEE,
    tickSpacing: TICK_SPACING,
    hooks:       HOOKS,
  };

  console.log("\n── Pool Key ─────────────────────────────────────────");
  console.log("currency0:   ", poolKey.currency0, "(native ETH)");
  console.log("currency1:   ", poolKey.currency1, "(wTXM)");
  console.log("fee:         ", poolKey.fee, "(1%)");
  console.log("tickSpacing: ", poolKey.tickSpacing);
  console.log("hooks:       ", poolKey.hooks, "(none)");

  const pm = new ethers.Contract(POSITION_MANAGER, PM_ABI, deployer);

  // ── Check if pool already initialized ────────────────────────────────────
  console.log("\n── Pool Status ──────────────────────────────────────");
  try {
    const poolMgrAddr = await pm.poolManager();
    const poolMgr = new ethers.Contract(poolMgrAddr, POOL_MGR_ABI, ethers.provider);
    const poolId = poolKeyHash(
      poolKey.currency0, poolKey.currency1,
      poolKey.fee, poolKey.tickSpacing, poolKey.hooks
    );
    console.log("Pool ID:", poolId);

    const slot0 = await poolMgr.getSlot0(poolId).catch(() => null);
    if (slot0 && slot0.sqrtPriceX96 > 0n) {
      const existingPrice = (Number(slot0.sqrtPriceX96) / 2**96) ** 2;
      const existingTxmUsd = ethUsd / existingPrice;
      console.log(`Pool already initialized!`);
      console.log(`  sqrtPriceX96: ${slot0.sqrtPriceX96}`);
      console.log(`  Current price: $${existingTxmUsd.toFixed(6)}/TXM`);
      console.log("\nPool is ready. Add liquidity via Uniswap UI:");
      console.log(`  https://app.uniswap.org/add/ETH/${WTXM}/${FEE}?chain=optimism`);
      return;
    }
  } catch (e) {
    // Pool not yet initialized, proceed
    console.log("Pool not yet initialized — proceeding.");
  }

  // ── Initialize pool ───────────────────────────────────────────────────────
  console.log("\nInitializing pool...");
  console.log("(This sets the initial price — no liquidity added yet.)");

  const tx = await pm.initializePool(
    [poolKey.currency0, poolKey.currency1, poolKey.fee, poolKey.tickSpacing, poolKey.hooks],
    sqrtPriceX96,
    { value: 0 }
  );

  console.log("Tx submitted:", tx.hash);
  const receipt = await tx.wait();
  console.log("Confirmed in block:", receipt.blockNumber);

  console.log("\n══════════════════════════════════════════════════════");
  console.log("  Pool initialized!");
  console.log("  wTXM/ETH @ $" + TXM_USD_TARGET + " initial price");
  console.log("\n  Add liquidity (Uniswap UI):");
  console.log(`  https://app.uniswap.org/add/ETH/${WTXM}/${FEE}?chain=optimism`);
  console.log("\n  wTXM contract:");
  console.log(`  https://optimistic.etherscan.io/address/${WTXM}`);
  console.log("══════════════════════════════════════════════════════\n");

  // Save result
  const result = {
    network:       "op-mainnet",
    timestamp:     new Date().toISOString(),
    poolManager:   POOL_MANAGER,
    positionManager: POSITION_MANAGER,
    currency0:     "ETH (native)",
    currency1:     WTXM,
    fee:           FEE,
    tickSpacing:   TICK_SPACING,
    hooks:         HOOKS,
    sqrtPriceX96:  sqrtPriceX96.toString(),
    ethUsdAtInit:  ethUsd,
    txmUsdTarget:  TXM_USD_TARGET,
    initTx:        tx.hash,
    initBlock:     receipt.blockNumber,
    uniswapAddUrl: `https://app.uniswap.org/add/ETH/${WTXM}/${FEE}?chain=optimism`,
  };

  const { writeFileSync } = await import("fs");
  const outPath = new URL("../deployments/v4-pool-op-mainnet.json", import.meta.url).pathname;
  writeFileSync(outPath, JSON.stringify(result, null, 2));
  console.log("Saved to deployments/v4-pool-op-mainnet.json");
}

main().catch(e => { console.error(e); process.exit(1); });
