//! Database models for the signer.

use std::ops::Deref;

use bitcoin::hashes::Hash as _;
use bitcoin::Address;
use bitcoin::Network;
use clarity::vm::types::PrincipalData;
use sbtc::deposits::Deposit;
use serde::Deserialize;
use serde::Serialize;
use stacks_common::types::chainstate::StacksBlockId;

use crate::error::Error;
use crate::keys::PublicKey;
use crate::stacks::events;

/// Completed deposit event (from stacks event observer)
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
pub struct CompletedDepositEvent {
    /// The id of the stacks transaction that generated this event.
    pub txid: StacksTxId,
    /// This is the amount of sBTC to mint to the intended recipient.
    #[sqlx(try_from = "i64")]
    pub amount: u64,
    /// This is the outpoint of the original bitcoin deposit transaction.
    pub bitcoin_txid: BitcoinTxId,
    #[sqlx(try_from = "i64")]
    /// This is the output index of the original bitcoin deposit transaction.
    pub output_index: u32,
}

impl From<events::CompletedDepositEvent> for CompletedDepositEvent {
    fn from(event: events::CompletedDepositEvent) -> Self {
        Self {
            txid: event.txid.into(),
            amount: event.amount,
            bitcoin_txid: event.outpoint.txid.into(),
            output_index: event.outpoint.vout,
        }
    }
}

/// Withdrawal created event (from stacks event observer)
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
pub struct WithdrawalCreatedEvent {
    /// The id of the stacks transaction that generated this event.
    pub txid: StacksTxId,
    /// The id of the withdrawal request, as reported by the stacks node.
    #[sqlx(try_from = "i64")]
    pub request_id: u64,
    /// The amount of the withdrawal.
    #[sqlx(try_from = "i64")]
    pub amount: u64,
    /// The address which initiated the withdrawal request.
    pub sender: StacksPrincipal,
    /// The address which should receive the BTC withdrawal.
    pub recipient: BitcoinAddress,
    /// The maximum portion of the withdrawn amount that may be used to pay for 
    /// transaction fees.
    #[sqlx(try_from = "i64")]
    pub max_fee: u64,
    /// The stacks block height at which the withdrawal request was created.
    #[sqlx(try_from = "i64")]
    pub block_height: u64,
}

impl From<events::WithdrawalCreateEvent> for WithdrawalCreatedEvent {
    fn from(event: events::WithdrawalCreateEvent) -> Self {
        Self {
            txid: event.txid.into(),
            request_id: event.request_id,
            amount: event.amount,
            sender: event.sender.into(),
            recipient: event.recipient.to_string(),
            max_fee: event.max_fee,
            block_height: event.block_height,
        }
    }
}
/// Withdrawal accepted event (from stacks event observer)
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
pub struct WithdrawalAcceptedEvent {
    /// The id of the stacks transaction that generated this event.
    pub txid: StacksTxId,
    /// The id of the withdrawal request, as reported by the stacks node.
    #[sqlx(try_from = "i64")]
    pub request_id: u64,
    // TODO: sqlx decode
    //pub signer_bitmap: BitArray<[u8; 16]>,
    /// The bitcoin transaction ID of the withdrawal.
    pub bitcoin_txid: BitcoinTxId,
    #[sqlx(try_from = "i64")]
    /// The output index of the withdrawal.
    pub output_index: u32,
    /// The fee paid for the withdrawal.
    #[sqlx(try_from = "i64")]
    pub fee: u64,
}

impl From<events::WithdrawalAcceptEvent> for WithdrawalAcceptedEvent {
    fn from(event: events::WithdrawalAcceptEvent) -> Self {
        Self {
            txid: event.txid.into(),
            request_id: event.request_id,
            //signer_bitmap: event.signer_bitmap,
            bitcoin_txid: event.outpoint.txid.into(),
            output_index: event.outpoint.vout,
            fee: event.fee,
        }
    }
}

