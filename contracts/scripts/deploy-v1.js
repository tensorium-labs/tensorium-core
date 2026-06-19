import hardhat from "hardhat";
import { writeFileSync, mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const { ethers, network } = hardhat;
const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const maxPerTxEther = process.env.MAX_PER_TX || "1000000";

  const [deployer] = await ethers.getSigners();
  const maxPerTx = ethers.parseEther(maxPerTxEther);
  const balance = await ethers.provider.getBalance(deployer.address);

  console.log("Deployer:  ", deployer.address);
  console.log("Balance:   ", ethers.formatEther(balance), "ETH");
  console.log("maxPerTx:  ", maxPerTxEther, "wTXM");
  console.log("Network:   ", network.name);
  console.log("");

  // 1. Deploy WrappedTensorium (deployer = owner)
  const WrappedTensorium = await ethers.getContractFactory("WrappedTensorium");
  const token = await WrappedTensorium.deploy(
    "Wrapped Tensorium",
    "wTXM",
    deployer.address
  );
  await token.waitForDeployment();
  const tokenAddress = await token.getAddress();
  console.log("WrappedTensorium deployed:         ", tokenAddress);

  // 2. Deploy TensoriumBridgeController (deployer = owner)
  const Controller = await ethers.getContractFactory("TensoriumBridgeController");
  const controller = await Controller.deploy(
    tokenAddress,
    deployer.address,
    maxPerTx
  );
  await controller.waitForDeployment();
  const controllerAddress = await controller.getAddress();
  console.log("TensoriumBridgeController deployed:", controllerAddress);

  // 3. Wire: token.setBridgeController(controller)
  const tx1 = await token.setBridgeController(controllerAddress);
  await tx1.wait();
  console.log("setBridgeController done");

  // 4. Set deployer as operator (for minting on deposit)
  const tx2 = await controller.setOperator(deployer.address, true);
  await tx2.wait();
  console.log("setOperator(deployer) done");

  // 5. Set deployer as pauser on both
  const tx3 = await token.setPauser(deployer.address);
  await tx3.wait();
  const tx4 = await controller.setPauser(deployer.address);
  await tx4.wait();
  console.log("setPauser(deployer) done on both");

  const balAfter = await ethers.provider.getBalance(deployer.address);
  const cost = balance - balAfter;
  console.log("");
  console.log("Total deploy cost:", ethers.formatEther(cost), "ETH");
  console.log("Remaining balance:", ethers.formatEther(balAfter), "ETH");

  // 6. Save deployment record
  const deployment = {
    network: network.name,
    chainId: network.config.chainId,
    timestamp: new Date().toISOString(),
    version: "tensorhash-v1",
    deployer: deployer.address,
    owner: deployer.address,
    operator: deployer.address,
    pauser: deployer.address,
    maxPerTx: maxPerTxEther,
    WrappedTensorium: tokenAddress,
    TensoriumBridgeController: controllerAddress,
  };

  const dir = join(__dirname, "..", "deployments");
  mkdirSync(dir, { recursive: true });
  const outPath = join(dir, `op-mainnet-v1.json`);
  writeFileSync(outPath, JSON.stringify(deployment, null, 2));
  console.log("Deployment saved to:", outPath);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
