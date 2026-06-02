import hardhat from "hardhat";
const { ethers } = hardhat;

const SAFE_PROXY_FACTORY = "0xa6B71E26C5e0845f74c812102Ca7114b6a896AB2";
const SAFE_SINGLETON     = "0x3E5c63644E683549055b9Be8653de26E0B4CD36E";
const FALLBACK_HANDLER   = "0xf48f2B2d2a534e402487b3ee7C18c33Aec0Fe5e4";

const FACTORY_ABI = [
  "function createProxyWithNonce(address _singleton, bytes initializer, uint256 saltNonce) returns (address proxy)",
];
const SAFE_SETUP_ABI = [
  "function setup(address[] _owners, uint256 _threshold, address to, bytes data, address fallbackHandler, address paymentToken, uint256 payment, address paymentReceiver)",
];

async function main() {
  const [deployer] = await ethers.getSigners();
  console.log("Deployer:", deployer.address);
  console.log("Balance:", ethers.formatEther(await ethers.provider.getBalance(deployer.address)));

  const factory = new ethers.Contract(SAFE_PROXY_FACTORY, FACTORY_ABI, deployer);
  const safeIface = new ethers.Interface(SAFE_SETUP_ABI);

  const initializer = safeIface.encodeFunctionData("setup", [
    [deployer.address], 1n, ethers.ZeroAddress, "0x",
    FALLBACK_HANDLER, ethers.ZeroAddress, 0n, ethers.ZeroAddress,
  ]);

  const saltNonce = BigInt(Date.now());
  console.log("Salt nonce:", saltNonce.toString());

  // staticCall to get return value (predicted address)
  const proxyAddr = await factory.createProxyWithNonce.staticCall(
    SAFE_SINGLETON, initializer, saltNonce
  );
  console.log("Predicted Safe address:", proxyAddr);

  // Actually create
  const tx = await factory.createProxyWithNonce(SAFE_SINGLETON, initializer, saltNonce, {
    gasLimit: 500_000n,
  });
  console.log("Tx hash:", tx.hash);
  const receipt = await tx.wait();
  console.log("Status:", receipt.status);
  console.log("Logs:", receipt.logs.length);
  receipt.logs.forEach((log, i) => {
    console.log(`  Log[${i}]: addr=${log.address} topics[0]=${log.topics[0]?.slice(0,18)}`);
  });

  // The staticCall already gave us the address - verify it
  const code = await ethers.provider.getCode(proxyAddr);
  console.log("Safe deployed:", code.length > 2 ? "YES (" + code.length + " bytes)" : "NO");
  console.log("Safe address:", proxyAddr);
}

main().catch(e => { console.error(e.message); process.exit(1); });
