# Phase 9A Bridge Contracts Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite WrappedTensorium dan TensoriumBridgeController dari scaffold ke production-ready dengan Ownable2Step, pauser role, on-chain cap, dan auto-generated bridgeEventId untuk withdrawal.

**Architecture:** Dua contract terpisah — token (WrappedTensorium) dan controller (TensoriumBridgeController) — dengan interfaces yang updated, test suite lengkap (~18 tests), dan deployment script yang enforce multisig ownership transfer. Interfaces di-update setelah implementasi selesai supaya match exact ABI.

**Tech Stack:** Solidity ^0.8.24, OpenZeppelin v5 (ERC20, Ownable2Step, Pausable), Hardhat, ethers v6, Chai

---

## File Map

| File | Action |
|---|---|
| `contracts/src/WrappedTensorium.sol` | Rewrite |
| `contracts/src/TensoriumBridgeController.sol` | Rewrite |
| `contracts/test/bridge.js` | Rewrite (replace seluruh isi) |
| `contracts/scripts/deploy.js` | Create baru |
| `contracts/interfaces/IWrappedTensorium.sol` | Update |
| `contracts/interfaces/ITensoriumBridgeController.sol` | Update |
| `PHASE9A_EXECUTION_CHECKLIST.md` | Centang item contracts |

---

## Task 1: Write complete failing test suite

**Files:**
- Modify: `contracts/test/bridge.js`

- [ ] **Step 1: Replace isi test/bridge.js dengan test suite baru**

```javascript
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
          .map((l) => { try { return iface.parseLog(l); } catch { return null; } })
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
  });
});
```

- [ ] **Step 2: Jalankan tests, konfirmasi FAIL**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && npm test
```

Expected: test suite fail karena contracts belum punya `pauser`, `maxPerTx`, `Ownable2Step`, dll. Error bisa berupa `TypeError: token.setPauser is not a function` atau `revertedWithCustomError` yang salah.

---

## Task 2: Rewrite WrappedTensorium.sol

**Files:**
- Modify: `contracts/src/WrappedTensorium.sol`

- [ ] **Step 1: Replace isi WrappedTensorium.sol**

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable2Step.sol";
import "@openzeppelin/contracts/utils/Pausable.sol";

contract WrappedTensorium is ERC20, Ownable2Step, Pausable {
    address public bridgeController;
    address public pauser;

    event BridgeControllerUpdated(
        address indexed previousController,
        address indexed newController
    );
    event PauserUpdated(
        address indexed previousPauser,
        address indexed newPauser
    );

    error NotBridgeController();
    error InvalidBridgeController();
    error NotPauser();

    constructor(
        string memory name_,
        string memory symbol_,
        address initialOwner
    ) ERC20(name_, symbol_) Ownable(initialOwner) {}

    modifier onlyBridgeController() {
        if (msg.sender != bridgeController) revert NotBridgeController();
        _;
    }

    modifier onlyPauser() {
        if (msg.sender != pauser && msg.sender != owner()) revert NotPauser();
        _;
    }

    function setBridgeController(address newController) external onlyOwner {
        if (newController == address(0)) revert InvalidBridgeController();
        address previous = bridgeController;
        bridgeController = newController;
        emit BridgeControllerUpdated(previous, newController);
    }

    function setPauser(address newPauser) external onlyOwner {
        address previous = pauser;
        pauser = newPauser;
        emit PauserUpdated(previous, newPauser);
    }

    function pause() external onlyPauser {
        _pause();
    }

    function unpause() external onlyOwner {
        _unpause();
    }

    function bridgeMint(address to, uint256 amount)
        external
        onlyBridgeController
        whenNotPaused
    {
        _mint(to, amount);
    }

    function bridgeBurnFrom(address from, uint256 amount)
        external
        onlyBridgeController
        whenNotPaused
    {
        _burn(from, amount);
    }
}
```

