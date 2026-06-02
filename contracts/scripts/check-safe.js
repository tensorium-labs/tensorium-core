import hardhat from "hardhat";
const { ethers } = hardhat;

const SAFE = "0x9B3B2DB2eCf2b83f58ed256C252204f0d76dB6e9";
const WTXM = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";
const CTRL = "0x4b31C557AD64609B975610812273BF82F1475384";

async function main() {
  const safeAbi = [
    "function getOwners() view returns (address[])",
    "function getThreshold() view returns (uint256)",
    "function nonce() view returns (uint256)",
  ];
  const ownableAbi = ["function owner() view returns (address)"];

  const [owners, threshold, nonce, wtxmOwner, ctrlOwner] = await Promise.all([
    ethers.provider.call({to:SAFE, data: new ethers.Interface(safeAbi).encodeFunctionData("getOwners")}).then(r => new ethers.Interface(safeAbi).decodeFunctionResult("getOwners",r)[0]),
    ethers.provider.call({to:SAFE, data: new ethers.Interface(safeAbi).encodeFunctionData("getThreshold")}).then(r => new ethers.Interface(safeAbi).decodeFunctionResult("getThreshold",r)[0]),
    ethers.provider.call({to:SAFE, data: new ethers.Interface(safeAbi).encodeFunctionData("nonce")}).then(r => new ethers.Interface(safeAbi).decodeFunctionResult("nonce",r)[0]),
    ethers.provider.call({to:WTXM, data: new ethers.Interface(ownableAbi).encodeFunctionData("owner")}).then(r => new ethers.Interface(ownableAbi).decodeFunctionResult("owner",r)[0]),
    ethers.provider.call({to:CTRL, data: new ethers.Interface(ownableAbi).encodeFunctionData("owner")}).then(r => new ethers.Interface(ownableAbi).decodeFunctionResult("owner",r)[0]),
  ]);

  console.log("═══════════════════════════════════════════");
  console.log("  Tensorium Gnosis Safe — OP Mainnet");
  console.log("═══════════════════════════════════════════");
  console.log("Safe:      ", SAFE);
  console.log("Threshold: ", threshold.toString(), "of", owners.length);
  console.log("Nonce:     ", nonce.toString());
  console.log("Owners:");
  owners.forEach((o, i) => console.log(`  [${i}]`, o));
  console.log("\nwTXM owner:        ", wtxmOwner, wtxmOwner.toLowerCase() === SAFE.toLowerCase() ? "✅" : "❌");
  console.log("Controller owner:  ", ctrlOwner, ctrlOwner.toLowerCase() === SAFE.toLowerCase() ? "✅" : "❌");
  console.log("\nhttps://app.safe.global/home?safe=oeth:" + SAFE);
}

main().catch(e => { console.error(e.message); process.exit(1); });
