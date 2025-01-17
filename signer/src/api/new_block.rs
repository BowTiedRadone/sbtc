//! This module contains the handler for the `POST /new_block` endpoint,
//! which is for processing new block webhooks from a stacks node.
//!

use axum::extract::State;
use axum::http::StatusCode;
use clarity::vm::representations::ContractName;
use clarity::vm::types::QualifiedContractIdentifier;
use clarity::vm::types::StandardPrincipalData;
use emily_client::models::Chainstate;
use emily_client::models::CreateWithdrawalRequestBody;
use emily_client::models::DepositUpdate;
use emily_client::models::Fulfillment;
use emily_client::models::Status;
use emily_client::models::UpdateDepositsResponse;
use emily_client::models::UpdateWithdrawalsResponse;
use emily_client::models::Withdrawal;
use emily_client::models::WithdrawalParameters;
use emily_client::models::WithdrawalUpdate;
use futures::FutureExt;
use std::sync::OnceLock;

use crate::context::Context;
use crate::emily_client::EmilyInteract;
use crate::error::Error;
use crate::stacks::events::CompletedDepositEvent;
use crate::stacks::events::RegistryEvent;
use crate::stacks::events::TxInfo;
use crate::stacks::events::WithdrawalAcceptEvent;
use crate::stacks::events::WithdrawalCreateEvent;
use crate::stacks::events::WithdrawalRejectEvent;
use crate::stacks::webhooks::NewBlockEvent;
use crate::storage::model::BitcoinBlockHash;
use crate::storage::model::StacksBlock;
use crate::storage::model::StacksBlockHash;
use crate::storage::DbWrite;

use super::ApiState;
use super::SBTC_REGISTRY_CONTRACT_NAME;

/// The address for the sbtc-registry smart contract. This value is
/// populated using the deployer variable in the config.
///
/// Although the stacks node is supposed to only send sbtc-registry events,
/// the node can be misconfigured or have some bug where it sends other
/// events as well. Accepting such events would be a security issue, so we
/// filter out events that are not from the sbtc-registry.
///
/// See https://github.com/stacks-network/sbtc/issues/501.
static SBTC_REGISTRY_IDENTIFIER: OnceLock<QualifiedContractIdentifier> = OnceLock::new();

/// An enum representing the result of the event processing.
/// This is used to send the results of the events to Emily.
enum UpdateResult {
    Deposit(Result<UpdateDepositsResponse, Error>),
    Withdrawal(Result<UpdateWithdrawalsResponse, Error>),
    CreatedWithdrawal(Vec<Result<Withdrawal, Error>>),
    Chainstate(Result<Chainstate, Error>),
}

