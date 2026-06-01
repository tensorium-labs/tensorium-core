import { expect } from "chai";
import hardhat from "hardhat";

const { ethers } = hardhat;

describe("Phase 9A bridge contracts", function () {
  async function deployFixture() {
    const [owner, operator, pauser, user, stranger, pendingOwner] =
      await ethers.getSigners();
    const maxPerTx = ethers.parseEther("10000");

    const WrappedTensorium = await ethers.getContractFactory("WrappedTensorium");
    const token = await WrappedTensorium.deploy(
      "Wrapped Tensorium",
      "wTXM",
      owner.address
    );
    await token.waitForDeployment();

    const TensoriumBridgeController = await ethers.getContractFactory(
      "TensoriumBridgeController"
    );
    const controller = await TensoriumBridgeController.deploy(
      await token.getAddress(),
      owner.address,
      maxPerTx
    );
    await controller.waitForDeployment();

    await token.setBridgeController(await controller.getAddress());
    await token.setPauser(pauser.address);
    await controller.setOperator(operator.address, true);
    await controller.setPauser(pauser.address);

    return {
      owner,
      operator,
      pauser,
      user,
      stranger,
      pendingOwner,
      token,
      controller,
      maxPerTx,
    };
  }

  // ── WrappedTensorium ──────────────────────────────────────────────────────

  describe("WrappedTensorium", function () {
    it("owner can set bridge controller", async function () {
      const { owner, token, stranger } = await deployFixture();
      await expect(token.connect(owner).setBridgeController(stranger.address))
        .to.emit(token, "BridgeControllerUpdated");
      expect(await token.bridgeController()).to.equal(stranger.address);
    });

    it("non-owner cannot set bridge controller", async function () {
      const { token, stranger } = await deployFixture();
      await expect(
        token.connect(stranger).setBridgeController(stranger.address)
      ).to.be.revertedWithCustomError(token, "OwnableUnauthorizedAccount");
    });

    it("owner can set pauser", async function () {
      const { owner, token, stranger } = await deployFixture();
      await expect(token.connect(owner).setPauser(stranger.address)).to.emit(
        token,
        "PauserUpdated"
      );
      expect(await token.pauser()).to.equal(stranger.address);
    });

    it("pauser can pause", async function () {
      const { pauser, token } = await deployFixture();
      await expect(token.connect(pauser).pause()).to.emit(token, "Paused");
      expect(await token.paused()).to.be.true;
    });

    it("pauser cannot unpause", async function () {
      const { owner, pauser, token } = await deployFixture();
      await token.connect(owner).pause();
      await expect(
        token.connect(pauser).unpause()
      ).to.be.revertedWithCustomError(token, "OwnableUnauthorizedAccount");
    });

    it("non-pauser cannot pause", async function () {
      const { token, stranger } = await deployFixture();
      await expect(
        token.connect(stranger).pause()
      ).to.be.revertedWithCustomError(token, "NotPauser");
    });

    it("owner can unpause", async function () {
      const { owner, token } = await deployFixture();
      await token.connect(owner).pause();
      await expect(token.connect(owner).unpause()).to.emit(token, "Unpaused");
      expect(await token.paused()).to.be.false;
    });

    it("bridgeMint blocked when token is paused", async function () {
      const { owner, operator, user, token, controller } =
        await deployFixture();
      await token.connect(owner).pause();
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("deposit-paused"),
          ethers.id("tx-paused"),
          user.address,
          ethers.parseEther("1")
        )
      ).to.be.revertedWithCustomError(token, "EnforcedPause");
    });

    it("Ownable2Step: ownership transfer requires acceptance", async function () {
      const { owner, token, pendingOwner } = await deployFixture();
      await token.connect(owner).transferOwnership(pendingOwner.address);
      expect(await token.owner()).to.equal(owner.address);
      expect(await token.pendingOwner()).to.equal(pendingOwner.address);
      await token.connect(pendingOwner).acceptOwnership();
      expect(await token.owner()).to.equal(pendingOwner.address);
    });
  });

  // ── TensoriumBridgeController ─────────────────────────────────────────────

  describe("TensoriumBridgeController", function () {
    it("operator can mint, emits DepositMinted", async function () {
      const { operator, user, token, controller } = await deployFixture();
      const amount = ethers.parseEther("100");
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("deposit-1"),
          ethers.id("tx-1"),
          user.address,
          amount
        )
      ).to.emit(controller, "DepositMinted");
      expect(await token.balanceOf(user.address)).to.equal(amount);
    });

    it("duplicate bridgeEventId reverts BridgeEventAlreadyProcessed", async function () {
      const { operator, user, controller } = await deployFixture();
      const bridgeEventId = ethers.id("deposit-dup");
      const amount = ethers.parseEther("1");
      await controller.connect(operator).mintFromTensoriumDeposit(
        bridgeEventId,
        ethers.id("tx-dup-1"),
        user.address,
        amount
      );
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          bridgeEventId,
          ethers.id("tx-dup-2"),
          user.address,
          amount
        )
      ).to.be.revertedWithCustomError(
        controller,
        "BridgeEventAlreadyProcessed"
      );
    });

    it("mint amount exceeding maxPerTx reverts ExceedsMaxPerTx", async function () {
      const { operator, user, controller, maxPerTx } = await deployFixture();
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("deposit-over"),
          ethers.id("tx-over"),
          user.address,
          maxPerTx + 1n
        )
      ).to.be.revertedWithCustomError(controller, "ExceedsMaxPerTx");
    });

    it("non-operator cannot mint", async function () {
      const { user, controller } = await deployFixture();
      await expect(
        controller.connect(user).mintFromTensoriumDeposit(
          ethers.id("deposit-x"),
          ethers.id("tx-x"),
          user.address,
          ethers.parseEther("1")
        )
      ).to.be.revertedWithCustomError(controller, "NotOperator");
    });

    it("user can withdraw, balance burned, emits WithdrawalRequested", async function () {
      const { operator, user, token, controller } = await deployFixture();
      const amount = ethers.parseEther("25");
      await controller.connect(operator).mintFromTensoriumDeposit(
        ethers.id("deposit-2"),
        ethers.id("tx-2"),
        user.address,
        amount
      );
      await expect(
        controller
          .connect(user)
          .requestWithdrawalToTensorium("txm1qqexampledestination", amount)
      ).to.emit(controller, "WithdrawalRequested");
      expect(await token.balanceOf(user.address)).to.equal(0n);
    });

    it("two withdrawals produce unique bridgeEventIds", async function () {
      const { operator, user, controller } = await deployFixture();
      const amount = ethers.parseEther("1");
      await controller.connect(operator).mintFromTensoriumDeposit(
        ethers.id("d-unique"),
        ethers.id("t-unique"),
        user.address,
        ethers.parseEther("10")
      );
      const tx1 = await controller
        .connect(user)
        .requestWithdrawalToTensorium("txm1qqaaa", amount);
      const receipt1 = await tx1.wait();
      const tx2 = await controller
        .connect(user)
        .requestWithdrawalToTensorium("txm1qqbbb", amount);
      const receipt2 = await tx2.wait();
      const iface = controller.interface;
      const parse = (logs) =>
        logs
          .map((l) => {
            try { return iface.parseLog(l); } catch { return null; }
          })
          .find((e) => e?.name === "WithdrawalRequested");
      const ev1 = parse(receipt1.logs);
      const ev2 = parse(receipt2.logs);
      expect(ev1.args.bridgeEventId).to.not.equal(ev2.args.bridgeEventId);
    });

    it("withdrawal amount exceeding maxPerTx reverts ExceedsMaxPerTx", async function () {
      const { controller, user, maxPerTx } = await deployFixture();
      await expect(
        controller
          .connect(user)
          .requestWithdrawalToTensorium("txm1qqover", maxPerTx + 1n)
      ).to.be.revertedWithCustomError(controller, "ExceedsMaxPerTx");
    });

    it("pauser can pause, cannot unpause", async function () {
      const { pauser, controller } = await deployFixture();
      await expect(controller.connect(pauser).pause()).to.emit(
        controller,
        "BridgePaused"
      );
      expect(await controller.paused()).to.be.true;
      await expect(
        controller.connect(pauser).unpause()
      ).to.be.revertedWithCustomError(controller, "OwnableUnauthorizedAccount");
    });

    it("mint and withdraw blocked when controller is paused", async function () {
      const { owner, operator, user, controller } = await deployFixture();
      await controller.connect(owner).pause();
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("d-pause"),
          ethers.id("t-pause"),
          user.address,
          ethers.parseEther("1")
        )
      ).to.be.revertedWithCustomError(controller, "EnforcedPause");
      await expect(
        controller
          .connect(user)
          .requestWithdrawalToTensorium("txm1qqpause", ethers.parseEther("1"))
      ).to.be.revertedWithCustomError(controller, "EnforcedPause");
    });

    it("owner can update maxPerTx, emits MaxPerTxUpdated", async function () {
      const { owner, controller } = await deployFixture();
      const newMax = ethers.parseEther("5000");
      await expect(controller.connect(owner).setMaxPerTx(newMax)).to.emit(
        controller,
        "MaxPerTxUpdated"
      );
      expect(await controller.maxPerTx()).to.equal(newMax);
    });

    it("Ownable2Step: ownership transfer requires acceptance", async function () {
      const { owner, controller, pendingOwner } = await deployFixture();
      await controller.connect(owner).transferOwnership(pendingOwner.address);
      expect(await controller.owner()).to.equal(owner.address);
      expect(await controller.pendingOwner()).to.equal(pendingOwner.address);
      await controller.connect(pendingOwner).acceptOwnership();
      expect(await controller.owner()).to.equal(pendingOwner.address);
    });

    it("mint with zero recipient reverts InvalidRecipient", async function () {
      const { operator, controller } = await deployFixture();
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("d-zero-recipient"),
          ethers.id("t-zero-recipient"),
          ethers.ZeroAddress,
          ethers.parseEther("1")
        )
      ).to.be.revertedWithCustomError(controller, "InvalidRecipient");
    });

    it("mint with zero amount reverts InvalidAmount", async function () {
      const { operator, user, controller } = await deployFixture();
      await expect(
        controller.connect(operator).mintFromTensoriumDeposit(
          ethers.id("d-zero-amt"),
          ethers.id("t-zero-amt"),
          user.address,
          0n
        )
      ).to.be.revertedWithCustomError(controller, "InvalidAmount");
    });

    it("withdrawal with empty tensoriumAddress reverts InvalidTensoriumAddress", async function () {
      const { controller, user } = await deployFixture();
      await expect(
        controller.connect(user).requestWithdrawalToTensorium("", ethers.parseEther("1"))
      ).to.be.revertedWithCustomError(controller, "InvalidTensoriumAddress");
    });

    it("withdrawal with zero amount reverts InvalidAmount", async function () {
      const { controller, user } = await deployFixture();
      await expect(
        controller.connect(user).requestWithdrawalToTensorium("txm1qqtest", 0n)
      ).to.be.revertedWithCustomError(controller, "InvalidAmount");
    });

    it("withdrawal with insufficient wTXM balance reverts", async function () {
      const { controller, user } = await deployFixture();
      await expect(
        controller.connect(user).requestWithdrawalToTensorium(
          "txm1qqtest",
          ethers.parseEther("1")
        )
      ).to.be.reverted;
    });
  });
});

