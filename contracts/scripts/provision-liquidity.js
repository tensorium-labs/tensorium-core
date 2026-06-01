/**
 * provision-liquidity.js
 *
 * Creates a Uniswap v3 wTXM/WETH pool on Optimism mainnet and adds
 * initial liquidity. Run after top-upping the deployer wallet with ETH.
 *
 * Pre-conditions:
 *   - DEPLOYER_PRIVATE_KEY in .env (same key that owns bridge controller)
 *   - OP_MAINNET_RPC_URL in .env (or uses public endpoint)
 *   - Enough ETH in deployer wallet on Optimism mainnet:
 *       ~0.002 ETH gas + however much ETH you want in the pool
 *
 * Config (edit the PARAMS block below or pass as env vars):
 *   INITIAL_PRICE_ETH_PER_WTXM  — e.g. "0.0001"  (1 wTXM = 0.0001 ETH)
 *   WTXM_LIQUIDITY_AMOUNT       — wTXM to deposit, e.g. "50000"
 *   ETH_LIQUIDITY_AMOUNT        — ETH to pair (must match price × wTXM)
 *   PRICE_RANGE_MULTIPLIER      — full-range is 0 (uses TICK_LOWER/UPPER_MAX)
 *   FEE_TIER                    — 10000 = 1% (recommended for new tokens)
 *
 * Usage:
 *   npx hardhat run scripts/provision-liquidity.js --network op-mainnet
 */

import hardhat from "hardhat";
const { ethers, network } = hardhat;

// ── Uniswap v3 Optimism mainnet addresses ────────────────────────────────────
const UNISWAP_FACTORY       = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
const POSITION_MANAGER      = "0xC36442b4a4522E871399CD717aBDD847Ab11FE88";
const WETH_OPTIMISM         = "0x4200000000000000000000000000000000000006";

// ── Tensorium contracts ───────────────────────────────────────────────────────
const WTXM_ADDRESS          = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const BRIDGE_CONTROLLER     = "0x4b31C557AD64609B975610812273BF82F1475384";

// ── Pool parameters (override with env vars) ─────────────────────────────────
const PARAMS = {
  // Price: how many ETH per 1 wTXM
  // 0.0001 ETH/wTXM = ~$0.30 at ETH=$3000. Adjust before running.
  initialPriceEthPerWtxm: process.env.INITIAL_PRICE_ETH_PER_WTXM || "0.0001",

  // How much wTXM to put in the pool
  wtxmAmount:             process.env.WTXM_LIQUIDITY_AMOUNT       || "50000",

  // Fee tier: 10000 = 1% (standard for new/low-liquidity tokens)
  feeTier:     parseInt(process.env.FEE_TIER                      || "10000"),

  // Tick spacing for 1% fee tier is 200. Full range: ±887200 (nearest 200)
  tickLower:   -887200,
  tickUpper:    887200,
};

// ── ABI fragments ─────────────────────────────────────────────────────────────
const FACTORY_ABI = [
  "function getPool(address,address,uint24) external view returns (address)",
  "function createPool(address,address,uint24) external returns (address)",
];

const POOL_ABI = [
  "function initialize(uint160 sqrtPriceX96) external",
  "function slot0() external view returns (uint160 sqrtPriceX96,int24 tick,uint16,uint16,uint16,uint8,bool)",
  "function token0() external view returns (address)",
  "function token1() external view returns (address)",
];

const PM_ABI = [
  `function mint(tuple(
    address token0,
    address token1,
    uint24  fee,
    int24   tickLower,
    int24   tickUpper,
    uint256 amount0Desired,
    uint256 amount1Desired,
    uint256 amount0Min,
    uint256 amount1Min,
    address recipient,
    uint256 deadline
  ) params) external payable returns (
    uint256 tokenId,
    uint128 liquidity,
    uint256 amount0,
    uint256 amount1
  )`,
];

const ERC20_ABI = [
  "function approve(address,uint256) external returns (bool)",
  "function balanceOf(address) external view returns (uint256)",
  "function totalSupply() external view returns (uint256)",
];

const WETH_ABI = [
  "function deposit() external payable",
  "function approve(address,uint256) external returns (bool)",
  "function balanceOf(address) external view returns (uint256)",
];

const BRIDGE_CONTROLLER_ABI = [
  "function mintFromTensoriumDeposit(bytes32,bytes32,address,uint256) external",
  "function operators(address) external view returns (bool)",
];

// ── Math helpers ──────────────────────────────────────────────────────────────

