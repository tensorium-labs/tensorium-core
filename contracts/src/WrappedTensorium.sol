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
