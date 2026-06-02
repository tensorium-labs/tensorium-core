/**
 * add-safe-signers.js
 * Add 2 new owners to the Safe and upgrade threshold to 2-of-3.
 */
import hardhat from "hardhat";
const { ethers } = hardhat;

const SAFE      = "0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9";
const SIGNER_B  = "0x50B0EF4d9842aeeFE503087ca7250c28e3D2f8A3";
const SIGNER_C  = "0x950f0157848Fe8047AB464c8382Ea76f128828dF";

const SAFE_ABI = [
  "function nonce() view returns (uint256)",
  "function getOwners() view returns (address[])",
  "function getThreshold() view returns (uint256)",
  "function getTransactionHash(address to, uint256 value, bytes data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address refundReceiver, uint256 _nonce) view returns (bytes32)",
  "function execTransaction(address to, uint256 value, bytes data, uint8 operation, uint256 safeTxGas, uint256 baseGas, uint256 gasPrice, address gasToken, address refundReceiver, bytes signatures) payable returns (bool)",
  "function addOwnerWithThreshold(address owner, uint256 _threshold) external",
];

async function execSafeTx(safe, signer, to, calldata) {
  const nonce = await safe.nonce();
  const txHash = await safe.getTransactionHash(
    to, 0n, calldata, 0,
    0n, 0n, 0n,
    ethers.ZeroAddress, ethers.ZeroAddress, nonce
  );
  const sig = await signer.signMessage(ethers.getBytes(txHash));
  const sigBytes = ethers.getBytes(sig);
  sigBytes[64] = sigBytes[64] === 27 ? 31 : 32;
  const tx = await safe.execTransaction(
    to, 0n, calldata, 0,
    0n, 0n, 0n,
    ethers.ZeroAddress, ethers.ZeroAddress,
    ethers.hexlify(sigBytes),
    { gasLimit: 300_000n }
  );
  const receipt = await tx.wait();
  if (!receipt.status) throw new Error("execTransaction reverted");
  return receipt;
}

async function main() {
  const [deployer] = await ethers.getSigners();
  const safe = new ethers.Contract(SAFE, SAFE_ABI, deployer);
  const safeIface = new ethers.Interface(SAFE_ABI);

  console.log("Safe:     ", SAFE);
  console.log("Deployer: ", deployer.address);
  console.log("Adding:   ", SIGNER_B);
  console.log("          ", SIGNER_C);

  const ownersBefore = await safe.getOwners();
  console.log("\nCurrent owners:", ownersBefore.length, "| threshold:", (await safe.getThreshold()).toString());

  // Step 1: add Signer B, keep threshold 1
  console.log("\n[1] Adding signer B, threshold stays 1...");
  const data1 = safeIface.encodeFunctionData("addOwnerWithThreshold", [SIGNER_B, 1n]);
  const r1 = await execSafeTx(safe, deployer, SAFE, data1);
  console.log("✅ Signer B added. Tx:", r1.hash);

  // Step 2: add Signer C, upgrade threshold to 2-of-3
  console.log("\n[2] Adding signer C, upgrading threshold to 2-of-3...");
  const data2 = safeIface.encodeFunctionData("addOwnerWithThreshold", [SIGNER_C, 2n]);
  const r2 = await execSafeTx(safe, deployer, SAFE, data2);
  console.log("✅ Signer C added. Tx:", r2.hash);

  const ownersAfter = await safe.getOwners();
  const thresholdAfter = await safe.getThreshold();

  console.log("\n═══════════════════════════════════════════════════");
  console.log("  Safe upgraded to", thresholdAfter.toString() + "-of-" + ownersAfter.length);
  console.log("═══════════════════════════════════════════════════");
  ownersAfter.forEach((o, i) => console.log("  owner[" + i + "]:", o));
  console.log("\nhttps://app.safe.global/home?safe=oeth:" + SAFE);

  // Update deployment JSON
  const { writeFileSync } = await import("fs");
  writeFileSync(
    new URL("../deployments/safe-op-mainnet.json", import.meta.url).pathname,
    JSON.stringify({
      network: "op-mainnet",
      timestamp: new Date().toISOString(),
      safe: SAFE,
      owners: ownersAfter,
      threshold: Number(thresholdAfter),
      note: "2-of-3 multisig. signerA=deployer, signerB=0x50B0, signerC=0x950f",
      wTXM: "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e",
      controller: "0x4b31C557AD64609B975610812273BF82F1475384",
      safeUrl: "https://app.safe.global/home?safe=oeth:" + SAFE,
    }, null, 2)
  );
  console.log("Saved → deployments/safe-op-mainnet.json");
}

main().catch(e => { console.error(e.message || e); process.exit(1); });