/// A handler of `POST /new_block` webhook events.
///
/// # Notes
///
/// The event dispatcher functionality in a stacks node attempts to send
/// the payload to all interested observers, one-by-one. If the node fails
/// to connect to one of the observers, or if the response from the
/// observer is not a 200-299 response code, then it sleeps for 1 second
/// and tries again[^1]. From the looks of it, the node will not stop
/// trying to send the webhook until there is a success. Because of this,
/// unless we encounter an error where retrying in a second might succeed,
/// we will return a 200 OK status code.
///
/// TODO: We need to be careful to only return a non success status code a
/// fixed number of times.
///
/// [^1]: <https://github.com/stacks-network/stacks-core/blob/09c4b066e25104be8b066e8f7530ff0c6df4ccd5/testnet/stacks-node/src/event_dispatcher.rs#L317-L385>
pub async fn new_block_handler(state: State<ApiState<impl Context>>, body: String) -> StatusCode {
    tracing::debug!("Received a new block event from stacks-core");
    let api = state.0;

    let registry_address = SBTC_REGISTRY_IDENTIFIER.get_or_init(|| {
        // Although the following line can panic, our unit tests hit this
        // code path so if tests pass then this will work in production.
        let contract_name = ContractName::from(SBTC_REGISTRY_CONTRACT_NAME);
        let issuer = StandardPrincipalData::from(api.ctx.config().signer.deployer);
        QualifiedContractIdentifier::new(issuer, contract_name)
    });

    let new_block_event: NewBlockEvent = match serde_json::from_str(&body) {
        Ok(value) => value,
        // If we are here, then we failed to deserialize the webhook body
        // into the expected type. It's unlikely that retying this webhook
        // will lead to success, so we log the error and return `200 OK` so
        // that the node does not retry the webhook.
        Err(error) => {
            tracing::error!(%body, %error, "could not deserialize POST /new_block webhook:");
            return StatusCode::OK;
        }
    };

    // Although transactions can fail, only successful transactions emit
    // sBTC print events, since those events are emitted at the very end of
    // the contract call.
    let events = new_block_event
        .events
        .into_iter()
        .filter(|x| x.committed)
        .filter_map(|x| x.contract_event.map(|ev| (ev, x.txid)))
        .filter(|(ev, _)| &ev.contract_identifier == registry_address && ev.topic == "print");

    let stacks_chaintip = StacksBlock {
        block_hash: StacksBlockHash::from(new_block_event.index_block_hash),
        block_height: new_block_event.block_height,
        parent_hash: StacksBlockHash::from(new_block_event.parent_index_block_hash),
        bitcoin_anchor: BitcoinBlockHash::from(new_block_event.burn_block_hash),
    };
    let block_id = new_block_event.index_block_hash;
    let bitcoin_block_hash = new_block_event.burn_block_hash.to_hex();
    let bitcoin_block_height = new_block_event.burn_block_height as u64;

    // Create vectors to store the processed events for Emily.
    let mut completed_deposits = Vec::new();
    let mut updated_withdrawals = Vec::new();
    let mut created_withdrawals = Vec::new();

    for (ev, txid) in events {
        let tx_info = TxInfo { txid, block_id };
        let res = match RegistryEvent::try_new(ev.value, tx_info) {
            Ok(RegistryEvent::CompletedDeposit(event)) => handle_completed_deposit(
                &api.ctx,
                event,
                &stacks_chaintip,
                bitcoin_block_hash.clone(),
                bitcoin_block_height,
            )
            .await
            .map(|x| completed_deposits.push(x)),
            Ok(RegistryEvent::WithdrawalAccept(event)) => handle_withdrawal_accept(
                &api.ctx,
                event,
                &stacks_chaintip,
                bitcoin_block_hash.clone(),
                bitcoin_block_height,
            )
            .await
            .map(|x| updated_withdrawals.push(x)),
            Ok(RegistryEvent::WithdrawalReject(event)) => {
                handle_withdrawal_reject(&api.ctx, event, &stacks_chaintip)
                    .await
                    .map(|x| updated_withdrawals.push(x))
            }
            Ok(RegistryEvent::WithdrawalCreate(event)) => {
                handle_withdrawal_create(&api.ctx, event, stacks_chaintip.block_height)
                    .await
                    .map(|x| created_withdrawals.push(x))
            }
            Err(error) => {
                tracing::error!(%error, "Got an error when transforming the event ClarityValue");
                return StatusCode::OK;
            }
        };
        // If we got an error writing to the database, this might be an
        // issue that will resolve itself if we try again in a few moments.
        // So we return a non success status code so that the node retries
        // in a second.
        if let Err(Error::SqlxQuery(error)) = res {
            tracing::error!(%error, "Got an error when writing event to database");
            return StatusCode::INTERNAL_SERVER_ERROR;
        // If we got an error processing the event, we log the error and
        // return a success status code so that the node does not retry the
        // webhook. We rely on the redundancy of the other sBTC signers to
        // ensure that the update is sent to Emily.
        } else if let Err(error) = res {
            tracing::error!(%error, "Got an error when processing event");
        }
    }

    // Send the updates to Emily.
    let emily_client = api.ctx.get_emily_client();
    let chainstate = Chainstate::new(block_id.to_string(), new_block_event.block_height);
    let futures = vec![
        emily_client
            .update_deposits(completed_deposits)
            .map(UpdateResult::Deposit)
            .boxed(),
        emily_client
            .update_withdrawals(updated_withdrawals)
            .map(UpdateResult::Withdrawal)
            .boxed(),
        emily_client
            .create_withdrawals(created_withdrawals)
            .map(UpdateResult::CreatedWithdrawal)
            .boxed(),
        emily_client
            .set_chainstate(chainstate)
            .map(UpdateResult::Chainstate)
            .boxed(),
    ];
    // TODO: Ideally, we would use `futures::future::join_all` here, but Emily
    // randomly returns a `VersionConflict` error when we send multiple
    // requests that may update the chainstate.
    // let results = futures::future::join_all(futures).await;

    // Log any errors that occurred while updating Emily.
    // We don't return a non-success status code here because we rely on
    // the redundancy of the other sBTC signers to ensure that the update
    // is sent to Emily.
    for future in futures {
        match future.await {
            UpdateResult::Chainstate(Err(error)) => {
                tracing::warn!(%error, "Failed to set chainstate in Emily");
            }
            UpdateResult::Deposit(Err(error)) => {
                tracing::warn!(%error, "Failed to update deposits in Emily");
            }
            UpdateResult::Withdrawal(Err(error)) => {
                tracing::warn!(%error, "Failed to update withdrawals in Emily");
            }
            UpdateResult::CreatedWithdrawal(results) => {
                for result in results {
                    if let Err(error) = result {
                        tracing::warn!(%error, "Failed to create withdrawals in Emily");
                    }
                }
            }
            _ => {} // Ignore successful results.
        }
    }
    StatusCode::OK
}

