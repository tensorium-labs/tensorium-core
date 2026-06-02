import hardhat from "hardhat";
const { ethers } = hardhat;

async function main() {
  const FACTORY = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
  const WETH    = "0x4200000000000000000000000000000000000006";
  const USDC    = "0x7F5c764cBc14f9669B88837ca1490cCa17c31607";
  const WTXM    = "0x2e71FD45530FAe75B6b427F3e71A0CDEB146C20e";

  const ABI = [
    "function getPool(address,address,uint24) view returns (address)",
    "function feeAmountTickSpacing(uint24) view returns (int24)",
  ];
  const f = new ethers.Contract(FACTORY, ABI, ethers.provider);

  const [p500, p3000, p10000] = await Promise.all([
    f.getPool(USDC, WETH, 500),
    f.getPool(USDC, WETH, 3000),
    f.getPool(USDC, WETH, 10000),
  ]);
  console.log("USDC/WETH 0.05%:", p500);
  console.log("USDC/WETH 0.30%:", p3000);
  console.log("USDC/WETH 1.00%:", p10000);

  const [t500, t3000, t10000] = await Promise.all([
    f.feeAmountTickSpacing(500),
    f.feeAmountTickSpacing(3000),
    f.feeAmountTickSpacing(10000),
  ]);
  console.log("tickSpacing(500):", t500.toString());
  console.log("tickSpacing(3000):", t3000.toString());
  console.log("tickSpacing(10000):", t10000.toString());

  const [wt500, wt3000, wt10000] = await Promise.all([
    f.getPool(WTXM, WETH, 500),
    f.getPool(WTXM, WETH, 3000),
    f.getPool(WTXM, WETH, 10000),
  ]);
  console.log("wTXM/WETH 0.05%:", wt500);
  console.log("wTXM/WETH 0.30%:", wt3000);
  console.log("wTXM/WETH 1.00%:", wt10000);
}
main().catch(e => { console.error(e.message); process.exit(1); });