describe("Direct bridge role isolation", function () {
  async function deployFixture() {
    const [owner, stranger] = await ethers.getSigners();
    const WrappedTensorium = await ethers.getContractFactory("WrappedTensorium");
    const token = await WrappedTensorium.deploy("Wrapped Tensorium", "wTXM", owner.address);
    await token.waitForDeployment();
    return { owner, stranger, token };
  }

  it("direct bridgeMint by non-controller reverts NotBridgeController", async function () {
    const { stranger, token } = await deployFixture();
    await expect(
      token.connect(stranger).bridgeMint(stranger.address, ethers.parseEther("1"))
    ).to.be.revertedWithCustomError(token, "NotBridgeController");
  });

  it("direct bridgeBurnFrom by non-controller reverts NotBridgeController", async function () {
    const { stranger, token } = await deployFixture();
    await expect(
      token.connect(stranger).bridgeBurnFrom(stranger.address, ethers.parseEther("1"))
    ).to.be.revertedWithCustomError(token, "NotBridgeController");
  });

  it("setBridgeController to zero address reverts InvalidBridgeController", async function () {
    const { owner, token } = await deployFixture();
    await expect(
      token.connect(owner).setBridgeController(ethers.ZeroAddress)
    ).to.be.revertedWithCustomError(token, "InvalidBridgeController");
  });
});