/// Processes a completed deposit event by updating relevant deposit records
/// and preparing data to be sent to Emily.
///
/// # Parameters
/// - `ctx`: Shared application context containing configuration and database access.
/// - `event`: The deposit event to be processed.
/// - `stacks_chaintip`: Current chaintip information for the Stacks blockchain,
///   including block height and hash.
/// - `bitcoin_block_hash`: The hash of the Bitcoin block containing the
///   fullfilling tx.
/// - `bitcoin_block_height`: The height of the Bitcoin block containing the
///   fullfilling tx.
///
/// # Returns
/// - `Result<DepositUpdate, Error>`: On success, returns a `DepositUpdate` struct containing
///   information on the completed deposit to be sent to Emily.
///   In case of a database error, returns an `Error`
async fn handle_completed_deposit(
    ctx: &impl Context,
    event: CompletedDepositEvent,
    stacks_chaintip: &StacksBlock,
    // TODO (#493): We need the `bitcoin_block_hash` and `bitcoin_block_height`
    // of the block that included the fulfilling Bitcoin transaction.
    // After #493 is resolved, this value should be contained in the event itself
    // and these parameters should be removed.
    bitcoin_block_hash: String,
    bitcoin_block_height: u64,
) -> Result<DepositUpdate, Error> {
    ctx.get_storage_mut()
        .write_completed_deposit_event(&event)
        .await?;

    Ok(DepositUpdate {
        bitcoin_tx_output_index: event.outpoint.vout,
        bitcoin_txid: event.outpoint.txid.to_string(),
        status: Status::Confirmed,
        fulfillment: Some(Some(Box::new(Fulfillment {
            bitcoin_block_hash,
            bitcoin_block_height,
            bitcoin_tx_index: event.outpoint.vout,
            bitcoin_txid: event.outpoint.txid.to_string(),
            btc_fee: 1, // TODO (#712): We need to get the fee from the transaction. Currently missing from the event.
            stacks_txid: event.txid.to_hex(),
        }))),
        status_message: format!("Included in block {}", event.block_id.to_hex()),
        last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
        last_update_height: stacks_chaintip.block_height,
    })
}

