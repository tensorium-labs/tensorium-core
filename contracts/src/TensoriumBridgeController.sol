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
