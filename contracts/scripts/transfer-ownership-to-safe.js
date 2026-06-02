/**
 * transfer-ownership-to-safe.js
 *
 * Safe already exists at 0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9
 * This script:
 *   1. Calls transferOwnership(safe) on wTXM and Controller
 *   2. Executes acceptOwnership() on both FROM the Safe
 */

import hardhat from "hardhat";
const { ethers } = hardhat;

const SAFE       = "0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9";
const WTXM       = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const CONTROLLER = "0x4b31C557AD64609B975610812273BF82F1475384";

const SAFE_ABI = [
  "function nonce() view returns (uint256)",
  "function getOwners() view returns (address[])",
  "function getThreshold() view returns (uint256)",
  "function getTransactionHash(address to, uint256 value, bytes data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address refundReceiver, uint256 _nonce) view returns (bytes32)",
  "function execTransaction(address to, uint256 value, bytes data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address refundReceiver, bytes signatures) payable returns (bool)",
];

const OWNABLE2_ABI = [
  "function owner() view returns (address)",
  "function pendingOwner() view returns (address)",
  "function transferOwnership(address newOwner)",
  "function acceptOwnership()",
];

async function execSafeTx(safe, signer, to, calldata) {
  const nonce = await safe.nonce();
  const txHash = await safe.getTransactionHash(
    to, 0n, calldata, 0,
    0n, 0n, 0n,
    ethers.ZeroAddress, ethers.ZeroAddress,
    nonce
  );

  const sig = await signer.signMessage(ethers.getBytes(txHash));
  const sigBytes = ethers.getBytes(sig);
  // Safe v1.3.0: adjust v byte (27→31, 28→32) for eth_sign type
  sigBytes[64] = sigBytes[64] === 27 ? 31 : 32;

  const tx = await safe.execTransaction(
    to, 0n, calldata, 0,
    0n, 0n, 0n,
    ethers.ZeroAddress, ethers.ZeroAddress,
    ethers.hexlify(sigBytes),
    { gasLimit: 300_000n }
  );
  return tx.wait();
}

async function main() {
  const [deployer] = await ethers.getSigners();
  console.log("Deployer:", deployer.address);
  console.log("Safe:    ", SAFE);

  const safe = new ethers.Contract(SAFE, SAFE_ABI, deployer);
  const wtxm = new ethers.Contract(WTXM, OWNABLE2_ABI, deployer);
  const ctrl = new ethers.Contract(CONTROLLER, OWNABLE2_ABI, deployer);

  // Verify Safe setup
  const owners = await safe.getOwners();
  const threshold = await safe.getThreshold();
  console.log("Safe owners:", owners, "threshold:", threshold.toString());

  // Current owners
  console.log("\nwTXM.owner()       :", await wtxm.owner());
  console.log("wTXM.pendingOwner():", await wtxm.pendingOwner());
  console.log("Ctrl.owner()       :", await ctrl.owner());
  console.log("Ctrl.pendingOwner():", await ctrl.pendingOwner());

  const acceptData = new ethers.Interface(OWNABLE2_ABI)
    .encodeFunctionData("acceptOwnership");

  // ── wTXM ─────────────────────────────────────────────────────────────────
  const wtxmOwner = await wtxm.owner();
  const wtxmPending = await wtxm.pendingOwner();

  if (wtxmOwner.toLowerCase() === SAFE.toLowerCase()) {
    console.log("\n✅ wTXM already owned by Safe — skip");
  } else if (wtxmPending.toLowerCase() === SAFE.toLowerCase()) {
    console.log("\n[wTXM] pendingOwner already Safe — accepting...");
    const r = await execSafeTx(safe, deployer, WTXM, acceptData);
    console.log("✅ wTXM accepted. owner →", await wtxm.owner(), "tx:", r.hash);
  } else {
    console.log("\n[wTXM] transferOwnership →", SAFE);
    await (await wtxm.transferOwnership(SAFE)).wait();
    console.log("  pendingOwner →", await wtxm.pendingOwner());
    console.log("[wTXM] Safe accepts...");
    const r = await execSafeTx(safe, deployer, WTXM, acceptData);
    console.log("✅ wTXM.owner() →", await wtxm.owner(), "tx:", r.hash);
  }

  // ── Controller ───────────────────────────────────────────────────────────
  const ctrlOwner = await ctrl.owner();
  const ctrlPending = await ctrl.pendingOwner();

  if (ctrlOwner.toLowerCase() === SAFE.toLowerCase()) {
    console.log("✅ Controller already owned by Safe — skip");
  } else if (ctrlPending.toLowerCase() === SAFE.toLowerCase()) {
    console.log("\n[Controller] pendingOwner already Safe — accepting...");
    const r = await execSafeTx(safe, deployer, CONTROLLER, acceptData);
    console.log("✅ Controller accepted. owner →", await ctrl.owner(), "tx:", r.hash);
  } else {
    console.log("\n[Controller] transferOwnership →", SAFE);
    await (await ctrl.transferOwnership(SAFE)).wait();
    console.log("  pendingOwner →", await ctrl.pendingOwner());
    console.log("[Controller] Safe accepts...");
    const r = await execSafeTx(safe, deployer, CONTROLLER, acceptData);
    console.log("✅ Controller.owner() →", await ctrl.owner(), "tx:", r.hash);
  }

  // ── Final state ──────────────────────────────────────────────────────────
  console.log("\n══════════════════════════════════════════════════════");
  console.log("  Ownership transfer complete");
  console.log("══════════════════════════════════════════════════════");
  console.log("Safe:              ", SAFE);
  console.log("wTXM owner:        ", await wtxm.owner());
  console.log("Controller owner:  ", await ctrl.owner());
  console.log("Manage Safe: https://app.safe.global/home?safe=oeth:" + SAFE);

  const { writeFileSync } = await import("fs");
  writeFileSync(
    new URL("../deployments/safe-op-mainnet.json", import.meta.url).pathname,
    JSON.stringify({
      network: "op-mainnet",
      timestamp: new Date().toISOString(),
      safe: SAFE,
      owners: [deployer.address],
      threshold: 1,
      note: "1-of-1 Safe. Add signers B+C via app.safe.global to upgrade to 2-of-3.",
      wTXM: WTXM,
      controller: CONTROLLER,
      wTXM_owner: await wtxm.owner(),
      controller_owner: await ctrl.owner(),
      safeUrl: "https://app.safe.global/home?safe=oeth:" + SAFE,
    }, null, 2)
  );
  console.log("Saved → deployments/safe-op-mainnet.json");
}

main().catch(e => { console.error(e.message || e); process.exit(1); });