/// Handles a withdrawal acceptance event, updating database records and
/// preparing a response for Emily.
///
/// # Parameters
/// - `ctx`: Shared application context with configuration and database access.
/// - `event`: The withdrawal acceptance event to be processed.
/// - `bitcoin_block_hash`: The hash of the Bitcoin block containing the
///   fullfilling tx.
/// - `bitcoin_block_height`: The height of the Bitcoin block containing the
///   fullfilling tx.
/// - `stacks_chaintip`: Current Stacks blockchain chaintip information for
///   context on block height and hash.
///
/// # Returns
/// - `Result<WithdrawalUpdate, Error>`: On success, returns a `WithdrawalUpdate` struct
///   for Emily containing relevant withdrawal information.
///   In case of a database error, returns an `Error`
async fn handle_withdrawal_accept(
    ctx: &impl Context,
    event: WithdrawalAcceptEvent,
    stacks_chaintip: &StacksBlock,
    // TODO (#493): We need the `bitcoin_block_hash` and `bitcoin_block_height`
    // of the block that included the fulfilling Bitcoin transaction.
    // After #493 is resolved, this value should be contained in the event itself
    // and these parameters should be removed.
    bitcoin_block_hash: String,
    bitcoin_block_height: u64,
) -> Result<WithdrawalUpdate, Error> {
    ctx.get_storage_mut()
        .write_withdrawal_accept_event(&event)
        .await?;

    Ok(WithdrawalUpdate {
        request_id: event.request_id,
        status: Status::Confirmed,
        fulfillment: Some(Some(Box::new(Fulfillment {
            bitcoin_block_hash,
            bitcoin_block_height,
            bitcoin_tx_index: event.outpoint.vout,
            bitcoin_txid: event.outpoint.txid.to_string(),
            btc_fee: event.fee,
            stacks_txid: event.txid.to_hex(),
        }))),
        status_message: format!("Included in block {}", event.block_id.to_hex()),
        last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
        last_update_height: stacks_chaintip.block_height,
    })
}

/// Processes a withdrawal creation event, adding new withdrawal records to the
/// database and preparing the data for Emily.
///
/// # Parameters
/// - `ctx`: Shared application context containing configuration and database access.
/// - `event`: The withdrawal creation event to be processed.
/// - `stacks_block_height`: The height of the Stacks block containing the withdrawal tx.
///
/// # Returns
/// - `Result<CreateWithdrawalRequestBody, Error>`: On success, returns a `CreateWithdrawalRequestBody`
///   with withdrawal information. In case of a database error, returns an `Error`
async fn handle_withdrawal_create(
    ctx: &impl Context,
    event: WithdrawalCreateEvent,
    stacks_block_height: u64,
) -> Result<CreateWithdrawalRequestBody, Error> {
    ctx.get_storage_mut()
        .write_withdrawal_create_event(&event)
        .await?;

    Ok(CreateWithdrawalRequestBody {
        amount: event.amount,
        parameters: Box::new(WithdrawalParameters { max_fee: event.max_fee }),
        recipient: event.recipient.to_string(),
        request_id: event.request_id,
        stacks_block_hash: event.block_id.to_hex(),
        stacks_block_height,
    })
}

