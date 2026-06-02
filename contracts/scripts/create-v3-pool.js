/**
 * create-v3-pool.js
 *
 * Creates and initializes a Uniswap V3 wTXM/WETH 1% pool on Optimism mainnet.
 * - Creates pool via Factory.createPool()
 * - Initializes price via Pool.initialize(sqrtPriceX96)
 * - No liquidity added here — add via Uniswap UI once wTXM is in circulation
 *
 * Initial price: 1 TXM = $0.0005 USD (live ETH/USD from Chainlink)
 *
 * Usage:
 *   npx hardhat run scripts/create-v3-pool.js --network op-mainnet
 */

import hardhat from "hardhat";
const { ethers, network } = hardhat;

// ── Addresses (Optimism mainnet) ──────────────────────────────────────────────
const WTXM             = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const WETH9            = "0x4200000000000000000000000000000000000006";
const V3_FACTORY       = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
const CHAINLINK_ETH_USD = "0x13e3Ee699D1909E989722E753853AE30b17e08c5";
const FEE              = 10000;   // 1% — protects LPs on thin liquidity
const TXM_USD_TARGET   = 0.0005; // $0.0005 per TXM initial price

// ── ABIs ──────────────────────────────────────────────────────────────────────
const FACTORY_ABI = [
  "function createPool(address tokenA, address tokenB, uint24 fee) external returns (address)",
  "function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address)",
  "function feeAmountTickSpacing(uint24) view returns (int24)",
];

const POOL_ABI = [
  "function initialize(uint160 sqrtPriceX96) external",
  "function slot0() external view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked)",
  "function token0() external view returns (address)",
  "function token1() external view returns (address)",
];

const CHAINLINK_ABI = [
  "function latestAnswer() external view returns (int256)",
  "function decimals() external view returns (uint8)",
];

// ── Price helpers ─────────────────────────────────────────────────────────────
async function getEthUsd(provider) {
  const feed = new ethers.Contract(CHAINLINK_ETH_USD, CHAINLINK_ABI, provider);
  const [answer, dec] = await Promise.all([feed.latestAnswer(), feed.decimals()]);
  return Number(answer) / 10 ** Number(dec);
}

function calcSqrtPriceX96(token0, token1, txmPerEth) {
  // In V3: price = amount of token1 per token0
  // token0 < token1 by address sort
  // WETH = 0x4200... (lower address), wTXM = 0x2e71... (higher address)
  // Wait: 0x1F... < 0x2e... < 0x42... ? No: 0x1F < 0x2e < 0x4200
  // token0 = lower address = wTXM (0x2e71) or WETH (0x4200)?
  // 0x2e71... < 0x4200... so token0 = wTXM, token1 = WETH
  // price = WETH per wTXM = 1/txmPerEth (ETH per TXM)

  const isToken0WTXM = token0.toLowerCase() === WTXM.toLowerCase();
  let priceRatio; // token1 per token0
  if (isToken0WTXM) {
    // price = WETH/wTXM = (1/txmPerEth) — small number
    priceRatio = 1 / txmPerEth;
  } else {
    // price = wTXM/WETH = txmPerEth — large number
    priceRatio = txmPerEth;
  }

  // sqrtPriceX96 = sqrt(price) * 2^96
  // Both tokens have 18 decimals so ratio is exact
  const Q96 = 2n ** 96n;
  // Use BigInt math for precision
  // priceRatio might be very small (e.g. 0.00000025 ETH per TXM)
  // Scale up: priceRatio * 1e18 to avoid float loss
  const SCALE = 10n ** 18n;
  const priceScaled = BigInt(Math.round(priceRatio * 1e18)); // priceRatio * 1e18
  // sqrtPriceX96 = sqrt(priceScaled / 1e18) * Q96
  //              = sqrt(priceScaled) * Q96 / sqrt(1e18)
  // sqrt(1e18) = 1e9
  const sqrtPriceX96 = BigInt(Math.round(Math.sqrt(Number(priceScaled)) * Number(Q96) / 1e9));
  return sqrtPriceX96;
}

// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
  const [deployer] = await ethers.getSigners();
  const balance = await ethers.provider.getBalance(deployer.address);

  console.log("\n══════════════════════════════════════════════════════");
  console.log("  Tensorium — Uniswap V3 wTXM/WETH Pool");
  console.log("══════════════════════════════════════════════════════\n");
  console.log("Network:  ", network.name);
  console.log("Deployer: ", deployer.address);
  console.log("ETH:      ", ethers.formatEther(balance));

  if (balance < ethers.parseEther("0.001")) {
    throw new Error("Need at least 0.001 ETH for gas.");
  }

  const factory = new ethers.Contract(V3_FACTORY, FACTORY_ABI, deployer);

  // ── Check if pool exists ──────────────────────────────────────────────────
  const existingPool = await factory.getPool(WTXM, WETH9, FEE);
  console.log("\n── Pool Check ───────────────────────────────────────");
  if (existingPool !== ethers.ZeroAddress) {
    console.log("Pool already exists:", existingPool);
    const pool = new ethers.Contract(existingPool, POOL_ABI, ethers.provider);
    const slot0 = await pool.slot0();
    if (slot0.sqrtPriceX96 > 0n) {
      console.log("Pool already initialized!");
      console.log("\nAdd liquidity via Uniswap UI:");
      console.log(`  https://app.uniswap.org/add/${WTXM}/ETH/10000?chain=optimism`);
      return;
    }
    // Pool exists but not initialized — fall through to initialize
    console.log("Pool exists but not initialized — initializing...");
    const ethUsd = await getEthUsd(ethers.provider);
    const txmPerEth = ethUsd / TXM_USD_TARGET;
    const token0 = await pool.token0();
    const token1 = await pool.token1();
    console.log("token0:", token0, token0.toLowerCase()===WTXM.toLowerCase()?"(wTXM)":"(WETH)");
    console.log("token1:", token1);
    const sqrtPriceX96 = calcSqrtPriceX96(token0, token1, txmPerEth);
    console.log("sqrtPriceX96:", sqrtPriceX96.toString());
    const poolWrite = new ethers.Contract(existingPool, POOL_ABI, deployer);
    const tx = await poolWrite.initialize(sqrtPriceX96, { gasLimit: 200_000n });
    const receipt = await tx.wait();
    console.log("✅ Pool initialized. Tx:", receipt.hash);
    console.log("\nAdd liquidity:");
    console.log(`  https://app.uniswap.org/add/${WTXM}/ETH/10000?chain=optimism`);
    return;
  }

  // ── Fetch price ───────────────────────────────────────────────────────────
  const ethUsd = await getEthUsd(ethers.provider);
  const txmPerEth = ethUsd / TXM_USD_TARGET;

  console.log("\n── Price ────────────────────────────────────────────");
  console.log(`ETH/USD (Chainlink):  $${ethUsd.toFixed(2)}`);
  console.log(`TXM target price:     $${TXM_USD_TARGET}`);
  console.log(`wTXM per ETH:         ${txmPerEth.toFixed(0)}`);

  // ── Create pool ───────────────────────────────────────────────────────────
  console.log("\n── Creating pool ────────────────────────────────────");
  const tx1 = await factory.createPool(WTXM, WETH9, FEE, { gasLimit: 500_000n });
  const receipt1 = await tx1.wait();
  console.log("Pool created. Tx:", receipt1.hash);

  const poolAddr = await factory.getPool(WTXM, WETH9, FEE);
  console.log("Pool address:", poolAddr);

  // ── Initialize pool price ─────────────────────────────────────────────────
  console.log("\n── Initializing price ───────────────────────────────");
  const pool = new ethers.Contract(poolAddr, POOL_ABI, deployer);
  const token0 = await pool.token0();
  const token1 = await pool.token1();
  console.log("token0:", token0, token0.toLowerCase()===WTXM.toLowerCase()?"(wTXM)":"(WETH)");
  console.log("token1:", token1, token1.toLowerCase()===WTXM.toLowerCase()?"(wTXM)":"(WETH)");

  const sqrtPriceX96 = calcSqrtPriceX96(token0, token1, txmPerEth);
  console.log("sqrtPriceX96:", sqrtPriceX96.toString());

  const tx2 = await pool.initialize(sqrtPriceX96, { gasLimit: 200_000n });
  const receipt2 = await tx2.wait();
  console.log("Price initialized. Tx:", receipt2.hash);

  // Verify
  const slot0 = await pool.slot0();
  console.log("slot0.sqrtPriceX96:", slot0.sqrtPriceX96.toString());
  console.log("slot0.tick:", slot0.tick.toString());

  console.log("\n══════════════════════════════════════════════════════");
  console.log("  ✅ wTXM/WETH 1% pool live on Uniswap V3 (Optimism)");
  console.log("══════════════════════════════════════════════════════");
  console.log("Pool:    ", poolAddr);
  console.log("Price:    1 TXM ≈ $" + TXM_USD_TARGET, `(${txmPerEth.toFixed(0)} wTXM/ETH)`);
  console.log("\nAdd liquidity (when wTXM is in circulation):");
  console.log(`  https://app.uniswap.org/add/${WTXM}/ETH/10000?chain=optimism`);
  console.log("\nPool on Uniswap:");
  console.log(`  https://info.uniswap.org/#/optimism/pools/${poolAddr.toLowerCase()}`);

  const { writeFileSync } = await import("fs");
  writeFileSync(
    new URL("../deployments/v3-pool-op-mainnet.json", import.meta.url).pathname,
    JSON.stringify({
      network: "op-mainnet",
      timestamp: new Date().toISOString(),
      factory: V3_FACTORY,
      pool: poolAddr,
      token0, token1,
      fee: FEE,
      sqrtPriceX96: sqrtPriceX96.toString(),
      ethUsdAtInit: ethUsd,
      txmUsdTarget: TXM_USD_TARGET,
      createTx: receipt1.hash,
      initTx: receipt2.hash,
      uniswapAddUrl: `https://app.uniswap.org/add/${WTXM}/ETH/10000?chain=optimism`,
    }, null, 2)
  );
  console.log("\nSaved → deployments/v3-pool-op-mainnet.json");
}

main().catch(e => { console.error(e.message || e); process.exit(1); });
