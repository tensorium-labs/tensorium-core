/**
 * setup-safe-ownership.js
 *
 * 1. Creates a Gnosis Safe 1-of-1 on Optimism mainnet (deployer as sole owner).
 * 2. Calls transferOwnership(safe) on wTXM and Controller (Ownable2Step → sets pendingOwner).
 * 3. Executes acceptOwnership() on both contracts FROM the Safe (via execTransaction).
 *
 * After this script: deployer EOA no longer owns the contracts.
 * Ownership belongs to the Safe. Deployer can still act via Safe (1-of-1 threshold).
 * Add signer B and C later via app.safe.global to upgrade to 2-of-3.
 *
 * Usage:
 *   npx hardhat run scripts/setup-safe-ownership.js --network op-mainnet
 *
 * Required .env:
 *   DEPLOYER_PRIVATE_KEY
 */

import hardhat from "hardhat";
const { ethers, network } = hardhat;

// ── Addresses ─────────────────────────────────────────────────────────────────
const WTXM       = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const CONTROLLER = "0x4b31C557AD64609B975610812273BF82F1475384";

const SAFE_PROXY_FACTORY = "0xa6B71E26C5e0845f74c812102Ca7114b6a896AB2";
const SAFE_SINGLETON     = "0x3E5c63644E683549055b9Be8653de26E0B4CD36E";
const FALLBACK_HANDLER   = "0xf48f2B2d2a534e402487b3ee7C18c33Aec0Fe5e4";

// ── ABIs ──────────────────────────────────────────────────────────────────────
const FACTORY_ABI = [
  `function createProxyWithNonce(
      address _singleton,
      bytes   initializer,
      uint256 saltNonce
   ) returns (address proxy)`,
  `event ProxyCreation(address indexed proxy, address singleton)`,
];

const SAFE_ABI = [
  `function setup(
      address[] _owners,
      uint256   _threshold,
      address   to,
      bytes     data,
      address   fallbackHandler,
      address   paymentToken,
      uint256   payment,
      address   paymentReceiver
   )`,
  `function getOwners() view returns (address[])`,
  `function getThreshold() view returns (uint256)`,
  `function nonce() view returns (uint256)`,
  `function domainSeparator() view returns (bytes32)`,
  `function getTransactionHash(
      address to, uint256 value, bytes data, uint8 operation,
      uint256 safeTxGas, uint256 baseGas, uint256 gasPrice,
      address gasToken, address refundReceiver, uint256 _nonce
   ) view returns (bytes32)`,
  `function execTransaction(
      address to, uint256 value, bytes data, uint8 operation,
      uint256 safeTxGas, uint256 baseGas, uint256 gasPrice,
      address gasToken, address refundReceiver, bytes signatures
   ) payable returns (bool)`,
];

const OWNABLE2_ABI = [
  `function owner() view returns (address)`,
  `function pendingOwner() view returns (address)`,
  `function transferOwnership(address newOwner)`,
  `function acceptOwnership()`,
];

// ── Safe tx helper ────────────────────────────────────────────────────────────
async function execSafeTx(safe, signer, to, data) {
  const nonce = await safe.nonce();
  const txHash = await safe.getTransactionHash(
    to, 0n, data, 0,        // to, value, data, operation (Call=0)
    0n, 0n, 0n,             // safeTxGas, baseGas, gasPrice
    ethers.ZeroAddress,     // gasToken
    ethers.ZeroAddress,     // refundReceiver
    nonce
  );

  // Sign with deployer (owner of 1-of-1 Safe)
  const sig = await signer.signMessage(ethers.getBytes(txHash));
  // Convert v from 27/28 to 31/32 for Safe contract-approved signature format
  const sigBytes = ethers.getBytes(sig);
  sigBytes[64] = sigBytes[64] === 27 ? 31 : 32;
  const packedSig = ethers.hexlify(sigBytes);

  const tx = await safe.execTransaction(
    to, 0n, data, 0,
    0n, 0n, 0n,
    ethers.ZeroAddress,
    ethers.ZeroAddress,
    packedSig,
    { gasLimit: 300_000n }
  );
  return tx.wait();
}

// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
  const [deployer] = await ethers.getSigners();
  const deployerAddr = deployer.address;
  const balance = await ethers.provider.getBalance(deployerAddr);

  console.log("\n══════════════════════════════════════════════════════");
  console.log("  Tensorium — Gnosis Safe Setup + Ownership Transfer");
  console.log("══════════════════════════════════════════════════════\n");
  console.log("Network:   ", network.name);
  console.log("Deployer:  ", deployerAddr);
  console.log("ETH:       ", ethers.formatEther(balance));

  if (balance < ethers.parseEther("0.003")) {
    throw new Error("Need at least 0.003 ETH for gas.");
  }

  // ── 1. Create Safe ────────────────────────────────────────────────────────
  console.log("\n── Step 1: Create Gnosis Safe 1-of-1 ───────────────");
  const factory = new ethers.Contract(SAFE_PROXY_FACTORY, FACTORY_ABI, deployer);
  const safeIface = new ethers.Interface(SAFE_ABI);

  const initializer = safeIface.encodeFunctionData("setup", [
    [deployerAddr],       // owners
    1n,                   // threshold = 1
    ethers.ZeroAddress,   // to (no delegate call)
    "0x",                 // data
    FALLBACK_HANDLER,     // fallbackHandler
    ethers.ZeroAddress,   // paymentToken
    0n,                   // payment
    ethers.ZeroAddress,   // paymentReceiver
  ]);

  const saltNonce = BigInt(Date.now());

  // staticCall first to get deterministic address, then actually deploy
  const safeAddr = await factory.createProxyWithNonce.staticCall(SAFE_SINGLETON, initializer, saltNonce);
  const tx1 = await factory.createProxyWithNonce(SAFE_SINGLETON, initializer, saltNonce, { gasLimit: 500_000n });
  const receipt1 = await tx1.wait();
  if (!receipt1.status) throw new Error("Safe creation tx reverted");

  console.log("✅ Safe deployed at:", safeAddr);
  console.log("   Tx:", receipt1.hash);

  // Verify Safe setup
  const safe = new ethers.Contract(safeAddr, SAFE_ABI, deployer);
  const owners = await safe.getOwners();
  const threshold = await safe.getThreshold();
  console.log("   Owners:", owners);
  console.log("   Threshold:", threshold.toString(), "of", owners.length);

  // ── 2. transferOwnership → Safe ──────────────────────────────────────────
  console.log("\n── Step 2: transferOwnership → Safe ────────────────");
  const wtxm = new ethers.Contract(WTXM, OWNABLE2_ABI, deployer);
  const ctrl = new ethers.Contract(CONTROLLER, OWNABLE2_ABI, deployer);

  console.log("wTXM current owner:", await wtxm.owner());
  console.log("Controller current owner:", await ctrl.owner());

  const tx2a = await wtxm.transferOwnership(safeAddr);
  await tx2a.wait();
  console.log("✅ wTXM pendingOwner →", await wtxm.pendingOwner());

  const tx2b = await ctrl.transferOwnership(safeAddr);
  await tx2b.wait();
  console.log("✅ Controller pendingOwner →", await ctrl.pendingOwner());

  // ── 3. Safe.acceptOwnership on both ─────────────────────────────────────
  console.log("\n── Step 3: Safe accepts ownership of wTXM ──────────");
  const acceptData = new ethers.Interface(OWNABLE2_ABI)
    .encodeFunctionData("acceptOwnership");

  const r3a = await execSafeTx(safe, deployer, WTXM, acceptData);
  if (!r3a.status) throw new Error("acceptOwnership(wTXM) reverted");
  console.log("✅ wTXM ownership accepted by Safe. Tx:", r3a.hash);
  console.log("   wTXM.owner() →", await wtxm.owner());

  console.log("\n── Step 4: Safe accepts ownership of Controller ────");
  const r3b = await execSafeTx(safe, deployer, CONTROLLER, acceptData);
  if (!r3b.status) throw new Error("acceptOwnership(Controller) reverted");
  console.log("✅ Controller ownership accepted by Safe. Tx:", r3b.hash);
  console.log("   Controller.owner() →", await ctrl.owner());

  // ── Summary ───────────────────────────────────────────────────────────────
  console.log("\n══════════════════════════════════════════════════════");
  console.log("  DONE");
  console.log("══════════════════════════════════════════════════════");
  console.log("Safe address  :", safeAddr);
  console.log("Owners        :", owners.join(", "));
  console.log("Threshold     : 1-of-1 (add B+C via app.safe.global later)");
  console.log("wTXM owner    :", await wtxm.owner());
  console.log("Controller owner:", await ctrl.owner());
  console.log("\nManage Safe: https://app.safe.global/home?safe=oeth:" + safeAddr);

  // Save result
  const { writeFileSync } = await import("fs");
  const out = {
    network: "op-mainnet",
    timestamp: new Date().toISOString(),
    safe: safeAddr,
    owners: [deployerAddr],
    threshold: 1,
    wTXM: WTXM,
    controller: CONTROLLER,
    safeUrl: "https://app.safe.global/home?safe=oeth:" + safeAddr,
  };
  const outPath = new URL("../deployments/safe-op-mainnet.json", import.meta.url).pathname;
  writeFileSync(outPath, JSON.stringify(out, null, 2));
  console.log("\nSaved → deployments/safe-op-mainnet.json");
}

main().catch(e => { console.error(e); process.exit(1); });
