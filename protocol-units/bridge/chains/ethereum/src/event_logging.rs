use crate::types::{EthAddress, EventName, SCCResult, SCIResult};
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{address, FixedBytes, LogData};
use alloy::providers::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy::rpc::types::{Filter, Log};
use alloy::{
	json_abi::{Event, EventParam, Param},
	pubsub::PubSubFrontend,
	sol_types::SolEvent,
};
use bridge_shared::{
	bridge_monitoring::{BridgeContractInitiatorEvent, BridgeContractInitiatorMonitoring},
	types::{
		Amount, BridgeTransferDetails, BridgeTransferId, HashLock, HashLockPreImage,
		InitiatorAddress, LockDetails, RecipientAddress, TimeLock,
	},
};
use futures::{channel::mpsc::UnboundedReceiver, Stream, StreamExt};
use std::{fmt::Debug, pin::Pin, task::Poll};
use thiserror::Error;

use crate::{
	types::{AlloyParam, AtomicBridgeInitiator, CompletedDetails},
	EthHash,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveCounterpartyEvent<A, H> {
	LockedBridgeTransfer(LockDetails<A, H>),
	CompletedBridgeTransfer(CompletedDetails<A, H>),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MoveCounterpartyError {
	#[error("Transfer not found")]
	TransferNotFound,
	#[error("Invalid hash lock pre image (secret)")]
	InvalidHashLockPreImage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EthInitiatorEvent<A, H> {
	InitiatedBridgeTransfer(BridgeTransferDetails<A, H>),
	CompletedBridgeTransfer(BridgeTransferId<H>, HashLockPreImage),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EthInitiatorError {
	#[error("Failed to initiate bridge transfer")]
	InitiateTransferError,
	#[error("Transfer not found")]
	TransferNotFound,
	#[error("Invalid hash lock pre image (secret)")]
	InvalidHashLockPreImage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbstractBlockainEvent<A, H> {
	InitiatorContractEvent(SCIResult<A, H>),
	CounterpartyContractEvent(SCCResult<A, H>),
	Noop,
}

pub struct EthInitiatorMonitoring<A, H> {
	listener: UnboundedReceiver<AbstractBlockainEvent<A, H>>,
	ws: RootProvider<PubSubFrontend>,
}

impl BridgeContractInitiatorMonitoring for EthInitiatorMonitoring<EthAddress, EthHash> {
	type Address = EthAddress;
	type Hash = EthHash;
}

impl EthInitiatorMonitoring<EthAddress, EthHash> {
	async fn run(
		rpc_url: &str,
		listener: UnboundedReceiver<AbstractBlockainEvent<EthAddress, EthHash>>,
	) -> Result<Self, anyhow::Error> {
		let ws = WsConnect::new(rpc_url);
		let ws = ProviderBuilder::new().on_ws(ws).await?;

		let initiator_address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
		let filter = Filter::new()
			.address(initiator_address)
			.event("BridgeTransferInitiated(bytes32,address,bytes32,uint256)")
			.event("BridgeTransferCompleted(bytes32,bytes32)")
			.from_block(BlockNumberOrTag::Latest);

		let sub = ws.subscribe_logs(&filter).await?;
		let mut sub_stream = sub.into_stream();

		// Spawn a task to forward events to the listener channel
		let (sender, _) =
			tokio::sync::mpsc::unbounded_channel::<AbstractBlockainEvent<EthAddress, EthHash>>();

		tokio::spawn(async move {
			while let Some(log) = sub_stream.next().await {
				let event = AbstractBlockainEvent::InitiatorContractEvent(Ok(
					convert_log_to_event(EthAddress(initiator_address), log),
				));
				if sender.send(event).is_err() {
					tracing::error!("Failed to send event to listener channel");
					break;
				}
			}
		});

		Ok(Self { listener, ws })
	}
}

impl Stream for EthInitiatorMonitoring<EthAddress, EthHash> {
	type Item = BridgeContractInitiatorEvent<
		<Self as BridgeContractInitiatorMonitoring>::Address,
		<Self as BridgeContractInitiatorMonitoring>::Hash,
	>;

	fn poll_next(self: Pin<&mut Self>, cx: &mut std::task::Context) -> Poll<Option<Self::Item>> {
		let this = self.get_mut();
		if let Poll::Ready(Some(AbstractBlockainEvent::InitiatorContractEvent(contract_result))) =
			this.listener.poll_next_unpin(cx)
		{
			tracing::trace!(
				"InitiatorContractMonitoring: Received contract event: {:?}",
				contract_result
			);

			// Only listen to the initiator contract events
			match contract_result {
				Ok(contract_event) => match contract_event {
					BridgeContractInitiatorEvent::Initiated(details) => {
						return Poll::Ready(Some(BridgeContractInitiatorEvent::Initiated(details)));
					}
					BridgeContractInitiatorEvent::Completed(id) => {
						return Poll::Ready(Some(BridgeContractInitiatorEvent::Completed(id)))
					}
					BridgeContractInitiatorEvent::Refunded(id) => {
						return Poll::Ready(Some(BridgeContractInitiatorEvent::Refunded(id)))
					}
				},
				Err(e) => {
					tracing::error!("Error in contract event: {:?}", e);
				}
			}
		}
		Poll::Pending
	}
}

fn convert_log_to_event(
	address: EthAddress,
	log: Log,
) -> BridgeContractInitiatorEvent<EthAddress, EthHash> {
	let initiated_log = AtomicBridgeInitiator::BridgeTransferInitiated::SIGNATURE_HASH;
	let completed_log = AtomicBridgeInitiator::BridgeTransferCompleted::SIGNATURE_HASH;
	let refunded_log = AtomicBridgeInitiator::BridgeTransferRefunded::SIGNATURE_HASH;

	// Extract details from the log and map to event type
	let topics = log.topics();
	let data = log.data().clone();

	// Assuming the first topic is the event type identifier
	let topic = topics.get(0).expect("Expected event type in topics");

	match topic {
		t if t == &initiated_log => {
			// Decode the data for Initiated event
			let tokens = decode_log_data(
				address,
				EventName::Initiated.as_str(),
				data,
				vec![
					AlloyParam::BridgeTransferId.fill(),
					AlloyParam::InitiatorAddress.fill(),
					AlloyParam::RecipientAddress.fill(),
					AlloyParam::HashLock.fill(),
					AlloyParam::TimeLock.fill(),
					AlloyParam::Amount.fill(),
				],
			);

			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(tokens[0].clone().into_fixed_bytes().unwrap()));
			let initiator_address = InitiatorAddress(EthAddress(FixedBytes(
				tokens[1].clone().into_address().unwrap().0,
			)));
			let recipient_address = RecipientAddress(tokens[2].clone().into_fixed_bytes().unwrap());
			let hash_lock = HashLock(EthHash::from(tokens[3].clone().into_fixed_bytes().unwrap()));
			let time_lock = TimeLock(tokens[4].clone().into_uint().unwrap().as_u64());
			let amount = Amount(tokens[5].clone().into_uint().unwrap().as_u64());

			let details = BridgeTransferDetails {
				bridge_transfer_id,
				initiator_address,
				recipient_address,
				hash_lock,
				time_lock,
				amount,
			};

			BridgeContractInitiatorEvent::Initiated(details)
		}
		t if t == &completed_log => {
			// Decode the data for Completed event
			let tokens = decode_log_data(
				address,
				EventName::Completed.as_str(),
				data,
				vec![AlloyParam::BridgeTransferId.fill(), AlloyParam::PreImage.fill()],
			);
			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(tokens[0].clone().into_fixed_bytes().unwrap()));

			BridgeContractInitiatorEvent::Completed(bridge_transfer_id)
		}
		t if t == &refunded_log => {
			// Decode the data for Refunded event
			let tokens = decode_log_data(
				address,
				EventName::Refunded.as_str(),
				data,
				vec![AlloyParam::BridgeTransferId.fill()],
			);
			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(tokens[0].clone().into_fixed_bytes().unwrap()));

			BridgeContractInitiatorEvent::Refunded(bridge_transfer_id)
		}
		_ => unimplemented!("Unexpected event type"), //Return proper error type here
	}
}

fn decode_log_data(
	address: EthAddress,
	name: &str,
	data: LogData,
	params: Vec<Param>,
) -> Result<BridgeContractInitiatorEvent<EthAddress, EthHash>, anyhow::Error> {
	let event = Event {
		name: name.to_string(),
		inputs: params
			.iter()
			.map(|p| EventParam {
				ty: p.to_string(),
				name: p.name,
				indexed: false,
				components: vec![],
				internal_type: None, // for now
			})
			.collect(),
		anonymous: false,
	};

	let raw_log = RawLog { address, topics: vec![], data: data.clone() };
	let decoded = event.parse_log(raw_log)?.params;

	let event_name = EventName::from(name);
	match event_name {
		EventName::Completed => {
			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(decoded[0].value.clone().into_fixed_bytes()?));
			let initiator_address = InitiatorAddress(EthAddress(FixedBytes(
				decoded[1].value.clone().into_address()?.0,
			)));
			let recipient_address = RecipientAddress(decoded[2].value.clone().into_fixed_bytes()?);
			let hash_lock = HashLock(EthHash::from(decoded[3].value.clone().into_fixed_bytes()?));
			let time_lock = TimeLock(decoded[4].value.clone().into_uint()?.as_u64());
			let amount = Amount(decoded[5].value.clone().into_uint()?.as_u64());

			let details = BridgeTransferDetails {
				bridge_transfer_id,
				initiator_address,
				recipient_address,
				hash_lock,
				time_lock,
				amount,
			};

			Ok(BridgeContractInitiatorEvent::Initiated(details))
		}
		EventName::Completed => {
			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(decoded[0].value.clone().into_fixed_bytes()?));

			Ok(BridgeContractInitiatorEvent::Completed(bridge_transfer_id))
		}
		EventName::Refunded => {
			let bridge_transfer_id =
				BridgeTransferId(EthHash::from(decoded[0].value.clone().into_fixed_bytes()?));

			Ok(BridgeContractInitiatorEvent::Refunded(bridge_transfer_id))
		}
		_ => Err(anyhow::anyhow!("Unexpected event type")),
	}
}