- [ ] **Step 2: Jalankan tests, konfirmasi token tests pass**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && npm test
```

Expected: semua 9 test di suite "WrappedTensorium" PASS. Suite "TensoriumBridgeController" masih FAIL karena controller belum di-update.

---

## Task 3: Rewrite TensoriumBridgeController.sol

**Files:**
- Modify: `contracts/src/TensoriumBridgeController.sol`

- [ ] **Step 1: Replace isi TensoriumBridgeController.sol**

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/access/Ownable2Step.sol";
import "@openzeppelin/contracts/utils/Pausable.sol";

interface IWrappedTensoriumToken {
    function bridgeMint(address to, uint256 amount) external;
    function bridgeBurnFrom(address from, uint256 amount) external;
}

contract TensoriumBridgeController is Ownable2Step, Pausable {
    address public immutable token;
    uint256 public withdrawalNonce;
    uint256 public maxPerTx;
    address public pauser;

    mapping(address => bool) public operators;
    mapping(bytes32 => bool) public processedEventIds;

    event OperatorUpdated(address indexed operator, bool allowed);
    event PauserUpdated(
        address indexed previousPauser,
        address indexed newPauser
    );
    event MaxPerTxUpdated(uint256 previousMax, uint256 newMax);
    event BridgePaused(address indexed by);
    event BridgeUnpaused(address indexed by);
    event DepositMinted(
        bytes32 indexed bridgeEventId,
        bytes32 indexed tensoriumTxid,
        address indexed recipient,
        uint256 amount
    );
    event WithdrawalRequested(
        bytes32 indexed bridgeEventId,
        address indexed requester,
        string tensoriumAddress,
        uint256 amount
    );

    error NotOperator();
    error NotPauser();
    error InvalidToken();
    error InvalidRecipient();
    error InvalidAmount();
    error InvalidTensoriumAddress();
    error BridgeEventAlreadyProcessed();
    error ExceedsMaxPerTx();

    constructor(
        address token_,
        address initialOwner,
        uint256 maxPerTx_
    ) Ownable(initialOwner) {
        if (token_ == address(0)) revert InvalidToken();
        token = token_;
        maxPerTx = maxPerTx_;
    }

    modifier onlyOperator() {
        if (!operators[msg.sender]) revert NotOperator();
        _;
    }

    modifier onlyPauser() {
        if (msg.sender != pauser && msg.sender != owner()) revert NotPauser();
        _;
    }

    function setOperator(address account, bool allowed) external onlyOwner {
        operators[account] = allowed;
        emit OperatorUpdated(account, allowed);
    }

    function setPauser(address newPauser) external onlyOwner {
        address previous = pauser;
        pauser = newPauser;
        emit PauserUpdated(previous, newPauser);
    }

    function setMaxPerTx(uint256 newMax) external onlyOwner {
        uint256 previous = maxPerTx;
        maxPerTx = newMax;
        emit MaxPerTxUpdated(previous, newMax);
    }

    function pause() external onlyPauser {
        _pause();
        emit BridgePaused(msg.sender);
    }

    function unpause() external onlyOwner {
        _unpause();
        emit BridgeUnpaused(msg.sender);
    }

    function mintFromTensoriumDeposit(
        bytes32 bridgeEventId,
        bytes32 tensoriumTxid,
        address recipient,
        uint256 amount
    ) external onlyOperator whenNotPaused {
        if (recipient == address(0)) revert InvalidRecipient();
        if (amount == 0) revert InvalidAmount();
        if (amount > maxPerTx) revert ExceedsMaxPerTx();
        if (processedEventIds[bridgeEventId]) revert BridgeEventAlreadyProcessed();

        processedEventIds[bridgeEventId] = true;
        IWrappedTensoriumToken(token).bridgeMint(recipient, amount);

        emit DepositMinted(bridgeEventId, tensoriumTxid, recipient, amount);
    }

    function requestWithdrawalToTensorium(
        string calldata tensoriumAddress,
        uint256 amount
    ) external whenNotPaused {
        if (bytes(tensoriumAddress).length == 0) revert InvalidTensoriumAddress();
        if (amount == 0) revert InvalidAmount();
        if (amount > maxPerTx) revert ExceedsMaxPerTx();

        withdrawalNonce += 1;
        bytes32 bridgeEventId = keccak256(
            abi.encodePacked(withdrawalNonce, msg.sender, amount, tensoriumAddress)
        );

        processedEventIds[bridgeEventId] = true;
        IWrappedTensoriumToken(token).bridgeBurnFrom(msg.sender, amount);

        emit WithdrawalRequested(bridgeEventId, msg.sender, tensoriumAddress, amount);
    }
}
```

