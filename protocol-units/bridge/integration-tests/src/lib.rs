#![allow(dead_code)]
use alloy::node_bindings::AnvilInstance;
use alloy::primitives::Address;
use alloy::primitives::{keccak256, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy_network::EthereumWallet;
use aptos_sdk::rest_client::{Client, FaucetClient};
use aptos_sdk::types::account_address::AccountAddress;
use aptos_sdk::types::LocalAccount;
use bridge_config::Config;
use bridge_service::chains::ethereum::types::AlloyProvider;
use bridge_service::chains::ethereum::types::EthAddress;
use bridge_service::chains::ethereum::{client::EthClient, types::EthHash};
use bridge_service::chains::movement::utils::MovementAddress;
use bridge_service::chains::movement::{client::MovementClient, utils::MovementHash};
use bridge_service::types::{Amount, HashLock};
use bridge_service::{
	chains::bridge_contracts::{BridgeContract, BridgeContractResult},
	types::BridgeAddress,
};
use rand::SeedableRng;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use url::Url;

pub mod utils;

#[derive(Clone)]
pub struct EthToMovementCallArgs {
	pub initiator: Vec<u8>,
	pub recipient: MovementAddress,
	pub bridge_transfer_id: MovementHash,
	pub hash_lock: MovementHash,
	pub time_lock: u64,
	pub amount: u64,
}

#[derive(Clone)]
pub struct MovementToEthCallArgs {
	pub initiator: MovementAddress,
	pub recipient: Vec<u8>,
	pub bridge_transfer_id: EthHash,
	pub hash_lock: EthHash,
	pub time_lock: u64,
	pub amount: u64,
}

impl Default for EthToMovementCallArgs {
	fn default() -> Self {
		Self {
			initiator: b"0x123".to_vec(),
			recipient: MovementAddress(AccountAddress::new(*b"0x00000000000000000000000000face")),
			bridge_transfer_id: MovementHash(*b"00000000000000000000000transfer1"),
			hash_lock: MovementHash(*keccak256(b"secret")),
			time_lock: 3600,
			amount: 100,
		}
	}
}

impl Default for MovementToEthCallArgs {
	fn default() -> Self {
		let preimage = "secret".to_string();
		let serialized_preimage = bcs::to_bytes(&preimage).unwrap();
		let hash_lock = *keccak256(&serialized_preimage);

		Self {
			initiator: MovementAddress(AccountAddress::new(*b"0x00000000000000000000000000face")),
			recipient: b"0x123".to_vec(),
			bridge_transfer_id: EthHash(*b"00000000000000000000000transfer1"),
			hash_lock: EthHash(hash_lock),
			time_lock: 3600,
			amount: 100,
		}
	}
}

pub struct HarnessEthClient {
	pub eth_rpc_url: String,
	pub signer_private_key: PrivateKeySigner,
	pub eth_client: EthClient,
	pub anvil: AnvilInstance,
}

impl HarnessEthClient {
	pub async fn rpc_provider(&self) -> AlloyProvider {
		let rpc_provider = ProviderBuilder::new()
			.with_recommended_fillers()
			.wallet(EthereumWallet::from(self.signer_private_key.clone()))
			.on_builtin(&self.eth_rpc_url)
			.await
			.expect("Error during provider creation");
		rpc_provider
	}

	pub fn signer_address(&self) -> Address {
		self.signer_private_key.address()
	}

	pub async fn initiate_bridge_transfer(
		&mut self,
		initiator_address: BridgeAddress<EthAddress>,
		recipient_address: BridgeAddress<Vec<u8>>,
		hash_lock: HashLock,
		amount: Amount, // the amount
	) -> BridgeContractResult<()> {
		self.eth_client
			.initiate_bridge_transfer(initiator_address, recipient_address, hash_lock, amount)
			.await
	}

	pub async fn deposit_weth_and_approve(
		&mut self,
		initiator_address: BridgeAddress<EthAddress>,
		amount: Amount,
	) -> BridgeContractResult<()> {
		self.eth_client
			.deposit_weth_and_approve(initiator_address.0 .0, U256::from(amount.value()))
			.await
			.expect("Failed to deposit WETH");
		Ok(())
	}
}

pub struct HarnessMvtClient {
	pub movement_client: MovementClient,
	pub movement_process: tokio::process::Child,
	///The Apotos Rest Client
	pub rest_client: Client,
	///The Apotos Rest Client
	pub faucet_client: Arc<RwLock<FaucetClient>>,
}

impl HarnessMvtClient {
	pub fn gen_aptos_account() -> Vec<u8> {
		let mut rng = ::rand::rngs::StdRng::from_seed([3u8; 32]);
		let movement_recipient = LocalAccount::generate(&mut rng);
		movement_recipient.public_key().to_bytes().to_vec()
	}
}

pub struct TestHarness {
	// pub eth_client: Option<HarnessEthClient>,
	// pub movement_client: Option<MovementClient>,
}

impl TestHarness {
	pub async fn new_with_movement() -> (HarnessMvtClient, Config) {
		let (config, movement_process) =
			bridge_setup::test_mvt_setup().await.expect("Failed to setup MovementClient");

		let movement_client = MovementClient::new(&config.movement)
			.await
			.expect("Failed to create MovementClient");

		let node_connection_url = Url::from_str(&config.movement.mvt_rpc_connection_url())
			.expect("Bad movement rpc url in config");
		let rest_client = Client::new(node_connection_url.clone());

		let faucet_url = Url::from_str(&config.movement.mvt_faucet_connection_url())
			.expect("Bad movement faucet url in config");
		let faucet_client = Arc::new(RwLock::new(FaucetClient::new(
			faucet_url.clone(),
			node_connection_url.clone(),
		)));

		(HarnessMvtClient { movement_client, movement_process, rest_client, faucet_client }, config)
	}

	// pub fn movement_rest_client(&self) -> &Client {
	// 	self.movement_client().expect("Could not fetch Movement client").rest_client()
	// }

	// pub fn movement_faucet_client(&self) -> &Arc<RwLock<FaucetClient>> {
	// 	self.movement_client()
	// 		.expect("Could not fetch Movement client")
	// 		.faucet_client()
	// 		.expect("Faucet client not initialized")
	// }

	// pub fn movement_client(&self) -> Result<&MovementClient> {
	// 	self.movement_client
	// 		.as_ref()
	// 		.ok_or(anyhow::Error::msg("MovementClient not initialized"))
	// }

	// pub fn movement_client_mut(&mut self) -> Result<&mut MovementClient> {
	// 	self.movement_client
	// 		.as_mut()
	// 		.ok_or(anyhow::Error::msg("MovementClient not initialized"))
	// }

	pub async fn new_only_eth() -> (HarnessEthClient, Config) {
		let (config, anvil) =
			bridge_setup::test_eth_setup().await.expect("Test eth config setup failed.");
		let eth_rpc_url = config.eth.eth_rpc_connection_url().clone();
		let signer_private_key = config
			.eth
			.signer_private_key
			.parse::<PrivateKeySigner>()
			.expect("Error during parsing signer private key?");

		let eth_client = EthClient::new(&config.eth).await.expect("Failed to create EthClient");
		(HarnessEthClient { eth_client, eth_rpc_url, signer_private_key, anvil }, config)
	}

	// pub fn eth_client(&self) -> Result<&EthClient> {
	// 	self.eth_client
	// 		.as_ref()
	// 		.map(|eth| &eth.eth_client)
	// 		.ok_or(anyhow::Error::msg("EthClient not initialized"))
	// }

	// pub fn eth_client_mut(&mut self) -> Result<&mut EthClient> {
	// 	self.eth_client
	// 		.as_mut()
	// 		.map(|eth| &mut eth.eth_client)
	// 		.ok_or(anyhow::Error::msg("EthClient not initialized"))
	// }

	// pub fn set_eth_signer(&mut self, signer: SecretKey<Secp256k1>) -> Address {
	// 	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	// 	let wallet: &mut EthereumWallet = eth_client.rpc_provider_mut().wallet_mut();
	// 	let clone_signer = signer.clone();
	// 	wallet.register_default_signer(LocalSigner::from(signer));
	// 	eth_client.set_signer_address(clone_signer);
	// 	eth_client.get_signer_address()
	// }

	// pub fn eth_signer_address(&self) -> Address {
	// 	let eth_client = self.eth_client().expect("EthClient not initialized");
	// 	eth_client.get_signer_address()
	// }

	// The port that Anvil will listen on.
	// pub fn rpc_port(&self) -> u16 {
	// 	self.eth_client().expect("Could not fetch eth client").rpc_port()
	// }

	// pub async fn deploy_initiator_contract(&mut self) -> Address {
	// 	let eth_client = self.eth_client.as_mut().expect("EthClient not initialized");
	// 	let rpc_provider = ProviderBuilder::new()
	// 		.with_recommended_fillers()
	// 		.wallet(EthereumWallet::from(eth_client.signer_private_key.clone()))
	// 		.on_builtin(&eth_client.eth_rpc_url)
	// 		.await
	// 		.expect("Error during provider creation");

	// 	let contract = AtomicBridgeInitiator::deploy(rpc_provider.clone())
	// 		.await
	// 		.expect("Failed to deploy AtomicBridgeInitiator");
	// 	eth_client.set_initiator_contract(contract.with_cloned_provider());
	// 	eth_client.initiator_contract_address().expect("Initiator contract not set")
	// }

	// pub async fn deploy_weth_contract(&mut self) -> Address {
	// 	let eth_client = self.eth_client_mut().expect("EthClient not initialized");
	// 	let weth = WETH9::deploy(rpc_provider()).await.expect("Failed to deploy WETH9");
	// 	eth_client.set_weth_contract(weth.with_cloned_provider());
	// 	eth_client.weth_contract_address().expect("WETH contract not set")
	// }

	// pub async fn deploy_init_contracts(&mut self) {
	// 	let _ = self.deploy_initiator_contract().await;
	// 	let weth_address = self.deploy_weth_contract().await;
	// 	self.eth_client()
	// 		.expect("Failed to get EthClient")
	// 		.initialize_initiator_contract(
	// 			EthAddress(weth_address),
	// 			EthAddress(self.eth_signer_address()),
	// 			TimeLock(1),
	// 		)
	// 		.await
	// 		.expect("Failed to initialize contract");
	// }
}