/// Bitcoin block.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct BitcoinBlock {
    /// Block hash.
    pub block_hash: BitcoinBlockHash,
    /// Block height.
    #[sqlx(try_from = "i64")]
    pub block_height: u64,
    /// Hash of the parent block.
    pub parent_hash: BitcoinBlockHash,
    /// Stacks block confirmed by this block.
    #[cfg_attr(feature = "testing", dummy(default))]
    pub confirms: Vec<StacksBlockHash>,
}

/// Stacks block.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct StacksBlock {
    /// Block hash.
    pub block_hash: StacksBlockHash,
    /// Block height.
    #[sqlx(try_from = "i64")]
    pub block_height: u64,
    /// Hash of the parent block.
    pub parent_hash: StacksBlockHash,
}

/// Deposit request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct DepositRequest {
    /// Transaction ID of the deposit request transaction.
    pub txid: BitcoinTxId,
    /// Index of the deposit request UTXO.
    #[cfg_attr(feature = "testing", dummy(faker = "0..100"))]
    #[sqlx(try_from = "i32")]
    pub output_index: u32,
    /// Script spendable by the sBTC signers.
    pub spend_script: Bytes,
    /// Script spendable by the depositor.
    pub reclaim_script: Bytes,
    /// The address of which the sBTC should be minted,
    /// can be a smart contract address.
    pub recipient: StacksPrincipal,
    /// The amount deposited.
    #[sqlx(try_from = "i64")]
    #[cfg_attr(feature = "testing", dummy(faker = "100..1_000_000_000_000"))]
    pub amount: u64,
    /// The maximum portion of the deposited amount that may
    /// be used to pay for transaction fees.
    #[sqlx(try_from = "i64")]
    #[cfg_attr(feature = "testing", dummy(faker = "100..1_000_000_000_000"))]
    pub max_fee: u64,
    /// The addresses of the input UTXOs funding the deposit request.
    #[cfg_attr(
        feature = "testing",
        dummy(faker = "crate::testing::dummy::BitcoinAddresses(1..5)")
    )]
    pub sender_addresses: Vec<BitcoinAddress>,
}

impl DepositRequest {
    /// Create me from a full deposit.
    pub fn from_deposit(deposit: &Deposit, network: Network) -> Self {
        let tx_input_iter = deposit.tx.input.iter();
        // It's most likely the case that each of the inputs "come" from
        // the same Address, so we filter out duplicates.
        let sender_addresses: std::collections::HashSet<String> = tx_input_iter
            .flat_map(|tx_in| {
                Address::from_script(&tx_in.script_sig, network)
                    .inspect_err(|err| tracing::warn!("could not create address: {err}"))
                    .map(|address| address.to_string())
            })
            .collect();
        Self {
            txid: deposit.info.outpoint.txid.into(),
            output_index: deposit.info.outpoint.vout,
            spend_script: deposit.info.deposit_script.to_bytes(),
            reclaim_script: deposit.info.reclaim_script.to_bytes(),
            recipient: deposit.info.recipient.clone().into(),
            amount: deposit.info.amount,
            max_fee: deposit.info.max_fee,
            sender_addresses: sender_addresses.into_iter().collect(),
        }
    }
}

/// A signer acknowledging a deposit request.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct DepositSigner {
    /// TxID of the deposit request.
    pub txid: BitcoinTxId,
    /// Output index of the deposit request.
    #[cfg_attr(feature = "testing", dummy(faker = "0..100"))]
    #[sqlx(try_from = "i32")]
    pub output_index: u32,
    /// Public key of the signer.
    pub signer_pub_key: PublicKey,
    /// Signals if the signer is prepared to sign for this request.
    pub is_accepted: bool,
}

