import "@nomicfoundation/hardhat-toolbox";
import * as dotenv from "dotenv";

dotenv.config();

const deployerKey = process.env.DEPLOYER_PRIVATE_KEY;
const opSepoliaRpc = process.env.OP_SEPOLIA_RPC_URL || "https://sepolia.optimism.io";
const opMainnetRpc = process.env.OP_MAINNET_RPC_URL || "https://mainnet.optimism.io";

export default {
  solidity: {
    version: "0.8.24",
    settings: {
      optimizer: {
        enabled: true,
        runs: 200,
      },
    },
  },
  paths: {
    sources: "./src",
    tests: "./test",
    cache: "./cache",
    artifacts: "./artifacts",
  },
  networks: {
    "op-sepolia": {
      url: opSepoliaRpc,
      accounts: deployerKey ? [deployerKey] : [],
      chainId: 11155420,
    },
    "op-mainnet": {
      url: opMainnetRpc,
      accounts: deployerKey ? [deployerKey] : [],
      chainId: 10,
    },
  },
};