// sqrtPriceX96 = sqrt(price_token1_per_token0) * 2^96
// price_token1_per_token0 = (token1_amount / token0_amount) in raw units
// Both wTXM and WETH have 18 decimals so the 1e18 factors cancel.
function calcSqrtPriceX96(priceToken1PerToken0) {
  // Use BigInt math with 18-decimal precision
  const Q96 = 2n ** 96n;
  const PRECISION = 10n ** 18n;
  // price as integer * PRECISION
  const priceScaled = BigInt(Math.round(Number(priceToken1PerToken0) * 1e18));
  // sqrt(price * PRECISION^2) / PRECISION * Q96
  // = sqrt(priceScaled) / sqrt(PRECISION) * Q96
  // Use integer sqrt: babylonian method
  const target = priceScaled * Q96 * Q96;  // price * (2^96)^2
  let x = BigInt(Math.floor(Math.sqrt(Number(target / PRECISION)) * Math.sqrt(Number(PRECISION))));
  // Refine with Newton
  for (let i = 0; i < 10; i++) {
    const next = (x + target / x) / 2n;
    if (next >= x) break;
    x = next;
  }
  return x;
}

function encodeBridgeEventId(nonce, address) {
  // keccak256(abi.encodePacked(nonce, address)) as bytes32
  return ethers.keccak256(
    ethers.AbiCoder.defaultAbiCoder().encode(
      ["uint256", "address"],
      [nonce, address]
    )
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const [deployer] = await ethers.getSigners();
  const deployerAddr = deployer.address;

  console.log("\n═══════════════════════════════════════════════════════");
  console.log("  Tensorium wTXM/WETH Uniswap v3 Liquidity Provisioning");
  console.log("═══════════════════════════════════════════════════════\n");
  console.log("Network:     ", network.name);
  console.log("Deployer:    ", deployerAddr);

  const ethBalance = await ethers.provider.getBalance(deployerAddr);
  console.log("ETH balance: ", ethers.formatEther(ethBalance), "ETH");

  // ── Step 1: Check / mint wTXM ───────────────────────────────────────────────
  const wtxm = new ethers.Contract(WTXM_ADDRESS, ERC20_ABI, deployer);
  const controller = new ethers.Contract(BRIDGE_CONTROLLER, BRIDGE_CONTROLLER_ABI, deployer);

  const wtxmBalance = await wtxm.balanceOf(deployerAddr);
  const wtxmDesired = ethers.parseEther(PARAMS.wtxmAmount);

  console.log("\n── wTXM ────────────────────────────────────────────────");
  console.log("wTXM balance:", ethers.formatEther(wtxmBalance), "wTXM");
  console.log("wTXM desired:", PARAMS.wtxmAmount, "wTXM");

  if (wtxmBalance < wtxmDesired) {
    const toMint = wtxmDesired - wtxmBalance;
    console.log(`Minting ${ethers.formatEther(toMint)} wTXM via bridge controller...`);

    const isOperator = await controller.operators(deployerAddr);
    if (!isOperator) {
      throw new Error(
        `Deployer ${deployerAddr} is not an operator on the bridge controller.\n` +
        `The owner must call setOperator(deployer, true) first.`
      );
    }

    // Use a deterministic bridge event ID for the bootstrap mint
    const bridgeEventId = ethers.keccak256(
      ethers.toUtf8Bytes(`bootstrap-liquidity-${deployerAddr}-${Date.now()}`)
    );
    const tensoriumTxid = ethers.keccak256(ethers.toUtf8Bytes("bootstrap-liquidity-txid"));

    const tx = await controller.mintFromTensoriumDeposit(
      bridgeEventId,
      tensoriumTxid,
      deployerAddr,
      toMint
    );
    await tx.wait();
    console.log("Minted wTXM. Tx:", tx.hash);
  } else {
    console.log("Sufficient wTXM balance — skipping mint.");
  }

  // ── Step 2: Sort tokens ─────────────────────────────────────────────────────
  const token0Addr = WTXM_ADDRESS.toLowerCase() < WETH_OPTIMISM.toLowerCase()
    ? WTXM_ADDRESS : WETH_OPTIMISM;
  const token1Addr = WTXM_ADDRESS.toLowerCase() < WETH_OPTIMISM.toLowerCase()
    ? WETH_OPTIMISM : WTXM_ADDRESS;

  const wtxmIsToken0 = token0Addr.toLowerCase() === WTXM_ADDRESS.toLowerCase();
  console.log(`\n── Token order ────────────────────────────────────────`);
  console.log("token0:", token0Addr, wtxmIsToken0 ? "(wTXM)" : "(WETH)");
  console.log("token1:", token1Addr, wtxmIsToken0 ? "(WETH)" : "(wTXM)");

  // price in Uniswap terms: token1/token0
  const priceEthPerWtxm = parseFloat(PARAMS.initialPriceEthPerWtxm);
  const priceToken1PerToken0 = wtxmIsToken0 ? priceEthPerWtxm : (1 / priceEthPerWtxm);
  const sqrtPriceX96 = calcSqrtPriceX96(priceToken1PerToken0);

  console.log(`\nInitial price: 1 wTXM = ${PARAMS.initialPriceEthPerWtxm} ETH`);
  console.log(`sqrtPriceX96:  ${sqrtPriceX96.toString()}`);

  // ── Step 3: Create pool if needed ──────────────────────────────────────────
  const factory = new ethers.Contract(UNISWAP_FACTORY, FACTORY_ABI, deployer);
  let poolAddress = await factory.getPool(token0Addr, token1Addr, PARAMS.feeTier);

  console.log(`\n── Pool (fee ${PARAMS.feeTier / 100}%) ──────────────────────────────────────`);
  if (poolAddress === ethers.ZeroAddress) {
    console.log("Pool does not exist. Creating...");
    const tx = await factory.createPool(token0Addr, token1Addr, PARAMS.feeTier);
    await tx.wait();
    poolAddress = await factory.getPool(token0Addr, token1Addr, PARAMS.feeTier);
    console.log("Pool created:", poolAddress);
  } else {
    console.log("Pool exists:", poolAddress);
  }

  // ── Step 4: Initialize price if needed ─────────────────────────────────────
  const pool = new ethers.Contract(poolAddress, POOL_ABI, deployer);
  const slot0 = await pool.slot0();

  if (slot0.sqrtPriceX96 === 0n) {
    console.log("Initializing pool price...");
    const tx = await pool.initialize(sqrtPriceX96);
    await tx.wait();
    console.log("Pool initialized. Tx:", tx.hash);
  } else {
    console.log("Pool already initialized. sqrtPriceX96:", slot0.sqrtPriceX96.toString());
  }

  // ── Step 5: Wrap ETH ────────────────────────────────────────────────────────
  const ethDesired = ethers.parseEther(
    String(parseFloat(PARAMS.wtxmAmount) * priceEthPerWtxm)
  );
  console.log(`\n── WETH ───────────────────────────────────────────────`);
  console.log(`Wrapping ${ethers.formatEther(ethDesired)} ETH → WETH...`);

  const weth = new ethers.Contract(WETH_OPTIMISM, WETH_ABI, deployer);
  const wethBalance = await weth.balanceOf(deployerAddr);
  if (wethBalance < ethDesired) {
    const toWrap = ethDesired - wethBalance;
    const tx = await weth.deposit({ value: toWrap });
    await tx.wait();
    console.log("Wrapped ETH. Tx:", tx.hash);
  } else {
    console.log("Sufficient WETH balance — skipping wrap.");
  }

  // ── Step 6: Approve both tokens ────────────────────────────────────────────
  console.log("\n── Approvals ──────────────────────────────────────────");
  const MAX = ethers.MaxUint256;

  const approveTxWtxm = await wtxm.approve(POSITION_MANAGER, MAX);
  await approveTxWtxm.wait();
  console.log("wTXM approved. Tx:", approveTxWtxm.hash);

  const approveTxWeth = await weth.approve(POSITION_MANAGER, MAX);
  await approveTxWeth.wait();
  console.log("WETH approved. Tx:", approveTxWeth.hash);

  // ── Step 7: Mint position ──────────────────────────────────────────────────
  console.log("\n── Mint LP position ───────────────────────────────────");
  const pm = new ethers.Contract(POSITION_MANAGER, PM_ABI, deployer);

  const amount0Desired = wtxmIsToken0 ? wtxmDesired : ethDesired;
  const amount1Desired = wtxmIsToken0 ? ethDesired  : wtxmDesired;

  const mintParams = {
    token0:          token0Addr,
    token1:          token1Addr,
    fee:             PARAMS.feeTier,
    tickLower:       PARAMS.tickLower,
    tickUpper:       PARAMS.tickUpper,
    amount0Desired,
    amount1Desired,
    amount0Min:      0n,
    amount1Min:      0n,
    recipient:       deployerAddr,
    deadline:        BigInt(Math.floor(Date.now() / 1000) + 3600),
  };

  console.log("amount0Desired:", ethers.formatEther(amount0Desired), wtxmIsToken0 ? "wTXM" : "WETH");
  console.log("amount1Desired:", ethers.formatEther(amount1Desired), wtxmIsToken0 ? "WETH" : "wTXM");

  const mintTx = await pm.mint(mintParams);
  const receipt = await mintTx.wait();
  console.log("LP position minted. Tx:", mintTx.hash);

  // Parse tokenId from Transfer event
  const transferEvent = receipt.logs.find(
    l => l.topics[0] === ethers.id("Transfer(address,address,uint256)") &&
         l.address.toLowerCase() === POSITION_MANAGER.toLowerCase()
  );
  if (transferEvent) {
    const tokenId = BigInt(transferEvent.topics[3]);
    console.log("NFT Position ID:", tokenId.toString());
  }

  console.log("\n═══════════════════════════════════════════════════════");
  console.log("  Done! wTXM/WETH pool is live on Uniswap v3 Optimism.");
  console.log("  Pool address:", poolAddress);
  console.log("  View on Uniswap: https://app.uniswap.org/pools");
  console.log("═══════════════════════════════════════════════════════\n");
}

main().catch(e => { console.error(e); process.exit(1); });