/// Withdraw request.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct WithdrawRequest {
    /// Request ID of the withdrawal request.
    #[sqlx(try_from = "i64")]
    pub request_id: u64,
    /// Stacks block hash of the withdrawal request.
    pub block_hash: StacksBlockHash,
    /// The address that should receive the BTC withdrawal.
    pub recipient: BitcoinAddress,
    /// The amount to withdraw.
    #[sqlx(try_from = "i64")]
    #[cfg_attr(feature = "testing", dummy(faker = "100..1_000_000_000_000"))]
    pub amount: u64,
    /// The maximum portion of the withdrawn amount that may
    /// be used to pay for transaction fees.
    #[sqlx(try_from = "i64")]
    #[cfg_attr(feature = "testing", dummy(faker = "100..10000"))]
    pub max_fee: u64,
    /// The address that initiated the request.
    pub sender_address: StacksPrincipal,
}

/// A signer acknowledging a withdrawal request.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct WithdrawSigner {
    /// Request ID of the withdrawal request.
    #[sqlx(try_from = "i64")]
    pub request_id: u64,
    /// Stacks block hash of the withdrawal request.
    pub block_hash: StacksBlockHash,
    /// Public key of the signer.
    pub signer_pub_key: PublicKey,
    /// Signals if the signer is prepared to sign for this request.
    pub is_accepted: bool,
}

/// A connection between a bitcoin block and a bitcoin transaction.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
pub struct BitcoinTransaction {
    /// Transaction ID.
    pub txid: BitcoinTxId,
    /// The block in which the transaction exists.
    pub block_hash: BitcoinBlockHash,
}

/// A connection between a bitcoin block and a bitcoin transaction.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct StacksTransaction {
    /// Transaction ID.
    pub txid: StacksTxId,
    /// The block in which the transaction exists.
    pub block_hash: StacksBlockHash,
}

/// For writing to the stacks_transactions or bitcoin_transactions table.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransactionIds {
    /// Transaction IDs.
    pub tx_ids: Vec<[u8; 32]>,
    /// The blocks in which the transactions exist.
    pub block_hashes: Vec<[u8; 32]>,
}

/// A raw transaction on either Bitcoin or Stacks.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct Transaction {
    /// Transaction ID.
    pub txid: [u8; 32],
    /// Encoded transaction.
    pub tx: Bytes,
    /// The type of the transaction.
    pub tx_type: TransactionType,
    /// The block id of the stacks block that includes this transaction
    pub block_hash: [u8; 32],
}

/// Persisted DKG shares
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct EncryptedDkgShares {
    /// The aggregate key for these shares
    pub aggregate_key: PublicKey,
    /// The tweaked aggregate key for these shares
    pub tweaked_aggregate_key: PublicKey,
    /// The `scriptPubKey` for the aggregate public key.
    pub script_pubkey: Bytes,
    /// The encrypted DKG shares
    pub encrypted_private_shares: Bytes,
    /// The public DKG shares
    pub public_shares: Bytes,
}

/// Persisted public DKG shares from other signers
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::FromRow)]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
pub struct RotateKeysTransaction {
    /// Transaction ID.
    pub txid: StacksTxId,
    /// The aggregate key for these shares.
    pub aggregate_key: PublicKey,
    /// The public keys of the signers.
    pub signer_set: Vec<PublicKey>,
    /// The number of signatures required for the multi-sig wallet.
    #[sqlx(try_from = "i32")]
    pub signatures_required: u16,
}

/// The types of transactions the signer is interested in.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, sqlx::Type, strum::Display)]
#[sqlx(type_name = "sbtc_signer.transaction_type", rename_all = "snake_case")]
#[cfg_attr(feature = "testing", derive(fake::Dummy))]
#[strum(serialize_all = "snake_case")]
pub enum TransactionType {
    /// An sBTC transaction on Bitcoin.
    SbtcTransaction,
    /// A deposit request transaction on Bitcoin.
    DepositRequest,
    /// A withdrawal request transaction on Stacks.
    WithdrawRequest,
    /// A deposit accept transaction on Stacks.
    DepositAccept,
    /// A withdrawal accept transaction on Stacks.
    WithdrawAccept,
    /// A withdraw reject transaction on Stacks.
    WithdrawReject,
    /// A rotate keys call on Stacks.
    RotateKeys,
}

/// The bitcoin transaction ID
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitcoinTxId(bitcoin::Txid);

