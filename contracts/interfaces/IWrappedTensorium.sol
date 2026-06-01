// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IWrappedTensorium {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event BridgeControllerUpdated(address indexed previousController, address indexed newController);

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
    function bridgeController() external view returns (address);
    function paused() external view returns (bool);

    function setBridgeController(address newController) external;
    function pause() external;
    function unpause() external;

    function bridgeMint(address to, uint256 amount) external;
    function bridgeBurnFrom(address from, uint256 amount) external;
}