- [ ] **Step 2: Jalankan tests, konfirmasi semua 18 tests PASS**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && npm test
```

Expected output:
```
Phase 9A bridge contracts
  WrappedTensorium
    ✓ owner can set bridge controller
    ✓ non-owner cannot set bridge controller
    ✓ owner can set pauser
    ✓ pauser can pause
    ✓ pauser cannot unpause
    ✓ non-pauser cannot pause
    ✓ owner can unpause
    ✓ bridgeMint blocked when token is paused
    ✓ Ownable2Step: ownership transfer requires acceptance
  TensoriumBridgeController
    ✓ operator can mint, emits DepositMinted
    ✓ duplicate bridgeEventId reverts BridgeEventAlreadyProcessed
    ✓ mint amount exceeding maxPerTx reverts ExceedsMaxPerTx
    ✓ non-operator cannot mint
    ✓ user can withdraw, balance burned, emits WithdrawalRequested
    ✓ two withdrawals produce unique bridgeEventIds
    ✓ withdrawal amount exceeding maxPerTx reverts ExceedsMaxPerTx
    ✓ pauser can pause, cannot unpause
    ✓ mint and withdraw blocked when controller is paused
    ✓ owner can update maxPerTx, emits MaxPerTxUpdated
    ✓ Ownable2Step: ownership transfer requires acceptance

  20 passing
```

---

## Task 4: Write deployment script

**Files:**
- Create: `contracts/scripts/deploy.js`

- [ ] **Step 1: Buat file contracts/scripts/deploy.js**

```javascript
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
  console.log("⚠️  NEXT STEP: multisig must call acceptOwnership() on BOTH contracts:");
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
```

- [ ] **Step 2: Verify script syntax — dry compile (tidak deploy)**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && node --input-type=module --eval "import './scripts/deploy.js'" 2>&1 | head -5 || true
```

Expected: tidak ada syntax error (script akan exit karena MULTISIG_ADDRESS tidak di-set, itu normal).

---

## Task 5: Update interfaces

**Files:**
- Modify: `contracts/interfaces/IWrappedTensorium.sol`
- Modify: `contracts/interfaces/ITensoriumBridgeController.sol`

