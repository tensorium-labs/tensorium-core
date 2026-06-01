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
