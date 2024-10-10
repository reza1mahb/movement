// SPDX-License-Identifier: MIT
pragma solidity ^0.8.22;

import "forge-std/Script.sol";
import {AtomicBridgeInitiatorMOVE} from "../src/AtomicBridgeInitiatorMOVE.sol";
import {TransparentUpgradeableProxy} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract AtomicBridgeInitiatorMOVEDeployer is Script {
    TransparentUpgradeableProxy public atomicBridgeProxy;
    TimelockController public timelock;
    string public atomicBridgeSignature = "initialize(address,address,uint256,uint256)";
    address public moveAdmin;

    // Parameters
    address public moveTokenAddress = address(0xC36ba8B8fD9EcbF36288b9B9B0ae9FC3E0645227); 
    address public ownerAddress = address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266); 
    uint256 public timeLockDuration = 172800; // 48 hours in seconds
    uint256 public initialPoolBalance = 0 ether; // Initial pool balance
    uint256 public minDelay = 2 days; // 2-day delay for governance timelock

    // Safe addresses (replace these with actual safe addresses)
    address public movementLabsSafe = address(0x493516F6dB02c9b7f649E650c5de244646022Aa0); 
    address public movementFoundationSafe = address(0x00db70A9e12537495C359581b7b3Bc3a69379A00);

    bytes32 public constant DEFAULT_ADMIN_ROLE = 0x00;

    function run() external {
        uint256 signer = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(signer);

        address[] memory proposers = new address[](1);
        address[] memory executors = new address[](1);

        proposers[0] = movementLabsSafe;
        executors[0] = movementFoundationSafe;

        // Deploy TimelockController
        timelock = new TimelockController(minDelay, proposers, executors, address(0));
        console.log("Timelock deployed at:", address(timelock));

        // Deploy AtomicBridgeInitiatorMOVE contract
        _deployAtomicBridge();

        // Post-deployment checks (example checks)
        require(IERC20(moveTokenAddress).balanceOf(movementFoundationSafe) == 0, "Initial balance should be zero");

        vm.stopBroadcast();
    }

    function _deployAtomicBridge() internal {
        console.log("AtomicBridgeInitiatorMOVE: deploying");

        // Instantiate the implementation contract
        AtomicBridgeInitiatorMOVE atomicBridgeImplementation = new AtomicBridgeInitiatorMOVE();

        // Deploy the TransparentUpgradeableProxy
        atomicBridgeProxy = new TransparentUpgradeableProxy(
            address(atomicBridgeImplementation),
            address(timelock), // Admin is the timelock
            abi.encodeWithSignature(
                atomicBridgeSignature,
                moveTokenAddress,  // MOVE token address
                ownerAddress,      // Owner of the contract
                timeLockDuration,  // Timelock duration (48 hours)
                initialPoolBalance // Initial pool balance
            )
        );

        console.log("AtomicBridgeInitiatorMOVE deployed at proxy address:", address(atomicBridgeProxy));
        console.log("Implementation address:", address(atomicBridgeImplementation));
    }

    function _upgradeAtomicBridge() internal {
        console.log("AtomicBridgeInitiatorMOVE: upgrading");
        AtomicBridgeInitiatorMOVE newBridgeImplementation = new AtomicBridgeInitiatorMOVE();

        timelock.schedule(
            address(moveAdmin),
            0,
            abi.encodeWithSignature(
                "upgradeAndCall(address,address,bytes)",
                address(atomicBridgeProxy),
                address(newBridgeImplementation),
                abi.encodeWithSignature(
                    atomicBridgeSignature,
                    moveTokenAddress, 
                    ownerAddress, 
                    timeLockDuration, 
                    initialPoolBalance
                )
            ),
            bytes32(0),
            bytes32(0),
            block.timestamp + minDelay
        );
    }
}