- [ ] **Step 1: Replace IWrappedTensorium.sol**

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IWrappedTensorium {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event BridgeControllerUpdated(
        address indexed previousController,
        address indexed newController
    );
    event PauserUpdated(
        address indexed previousPauser,
        address indexed newPauser
    );

    error NotBridgeController();
    error InvalidBridgeController();
    error NotPauser();

    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
    function decimals() external view returns (uint8);
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function allowance(address owner, address spender) external view returns (uint256);
    function transfer(address to, uint256 value) external returns (bool);
    function approve(address spender, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);

    function owner() external view returns (address);
    function pendingOwner() external view returns (address);
    function bridgeController() external view returns (address);
    function pauser() external view returns (address);
    function paused() external view returns (bool);

    function transferOwnership(address newOwner) external;
    function acceptOwnership() external;
    function setBridgeController(address newController) external;
    function setPauser(address newPauser) external;
    function pause() external;
    function unpause() external;
    function bridgeMint(address to, uint256 amount) external;
    function bridgeBurnFrom(address from, uint256 amount) external;
}
```

- [ ] **Step 2: Replace ITensoriumBridgeController.sol**

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface ITensoriumBridgeController {
    event OperatorUpdated(address indexed operator, bool allowed);
    event PauserUpdated(
        address indexed previousPauser,
        address indexed newPauser
    );
    event MaxPerTxUpdated(uint256 previousMax, uint256 newMax);
    event BridgePaused(address indexed by);
    event BridgeUnpaused(address indexed by);
    event DepositMinted(
        bytes32 indexed bridgeEventId,
        bytes32 indexed tensoriumTxid,
        address indexed recipient,
        uint256 amount
    );
    event WithdrawalRequested(
        bytes32 indexed bridgeEventId,
        address indexed requester,
        string tensoriumAddress,
        uint256 amount
    );

    error NotOperator();
    error NotPauser();
    error InvalidToken();
    error InvalidRecipient();
    error InvalidAmount();
    error InvalidTensoriumAddress();
    error BridgeEventAlreadyProcessed();
    error ExceedsMaxPerTx();

    function owner() external view returns (address);
    function pendingOwner() external view returns (address);
    function token() external view returns (address);
    function paused() external view returns (bool);
    function withdrawalNonce() external view returns (uint256);
    function maxPerTx() external view returns (uint256);
    function pauser() external view returns (address);
    function operators(address account) external view returns (bool);
    function processedEventIds(bytes32 bridgeEventId) external view returns (bool);

    function transferOwnership(address newOwner) external;
    function acceptOwnership() external;
    function setOperator(address account, bool allowed) external;
    function setPauser(address newPauser) external;
    function setMaxPerTx(uint256 newMax) external;
    function pause() external;
    function unpause() external;

    function mintFromTensoriumDeposit(
        bytes32 bridgeEventId,
        bytes32 tensoriumTxid,
        address recipient,
        uint256 amount
    ) external;

    function requestWithdrawalToTensorium(
        string calldata tensoriumAddress,
        uint256 amount
    ) external;
}
```

- [ ] **Step 3: Compile ulang untuk verify tidak ada error**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && npm run compile
```

Expected: `Compiled N Solidity files successfully`

---

## Task 6: Update checklist dan commit

**Files:**
- Modify: `PHASE9A_EXECUTION_CHECKLIST.md`

- [ ] **Step 1: Centang item "Review contract ownership transfer path" di Phase 9A.1**

Di file `PHASE9A_EXECUTION_CHECKLIST.md`, ubah:
```
- [ ] Review contract ownership transfer path
```
menjadi:
```
- [x] Review contract ownership transfer path
```

Dan update bagian "Still not done" → pindahkan "contracts" ke "Already done":
```
- [x] contracts
```

- [ ] **Step 2: Run tests sekali lagi untuk final confirm**

```bash
cd /root/.openclaw/workspace/tensorium-core/contracts && npm test
```

Expected: semua 20 tests PASS, 0 failing.

- [ ] **Step 3: Commit**

```bash
cd /root/.openclaw/workspace/tensorium-core && git add \
  contracts/src/WrappedTensorium.sol \
  contracts/src/TensoriumBridgeController.sol \
  contracts/test/bridge.js \
  contracts/scripts/deploy.js \
  contracts/interfaces/IWrappedTensorium.sol \
  contracts/interfaces/ITensoriumBridgeController.sol \
  PHASE9A_EXECUTION_CHECKLIST.md \
  docs/superpowers/specs/2026-06-01-phase9a-bridge-contracts-design.md \
  docs/superpowers/plans/2026-06-01-phase9a-bridge-contracts-rewrite.md && \
git commit -m "feat(phase9a): rewrite bridge contracts to production-ready

- Ownable2Step on both contracts (pull-based ownership transfer)
- pauser role: pause-only, unpause remains onlyOwner
- on-chain maxPerTx cap with setMaxPerTx
- auto-generate bridgeEventId in requestWithdrawalToTensorium
- deployment script with MULTISIG_ADDRESS enforcement
- 20 tests covering all new behaviors"
```

- [ ] **Step 4: Push ke GitHub**

```bash
cd /root/.openclaw/workspace/tensorium-core && git push origin main
```

Expected: push berhasil ke tensorium-labs/tensorium-core.
