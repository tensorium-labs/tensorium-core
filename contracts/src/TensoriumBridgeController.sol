// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/Pausable.sol";

interface IWrappedTensoriumToken {
    function bridgeMint(address to, uint256 amount) external;
    function bridgeBurnFrom(address from, uint256 amount) external;
}

contract TensoriumBridgeController is Ownable, Pausable {
    address public immutable token;
    uint256 public withdrawalNonce;

    mapping(address => bool) public operators;
    mapping(bytes32 => bool) public processedEventIds;

    event OperatorUpdated(address indexed operator, bool allowed);
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
    error InvalidToken();
    error InvalidRecipient();
    error InvalidAmount();
    error InvalidTensoriumAddress();
    error BridgeEventAlreadyProcessed();

    constructor(address token_, address initialOwner) Ownable(initialOwner) {
        if (token_ == address(0)) revert InvalidToken();
        token = token_;
    }

    modifier onlyOperator() {
        if (!operators[msg.sender]) revert NotOperator();
        _;
    }

    function setOperator(address account, bool allowed) external onlyOwner {
        operators[account] = allowed;
        emit OperatorUpdated(account, allowed);
    }

    function pause() external onlyOwner {
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
        if (processedEventIds[bridgeEventId]) revert BridgeEventAlreadyProcessed();

        processedEventIds[bridgeEventId] = true;
        IWrappedTensoriumToken(token).bridgeMint(recipient, amount);

        emit DepositMinted(bridgeEventId, tensoriumTxid, recipient, amount);
    }

    function requestWithdrawalToTensorium(
        bytes32 bridgeEventId,
        string calldata tensoriumAddress,
        uint256 amount
    ) external whenNotPaused {
        if (bytes(tensoriumAddress).length == 0) revert InvalidTensoriumAddress();
        if (amount == 0) revert InvalidAmount();
        if (processedEventIds[bridgeEventId]) revert BridgeEventAlreadyProcessed();

        processedEventIds[bridgeEventId] = true;
        withdrawalNonce += 1;

        IWrappedTensoriumToken(token).bridgeBurnFrom(msg.sender, amount);

        emit WithdrawalRequested(bridgeEventId, msg.sender, tensoriumAddress, amount);
    }
}
