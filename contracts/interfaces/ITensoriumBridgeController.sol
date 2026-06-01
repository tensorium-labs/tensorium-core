// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface ITensoriumBridgeController {
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

    function owner() external view returns (address);
    function token() external view returns (address);
    function paused() external view returns (bool);
    function operators(address account) external view returns (bool);
    function processedEventIds(bytes32 bridgeEventId) external view returns (bool);

    function setOperator(address account, bool allowed) external;
    function pause() external;
    function unpause() external;

    function mintFromTensoriumDeposit(
        bytes32 bridgeEventId,
        bytes32 tensoriumTxid,
        address recipient,
        uint256 amount
    ) external;

    function requestWithdrawalToTensorium(
        bytes32 bridgeEventId,
        string calldata tensoriumAddress,
        uint256 amount
    ) external;
}
