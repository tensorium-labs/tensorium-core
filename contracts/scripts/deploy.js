import hardhat from "hardhat";
import { writeFileSync, mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const { ethers, network } = hardhat;
const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const multisig = process.env.MULTISIG_ADDRESS;
  const operatorAddr = process.env.OPERATOR_ADDRESS;
  const pauserAddr = process.env.PAUSER_ADDRESS;
  const maxPerTxEther = process.env.MAX_PER_TX || "10000";

  if (!multisig || multisig === ethers.ZeroAddress) {
    throw new Error(
      "MULTISIG_ADDRESS env var must be set to a non-zero address.\n" +
        "For Sepolia testing, set this to a deployer EOA.\n" +
        "For mainnet, set this to a Gnosis Safe address."
    );
  }

  const [deployer] = await ethers.getSigners();
  const maxPerTx = ethers.parseEther(maxPerTxEther);

  console.log("Deployer:          ", deployer.address);
  console.log("Multisig (pending):", multisig);
  console.log("Operator:          ", operatorAddr || "(not set)");
  console.log("Pauser:            ", pauserAddr || "(not set)");
  console.log("maxPerTx:          ", maxPerTxEther, "wTXM");
  console.log("Network:           ", network.name);
  console.log("");

  // 1. Deploy WrappedTensorium
  const WrappedTensorium = await ethers.getContractFactory("WrappedTensorium");
  const token = await WrappedTensorium.deploy(
    "Wrapped Tensorium",
    "wTXM",
    deployer.address
  );
  await token.waitForDeployment();
  const tokenAddress = await token.getAddress();
  console.log("WrappedTensorium deployed:         ", tokenAddress);

  // 2. Deploy TensoriumBridgeController
  const TensoriumBridgeController = await ethers.getContractFactory(
    "TensoriumBridgeController"
  );
  const controller = await TensoriumBridgeController.deploy(
    tokenAddress,
    deployer.address,
    maxPerTx
  );
  await controller.waitForDeployment();
  const controllerAddress = await controller.getAddress();
  console.log("TensoriumBridgeController deployed:", controllerAddress);

  // 3. Wire up
  await token.setBridgeController(controllerAddress);
  console.log("setBridgeController done");

  if (pauserAddr && pauserAddr !== ethers.ZeroAddress) {
    await token.setPauser(pauserAddr);
    await controller.setPauser(pauserAddr);
    console.log("setPauser done:", pauserAddr);
  }

  if (operatorAddr && operatorAddr !== ethers.ZeroAddress) {
    await controller.setOperator(operatorAddr, true);
    console.log("setOperator done:", operatorAddr);
  }

  // 4. Initiate ownership transfer (Ownable2Step — multisig must acceptOwnership)
  await token.transferOwnership(multisig);
  await controller.transferOwnership(multisig);
  console.log("");
  console.log("transferOwnership initiated to:", multisig);
  console.log(
    "⚠️  NEXT STEP: multisig must call acceptOwnership() on BOTH contracts:"
  );
  console.log("   token:     ", tokenAddress);
  console.log("   controller:", controllerAddress);

  // 5. Save deployment record
  const deployments = {
    network: network.name,
    timestamp: new Date().toISOString(),
    deployer: deployer.address,
    multisig,
    operator: operatorAddr || null,
    pauser: pauserAddr || null,
    maxPerTx: maxPerTxEther,
    WrappedTensorium: tokenAddress,
    TensoriumBridgeController: controllerAddress,
  };

  const dir = join(__dirname, "..", "deployments");
  mkdirSync(dir, { recursive: true });
  const outPath = join(dir, `${network.name}.json`);
  writeFileSync(outPath, JSON.stringify(deployments, null, 2));
  console.log("");
  console.log("Deployment saved to:", outPath);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
