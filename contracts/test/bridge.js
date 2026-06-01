import { expect } from "chai";
import hardhat from "hardhat";

const { ethers } = hardhat;

describe("Phase 9A bridge contracts", function () {
  async function deployFixture() {
    const [owner, operator, user] = await ethers.getSigners();

    const WrappedTensorium = await ethers.getContractFactory("WrappedTensorium");
    const token = await WrappedTensorium.deploy("Wrapped Tensorium", "wTXM", owner.address);
    await token.waitForDeployment();

    const TensoriumBridgeController = await ethers.getContractFactory("TensoriumBridgeController");
    const controller = await TensoriumBridgeController.deploy(await token.getAddress(), owner.address);
    await controller.waitForDeployment();

    await token.setBridgeController(await controller.getAddress());
    await controller.setOperator(operator.address, true);

    return { owner, operator, user, token, controller };
  }

  it("allows an operator to mint once per bridge event", async function () {
    const { operator, user, token, controller } = await deployFixture();

    const bridgeEventId = ethers.id("deposit-1");
    const tensoriumTxid = ethers.id("tensorium-tx-1");
    const amount = ethers.parseEther("100");

    await expect(
      controller.connect(operator).mintFromTensoriumDeposit(
        bridgeEventId,
        tensoriumTxid,
        user.address,
        amount
      )
    ).to.emit(controller, "DepositMinted");

    expect(await token.balanceOf(user.address)).to.equal(amount);

    await expect(
      controller.connect(operator).mintFromTensoriumDeposit(
        bridgeEventId,
        tensoriumTxid,
        user.address,
        amount
      )
    ).to.be.revertedWithCustomError(controller, "BridgeEventAlreadyProcessed");
  });

  it("lets a user request withdrawal through the controller and burns balance", async function () {
    const { operator, user, token, controller } = await deployFixture();

    const depositId = ethers.id("deposit-2");
    const depositTxid = ethers.id("tensorium-tx-2");
    const amount = ethers.parseEther("25");

    await controller.connect(operator).mintFromTensoriumDeposit(
      depositId,
      depositTxid,
      user.address,
      amount
    );

    const withdrawalId = ethers.id("withdrawal-1");

    await expect(
      controller.connect(user).requestWithdrawalToTensorium(
        withdrawalId,
        "txm1qqexampledestination",
        amount
      )
    ).to.emit(controller, "WithdrawalRequested");

    expect(await token.balanceOf(user.address)).to.equal(0n);
  });

  it("blocks minting while paused", async function () {
    const { owner, operator, user, controller } = await deployFixture();

    await controller.connect(owner).pause();

    await expect(
      controller.connect(operator).mintFromTensoriumDeposit(
        ethers.id("deposit-3"),
        ethers.id("tensorium-tx-3"),
        user.address,
        ethers.parseEther("1")
      )
    ).to.be.revertedWithCustomError(controller, "EnforcedPause");
  });
});
