// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/Pausable.sol";

contract WrappedTensorium is ERC20, Ownable, Pausable {
    address public bridgeController;

    event BridgeControllerUpdated(address indexed previousController, address indexed newController);

    error NotBridgeController();
    error InvalidBridgeController();

    constructor(
        string memory name_,
        string memory symbol_,
        address initialOwner
    ) ERC20(name_, symbol_) Ownable(initialOwner) {}

    modifier onlyBridgeController() {
        if (msg.sender != bridgeController) revert NotBridgeController();
        _;
    }

    function setBridgeController(address newController) external onlyOwner {
        if (newController == address(0)) revert InvalidBridgeController();
        address previous = bridgeController;
        bridgeController = newController;
        emit BridgeControllerUpdated(previous, newController);
    }

    function pause() external onlyOwner {
        _pause();
    }

    function unpause() external onlyOwner {
        _unpause();
    }

    function bridgeMint(address to, uint256 amount) external onlyBridgeController whenNotPaused {
        _mint(to, amount);
    }

    function bridgeBurnFrom(address from, uint256 amount) external onlyBridgeController whenNotPaused {
        _burn(from, amount);
    }
}
