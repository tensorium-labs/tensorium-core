/**
 * create-v3-pool-v2.js
 * Uses NonfungiblePositionManager.createAndInitializePoolIfNecessary
 * instead of Factory.createPool — more reliable with Hardhat ethers v6.
 */
import hardhat from "hardhat";
const { ethers, network } = hardhat;

const WTXM    = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const WETH9   = "0x4200000000000000000000000000000000000006";
const NPM     = "0xC36442b4a4522E871399CD717aBDD847Ab11FE88"; // NonfungiblePositionManager
const FACTORY = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
const CHAINLINK_ETH_USD = "0x13e3Ee699D1909E989722E753853AE30b17e08c5";
const FEE     = 10000;
const TXM_USD = 0.0005;

const NPM_ABI = [
  `function createAndInitializePoolIfNecessary(
      address token0,
      address token1,
      uint24  fee,
      uint160 sqrtPriceX96
   ) external payable returns (address pool)`,
];

const FACTORY_ABI = [
  "function getPool(address,address,uint24) view returns (address)",
];

const POOL_ABI = [
  "function slot0() view returns (uint160 sqrtPriceX96,int24 tick,uint16,uint16,uint16,uint8,bool)",
  "function token0() view returns (address)",
  "function token1() view returns (address)",
];

const CHAINLINK_ABI = [
  "function latestAnswer() view returns (int256)",
  "function decimals() view returns (uint8)",
];

function calcSqrtPriceX96(isToken0WTXM, txmPerEth) {
  // price = token1/token0
  // if token0=wTXM: price = WETH/wTXM = 1/txmPerEth  (tiny)
  // if token0=WETH: price = wTXM/WETH = txmPerEth     (large)
  const priceRatio = isToken0WTXM ? (1 / txmPerEth) : txmPerEth;
  // sqrtPriceX96 = sqrt(priceRatio) * 2^96
  // Both 18 decimals → no decimal adjustment needed
  const Q96 = BigInt("79228162514264337593543950336"); // 2^96
  // Use float for sqrt then convert to bigint
  const sqrtPrice = Math.sqrt(priceRatio);
  // To preserve precision: sqrtPrice * 2^96
  const sqrtQ96Float = sqrtPrice * Number(Q96);
  return BigInt(Math.round(sqrtQ96Float));
}

async function main() {
  const [deployer] = await ethers.getSigners();
  const bal = await ethers.provider.getBalance(deployer.address);
  console.log("Deployer:", deployer.address);
  console.log("ETH:", ethers.formatEther(bal));

  // Price
  const feed = new ethers.Contract(CHAINLINK_ETH_USD, CHAINLINK_ABI, ethers.provider);
  const [ans, dec] = await Promise.all([feed.latestAnswer(), feed.decimals()]);
  const ethUsd = Number(ans) / 10 ** Number(dec);
  const txmPerEth = ethUsd / TXM_USD;
  console.log(`ETH/USD: $${ethUsd.toFixed(2)} | TXM target: $${TXM_USD} | wTXM/ETH: ${txmPerEth.toFixed(0)}`);

  // Determine token ordering
  const token0 = WTXM.toLowerCase() < WETH9.toLowerCase() ? WTXM : WETH9;
  const token1 = WTXM.toLowerCase() < WETH9.toLowerCase() ? WETH9 : WTXM;
  const isToken0WTXM = token0.toLowerCase() === WTXM.toLowerCase();
  console.log("token0:", token0, isToken0WTXM ? "(wTXM)" : "(WETH)");
  console.log("token1:", token1, isToken0WTXM ? "(WETH)" : "(wTXM)");

  const sqrtPriceX96 = calcSqrtPriceX96(isToken0WTXM, txmPerEth);
  console.log("sqrtPriceX96:", sqrtPriceX96.toString());

  // Check existing
  const factory = new ethers.Contract(FACTORY, FACTORY_ABI, ethers.provider);
  const existing = await factory.getPool(WTXM, WETH9, FEE);
  if (existing !== ethers.ZeroAddress) {
    const pool = new ethers.Contract(existing, POOL_ABI, ethers.provider);
    const slot0 = await pool.slot0();
    if (slot0.sqrtPriceX96 > 0n) {
      console.log("Pool already initialized:", existing);
      console.log("Add liquidity: https://app.uniswap.org/add/" + WTXM + "/ETH/10000?chain=optimism");
      return;
    }
  }

  // createAndInitializePoolIfNecessary
  console.log("\nCalling createAndInitializePoolIfNecessary...");
  const npm = new ethers.Contract(NPM, NPM_ABI, deployer);

  // Static call to check
  try {
    const poolAddr = await npm.createAndInitializePoolIfNecessary.staticCall(
      token0, token1, FEE, sqrtPriceX96
    );
    console.log("staticCall ok — pool will be at:", poolAddr);
  } catch (e) {
    console.error("staticCall failed:", e.message.slice(0, 120));
    // Try to decode revert
  }

  const tx = await npm.createAndInitializePoolIfNecessary(
    token0, token1, FEE, sqrtPriceX96,
    { gasLimit: 1_000_000n }
  );
  console.log("Tx:", tx.hash);
  const receipt = await tx.wait();
  console.log("Status:", receipt.status ? "✅ Success" : "❌ Reverted");
  console.log("Gas used:", receipt.gasUsed.toString());

  if (receipt.status) {
    const poolAddr = await factory.getPool(WTXM, WETH9, FEE);
    console.log("\n✅ Pool:", poolAddr);
    console.log("Add liquidity: https://app.uniswap.org/add/" + WTXM + "/ETH/10000?chain=optimism");
    console.log("Pool info: https://info.uniswap.org/#/optimism/pools/" + poolAddr.toLowerCase());

    const { writeFileSync } = await import("fs");
    writeFileSync(
      new URL("../deployments/v3-pool-op-mainnet.json", import.meta.url).pathname,
      JSON.stringify({ network:"op-mainnet", timestamp:new Date().toISOString(),
        factory:FACTORY, pool:poolAddr, token0, token1, fee:FEE,
        sqrtPriceX96:sqrtPriceX96.toString(), ethUsdAtInit:ethUsd, txmUsdTarget:TXM_USD,
        tx:receipt.hash,
        addLiquidityUrl: `https://app.uniswap.org/add/${WTXM}/ETH/10000?chain=optimism`,
      }, null, 2)
    );
    console.log("Saved → v3-pool-op-mainnet.json");
  }
}

main().catch(e => { console.error(e.message || e); process.exit(1); });