impl BitcoinTxId {
    /// Return the inner bytes for the block hash
    pub fn into_bytes(&self) -> [u8; 32] {
        self.0.to_byte_array()
    }
}

impl From<bitcoin::Txid> for BitcoinTxId {
    fn from(value: bitcoin::Txid) -> Self {
        Self(value)
    }
}

impl From<BitcoinTxId> for bitcoin::Txid {
    fn from(value: BitcoinTxId) -> Self {
        value.0
    }
}

impl From<[u8; 32]> for BitcoinTxId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bitcoin::Txid::from_byte_array(bytes))
    }
}

/// Bitcoin block hash
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BitcoinBlockHash(bitcoin::BlockHash);

impl BitcoinBlockHash {
    /// Return the inner bytes for the block hash
    pub fn into_bytes(&self) -> [u8; 32] {
        self.0.to_byte_array()
    }
}

impl AsRef<[u8; 32]> for BitcoinBlockHash {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

impl From<bitcoin::BlockHash> for BitcoinBlockHash {
    fn from(value: bitcoin::BlockHash) -> Self {
        Self(value)
    }
}

impl From<&BitcoinBlockHash> for bitcoin::BlockHash {
    fn from(value: &BitcoinBlockHash) -> Self {
        value.0
    }
}

impl From<BitcoinBlockHash> for bitcoin::BlockHash {
    fn from(value: BitcoinBlockHash) -> Self {
        value.0
    }
}

impl From<[u8; 32]> for BitcoinBlockHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bitcoin::BlockHash::from_byte_array(bytes))
    }
}

/// The stacks block Id. This is different from the block header hash.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StacksBlockHash(StacksBlockId);

impl Deref for StacksBlockHash {
    type Target = StacksBlockId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<StacksBlockId> for StacksBlockHash {
    fn from(value: StacksBlockId) -> Self {
        Self(value)
    }
}

impl From<[u8; 32]> for StacksBlockHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(StacksBlockId(bytes))
    }
}

/// Stacks transaction ID
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StacksTxId(blockstack_lib::burnchains::Txid);

impl Deref for StacksTxId {
    type Target = blockstack_lib::burnchains::Txid;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<blockstack_lib::burnchains::Txid> for StacksTxId {
    fn from(value: blockstack_lib::burnchains::Txid) -> Self {
        Self(value)
    }
}

impl From<[u8; 32]> for StacksTxId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(blockstack_lib::burnchains::Txid(bytes))
    }
}

/// A stacks address. It can be either a smart contract address or a
/// standard address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StacksPrincipal(PrincipalData);

impl Deref for StacksPrincipal {
    type Target = PrincipalData;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::str::FromStr for StacksPrincipal {
    type Err = Error;
    fn from_str(literal: &str) -> Result<Self, Self::Err> {
        let principal = PrincipalData::parse(literal).map_err(Error::ParsePrincipalData)?;
        Ok(Self(principal))
    }
}

impl From<PrincipalData> for StacksPrincipal {
    fn from(value: PrincipalData) -> Self {
        Self(value)
    }
}

impl From<StacksPrincipal> for PrincipalData {
    fn from(value: StacksPrincipal) -> Self {
        value.0
    }
}

impl Ord for StacksPrincipal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (&self.0, &other.0) {
            (PrincipalData::Contract(x), PrincipalData::Contract(y)) => x.cmp(y),
            (PrincipalData::Standard(x), PrincipalData::Standard(y)) => x.cmp(y),
            (PrincipalData::Standard(x), PrincipalData::Contract(y)) => {
                x.cmp(&y.issuer).then(std::cmp::Ordering::Less)
            }
            (PrincipalData::Contract(x), PrincipalData::Standard(y)) => {
                x.issuer.cmp(y).then(std::cmp::Ordering::Greater)
            }
        }
    }
}

impl PartialOrd for StacksPrincipal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Arbitrary bytes
pub type Bytes = Vec<u8>;
/// Bitcoin address
pub type BitcoinAddress = String;