/// Processes a withdrawal rejection event by updating records and preparing
/// the response data to be sent to Emily.
///
/// # Parameters
/// - `ctx`: Shared application context containing configuration and database access.
/// - `event`: The withdrawal rejection event to be processed.
/// - `stacks_chaintip`: Information about the current chaintip of the Stacks blockchain,
///   such as block height and hash.
///
/// # Returns
/// - `Result<WithdrawalUpdate, Error>`: Returns a `WithdrawalUpdate` with rejection information.
///   In case of a database error, returns an `Error`.
async fn handle_withdrawal_reject(
    ctx: &impl Context,
    event: WithdrawalRejectEvent,
    stacks_chaintip: &StacksBlock,
) -> Result<WithdrawalUpdate, Error> {
    ctx.get_storage_mut()
        .write_withdrawal_reject_event(&event)
        .await?;

    Ok(WithdrawalUpdate {
        fulfillment: None,
        last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
        last_update_height: stacks_chaintip.block_height,
        request_id: event.request_id,
        status: Status::Failed,
        status_message: "Rejected".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use bitcoin::OutPoint;
    use bitcoin::ScriptBuf;
    use bitvec::array::BitArray;
    use clarity::vm::types::PrincipalData;
    use emily_client::models::UpdateDepositsResponse;
    use emily_client::models::UpdateWithdrawalsResponse;
    use fake::Fake;
    use rand::rngs::OsRng;
    use rand::SeedableRng as _;
    use test_case::test_case;

    use crate::storage::in_memory::Store;
    use crate::storage::model::StacksPrincipal;
    use crate::testing::context::*;
    use crate::testing::storage::model::TestData;

    /// These were generated from a stacks node after running the
    /// "complete-deposit standard recipient", "accept-withdrawal",
    /// "create-withdrawal", and "reject-withdrawal" variants,
    /// respectively, of the `complete_deposit_wrapper_tx_accepted`
    /// integration test.
    const COMPLETED_DEPOSIT_WEBHOOK: &str =
        include_str!("../../tests/fixtures/completed-deposit-event.json");

    const WITHDRAWAL_ACCEPT_WEBHOOK: &str =
        include_str!("../../tests/fixtures/withdrawal-accept-event.json");

    const WITHDRAWAL_CREATE_WEBHOOK: &str =
        include_str!("../../tests/fixtures/withdrawal-create-event.json");

    const WITHDRAWAL_REJECT_WEBHOOK: &str =
        include_str!("../../tests/fixtures/withdrawal-reject-event.json");

    #[test_case(COMPLETED_DEPOSIT_WEBHOOK, |db| db.completed_deposit_events.get(&OutPoint::null()).is_none(); "completed-deposit")]
    #[test_case(WITHDRAWAL_CREATE_WEBHOOK, |db| db.withdrawal_create_events.get(&1).is_none(); "withdrawal-create")]
    #[test_case(WITHDRAWAL_ACCEPT_WEBHOOK, |db| db.withdrawal_accept_events.get(&1).is_none(); "withdrawal-accept")]
    #[test_case(WITHDRAWAL_REJECT_WEBHOOK, |db| db.withdrawal_reject_events.get(&2).is_none(); "withdrawal-reject")]
    #[tokio::test]
    async fn test_events<F>(body_str: &str, table_is_empty: F)
    where
        F: Fn(tokio::sync::MutexGuard<'_, Store>) -> bool,
    {
        let mut ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let api = ApiState { ctx: ctx.clone() };

        let db = ctx.inner_storage();

        // Hey look, there is nothing here!
        assert!(table_is_empty(db.lock().await));

        let state = State(api);
        let body = body_str.to_string();

        let new_block_event = serde_json::from_str::<NewBlockEvent>(&body).unwrap();
        // Set up the mock expectation for set_chainstate
        let chainstate = Chainstate::new(
            new_block_event.index_block_hash.to_string(),
            new_block_event.block_height,
        );
        ctx.with_emily_client(|client| {
            client.expect_set_chainstate().times(1).returning(move |_| {
                let chainstate = chainstate.clone();
                Box::pin(async { Ok(chainstate) })
            });
            client
                .expect_update_deposits()
                .times(1)
                .returning(move |_| {
                    Box::pin(async { Ok(UpdateDepositsResponse { deposits: vec![] }) })
                });
            client
                .expect_update_withdrawals()
                .times(1)
                .returning(move |_| {
                    Box::pin(async { Ok(UpdateWithdrawalsResponse { withdrawals: vec![] }) })
                });
            client
                .expect_create_withdrawals()
                .times(1)
                .returning(move |_| Box::pin(async { vec![] }));
        })
        .await;

        let res = new_block_handler(state, body).await;
        assert_eq!(res, StatusCode::OK);

        // Now there should be something here
        assert!(!table_is_empty(db.lock().await));
    }

    #[test_case(COMPLETED_DEPOSIT_WEBHOOK, |db| db.completed_deposit_events.get(&OutPoint::null()).is_none(); "completed-deposit")]
    #[test_case(WITHDRAWAL_CREATE_WEBHOOK, |db| db.withdrawal_create_events.get(&1).is_none(); "withdrawal-create")]
    #[test_case(WITHDRAWAL_ACCEPT_WEBHOOK, |db| db.withdrawal_accept_events.get(&1).is_none(); "withdrawal-accept")]
    #[test_case(WITHDRAWAL_REJECT_WEBHOOK, |db| db.withdrawal_reject_events.get(&2).is_none(); "withdrawal-reject")]
    #[tokio::test]
    async fn test_fishy_events<F>(body_str: &str, table_is_empty: F)
    where
        F: Fn(tokio::sync::MutexGuard<'_, Store>) -> bool,
    {
        let mut ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let api = ApiState { ctx: ctx.clone() };

        let db = ctx.inner_storage();

        // Hey look, there is nothing here!
        assert!(table_is_empty(db.lock().await));

        // Okay, we want to make sure that events that are from an
        // unexpected contract are filtered out. So we manually switch the
        // address to some random one and check the output. To do that we
        // do a string replace for the expected one with the fishy one.
        let issuer = StandardPrincipalData::from(ctx.config().signer.deployer);
        let contract_name = ContractName::from(SBTC_REGISTRY_CONTRACT_NAME);
        let identifier = QualifiedContractIdentifier::new(issuer, contract_name.clone());

        let fishy_principal: StacksPrincipal = fake::Faker.fake_with_rng(&mut OsRng);
        let fishy_issuer = match PrincipalData::from(fishy_principal) {
            PrincipalData::Contract(contract) => contract.issuer,
            PrincipalData::Standard(standard) => standard,
        };
        let fishy_identifier = QualifiedContractIdentifier::new(fishy_issuer, contract_name);

        let body = body_str.replace(&identifier.to_string(), &fishy_identifier.to_string());
        // Okay let's check that it was actually replaced.
        assert!(body.contains(&fishy_identifier.to_string()));

        // Let's check that we can still deserialize the JSON string since
        // the `new_block_handler` function will return early with
        // StatusCode::OK on failure to deserialize.
        let new_block_event = serde_json::from_str::<NewBlockEvent>(&body).unwrap();
        let events: Vec<_> = new_block_event
            .events
            .into_iter()
            .filter_map(|x| x.contract_event)
            .collect();

        // An extra check that we have events with our fishy identifier.
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .all(|x| x.contract_identifier == fishy_identifier));

        // Set up the mock expectation for set_chainstate
        let chainstate = Chainstate::new(
            new_block_event.index_block_hash.to_string(),
            new_block_event.block_height,
        );

        ctx.with_emily_client(|client| {
            client.expect_set_chainstate().times(1).returning(move |_| {
                let chainstate = chainstate.clone();
                Box::pin(async { Ok(chainstate) })
            });
            client
                .expect_update_deposits()
                .times(1)
                .returning(move |_| {
                    Box::pin(async { Ok(UpdateDepositsResponse { deposits: vec![] }) })
                });
            client
                .expect_update_withdrawals()
                .times(1)
                .returning(move |_| {
                    Box::pin(async { Ok(UpdateWithdrawalsResponse { withdrawals: vec![] }) })
                });
            client
                .expect_create_withdrawals()
                .times(1)
                .returning(move |_| Box::pin(async { vec![] }));
        })
        .await;
        // Okay now to do the check.
        let state = State(api.clone());
        let res = new_block_handler(state, body).await;
        assert_eq!(res, StatusCode::OK);

        // This event should be filtered out, so the table should still be
        // empty.
        assert!(table_is_empty(db.lock().await));
    }

    /// Tests handling a completed deposit event.
    /// This function validates that a completed deposit is correctly processed,
    /// including verifying the successful database update.
    #[tokio::test]
    async fn test_handle_completed_deposit() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        let ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let test_params = crate::testing::storage::model::Params {
            num_bitcoin_blocks: 1,
            num_stacks_blocks_per_bitcoin_block: 1,
            num_deposit_requests_per_block: 1,
            num_withdraw_requests_per_block: 1,
            num_signers_per_request: 0,
        };
        let db = ctx.inner_storage();
        let test_data = TestData::generate(&mut rng, &[], &test_params);

        let txid = test_data.bitcoin_transactions[0].txid;
        let bitcoin_block = &test_data.bitcoin_blocks[0];
        let stacks_chaintip = &test_data.stacks_blocks[0];
        let stacks_txid = test_data.stacks_transactions[0].txid;

        let outpoint = OutPoint { txid: *txid, vout: 0 };
        let event = CompletedDepositEvent {
            outpoint: outpoint.clone(),
            txid: *stacks_txid,
            block_id: *stacks_chaintip.block_hash,
            amount: 100,
        };
        let expectation = DepositUpdate {
            bitcoin_tx_output_index: event.outpoint.vout,
            bitcoin_txid: txid.to_string(),
            status: Status::Confirmed,
            fulfillment: Some(Some(Box::new(Fulfillment {
                bitcoin_block_hash: bitcoin_block.block_hash.to_string(),
                bitcoin_block_height: bitcoin_block.block_height,
                bitcoin_tx_index: event.outpoint.vout,
                bitcoin_txid: txid.to_string(),
                btc_fee: 1,
                stacks_txid: stacks_txid.to_hex(),
            }))),
            status_message: format!("Included in block {}", stacks_chaintip.block_hash.to_hex()),
            last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
            last_update_height: stacks_chaintip.block_height,
        };
        let res = handle_completed_deposit(
            &ctx,
            event,
            stacks_chaintip,
            bitcoin_block.block_hash.to_string(),
            bitcoin_block.block_height,
        )
        .await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expectation);
        let db = db.lock().await;
        assert_eq!(db.completed_deposit_events.len(), 1);
        assert!(db.completed_deposit_events.get(&outpoint).is_some());
    }

    /// Tests handling a withdrawal acceptance event.
    /// This function validates that when a withdrawal is accepted, the handler
    /// correctly updates the database and returns the expected response.
    #[tokio::test]
    async fn test_handle_withdrawal_accept() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        let ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let test_params = crate::testing::storage::model::Params {
            num_bitcoin_blocks: 2,
            num_stacks_blocks_per_bitcoin_block: 1,
            num_deposit_requests_per_block: 2,
            num_withdraw_requests_per_block: 2,
            num_signers_per_request: 0,
        };

        let db = ctx.inner_storage();

        let test_data = TestData::generate(&mut rng, &[], &test_params);

        let txid = test_data.bitcoin_transactions[0].txid;
        let stacks_tx = &test_data.stacks_transactions[0];
        let bitcoin_block = &test_data.bitcoin_blocks[0];
        let stacks_chaintip = test_data
            .stacks_blocks
            .last()
            .expect("STX block generation failed");

        let event = WithdrawalAcceptEvent {
            request_id: 1,
            outpoint: OutPoint { txid: *txid, vout: 0 },
            txid: *stacks_tx.txid,
            block_id: *stacks_tx.block_hash,
            fee: 1,
            signer_bitmap: BitArray::<_>::ZERO,
        };

        // Expected struct to be added to the accepted_withdrawals vector
        let expectation = WithdrawalUpdate {
            request_id: event.request_id,
            status: Status::Confirmed,
            fulfillment: Some(Some(Box::new(Fulfillment {
                bitcoin_block_hash: bitcoin_block.block_hash.to_string(),
                bitcoin_block_height: bitcoin_block.block_height,
                bitcoin_tx_index: event.outpoint.vout,
                bitcoin_txid: txid.to_string(),
                btc_fee: event.fee,
                stacks_txid: stacks_tx.txid.to_hex(),
            }))),
            status_message: format!("Included in block {}", event.block_id.to_hex()),
            last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
            last_update_height: stacks_chaintip.block_height,
        };
        let res = handle_withdrawal_accept(
            &ctx,
            event,
            stacks_chaintip,
            bitcoin_block.block_hash.to_string(),
            bitcoin_block.block_height,
        )
        .await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expectation);
        let db = db.lock().await;
        assert_eq!(db.withdrawal_accept_events.len(), 1);
        assert!(db
            .withdrawal_accept_events
            .get(&expectation.request_id)
            .is_some());
    }

    /// Tests handling of a withdrawal request.
    /// This test confirms that when a withdrawal is created, the system updates
    /// the database correctly and returns the expected response.
    #[tokio::test]
    async fn test_handle_withdrawal_create() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        let ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let test_params = crate::testing::storage::model::Params {
            num_bitcoin_blocks: 2,
            num_stacks_blocks_per_bitcoin_block: 1,
            num_deposit_requests_per_block: 2,
            num_withdraw_requests_per_block: 2,
            num_signers_per_request: 0,
        };

        let db = ctx.inner_storage();
        let test_data = TestData::generate(&mut rng, &[], &test_params);

        let stacks_first_tx = &test_data.stacks_transactions[0];
        let stacks_first_block = &test_data.stacks_blocks[0];

        let event = WithdrawalCreateEvent {
            request_id: 1,
            block_id: *stacks_first_tx.block_hash,
            amount: 100,
            max_fee: 1,
            recipient: ScriptBuf::default(),
            txid: *stacks_first_tx.txid,
            sender: PrincipalData::Standard(StandardPrincipalData::transient()),
            block_height: test_data.bitcoin_blocks[0].block_height,
        };

        // Expected struct to be added to the created_withdrawals vector
        let expectation = CreateWithdrawalRequestBody {
            amount: event.amount,
            parameters: Box::new(WithdrawalParameters { max_fee: event.max_fee }),
            recipient: event.recipient.to_string(),
            request_id: event.request_id,
            stacks_block_hash: stacks_first_block.block_hash.to_hex(),
            stacks_block_height: stacks_first_block.block_height,
        };

        let res = handle_withdrawal_create(&ctx, event, stacks_first_block.block_height).await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expectation);
        let db = db.lock().await;
        assert_eq!(db.withdrawal_create_events.len(), 1);
        assert!(db
            .withdrawal_create_events
            .get(&expectation.request_id)
            .is_some());
    }

    /// Tests handling a withdrawal rejection event.
    /// This function checks that a rejected withdrawal transaction is processed
    /// correctly, including updating the database and returning the expected response.
    #[tokio::test]
    async fn test_handle_withdrawal_reject() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        let ctx = TestContext::builder()
            .with_in_memory_storage()
            .with_mocked_clients()
            .build();

        let db = ctx.inner_storage();

        let test_params = crate::testing::storage::model::Params {
            num_bitcoin_blocks: 2,
            num_stacks_blocks_per_bitcoin_block: 1,
            num_deposit_requests_per_block: 2,
            num_withdraw_requests_per_block: 2,
            num_signers_per_request: 0,
        };

        let test_data = TestData::generate(&mut rng, &[], &test_params);

        let stacks_chaintip = test_data
            .stacks_blocks
            .last()
            .expect("STX block generation failed");

        let event = WithdrawalRejectEvent {
            request_id: 1,
            block_id: *stacks_chaintip.block_hash,
            txid: *test_data.stacks_transactions[0].txid,
            signer_bitmap: BitArray::<_>::ZERO,
        };

        // Expected struct to be added to the rejected_withdrawals vector
        let expectation = WithdrawalUpdate {
            request_id: event.request_id,
            status: Status::Failed,
            fulfillment: None,
            last_update_block_hash: stacks_chaintip.block_hash.to_hex(),
            last_update_height: stacks_chaintip.block_height,
            status_message: "Rejected".to_string(),
        };

        let res = handle_withdrawal_reject(&ctx, event, stacks_chaintip).await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), expectation);
        let db = db.lock().await;
        assert_eq!(db.withdrawal_reject_events.len(), 1);
        assert!(db
            .withdrawal_reject_events
            .get(&expectation.request_id)
            .is_some());
    }
}
