//! validation of bitcoin transactions.

use std::ops::Deref as _;

use bitcoin::relative::LockTime;
use bitcoin::Amount;
use bitcoin::OutPoint;
use bitcoin::TxOut;

use crate::context::Context;
use crate::error::Error;
use crate::keys::PublicKey;
use crate::storage::model::BitcoinBlockHash;
use crate::storage::model::BitcoinTx;
use crate::storage::model::QualifiedRequestId;
use crate::storage::model::ScriptPubKey;
use crate::storage::DbRead;
use crate::DEPOSIT_LOCKTIME_BLOCK_BUFFER;

/// The necessary information for validating a bitcoin transaction.
#[derive(Debug, Clone)]
pub struct BitcoinTxContext {
    /// This signer's current view of the chain tip of the canonical
    /// bitcoin blockchain. It is the block hash and height of the block on
    /// the bitcoin blockchain with the greatest height. On ties, we sort
    /// by the block hash descending and take the first one.
    pub chain_tip: BitcoinBlockHash,
    /// The block height of the bitcoin chain tip identified by the
    /// `chain_tip` field.
    pub chain_tip_height: u64,
    /// How many bitcoin blocks back from the chain tip the signer will
    /// look for requests.
    pub tx: BitcoinTx,
    /// The requests
    pub request_ids: Vec<QualifiedRequestId>,
    /// The public key of the signer that created the deposit request
    /// transaction. This is very unlikely to ever be used in the
    /// [`BitcoinTx::validate`] function, but is here for logging and
    /// tracking purposes.
    pub origin: PublicKey,
}

impl BitcoinTxContext {
    /// 1. All deposit requests consumed by the bitcoin transaction are
    ///    accepted by the signer.
    /// 2. All withdraw requests fulfilled by the bitcoin transaction are
    ///    accepted by the signer.
    /// 3. The apportioned transaction fee for each request does not exceed
    ///    any max_fee.
    /// 4. All transaction inputs are spendable by the signers.
    /// 5. Any transaction outputs that aren't fulfilling withdraw requests
    ///    are spendable by the signers or unspendable.
    /// 6. Each deposit request input has an associated amount that is
    ///    greater than their assessed fee.
    /// 7. There is at least 2 blocks and 2 hours of lock-time left before
    ///    the depositor can reclaim their funds.
    /// 8. Each deposit is on the canonical bitcoin blockchain.
    pub async fn validate<C>(&self, ctx: &C) -> Result<(), Error>
    where
        C: Context + Send + Sync,
    {
        let signer_amount = self.validate_signer_input(ctx).await?;
        let deposit_amounts = self.validate_deposits(ctx).await?;

        self.validate_signer_outputs(ctx).await?;
        self.validate_withdrawals(ctx).await?;

        let input_amounts = signer_amount + deposit_amounts;

        self.validate_fees(input_amounts)?;
        Ok(())
    }

    fn validate_fees(&self, _input_amounts: Amount) -> Result<(), Error> {
        let _output_amounts = self
            .tx
            .output
            .iter()
            .map(|tx_out| tx_out.value)
            .sum::<Amount>();

        Ok(())
    }

    /// Validate the signers' input UTXO
    async fn validate_signer_input<C>(&self, ctx: &C) -> Result<Amount, Error>
    where
        C: Context + Send + Sync,
    {
        let db = ctx.get_storage();
        let Some(signer_txo_input) = self.tx.input.first() else {
            return Err(BitcoinSignerInputError::MissingInputs.into_error(self));
        };
        let signer_txo_txid = signer_txo_input.previous_output.txid.into();

        let Some(signer_tx) = db.get_bitcoin_tx(&signer_txo_txid).await? else {
            return Err(BitcoinSignerInputError::InvalidPrevout.into_error(self));
        };

        // This as usize cast is fine because we only support CPU
        // architectures with 32 or 64 bit pointer widths.
        let output_index = signer_txo_input.previous_output.vout as usize;
        let Ok(signer_prevout_utxo) = signer_tx.tx_out(output_index) else {
            return Err(BitcoinSignerInputError::PrevoutMissingFromSourceTx.into_error(self));
        };
        let script = signer_prevout_utxo.script_pubkey.clone().into();

        if !db.is_signer_script_pub_key(&script).await? {
            return Err(BitcoinSignerInputError::InvalidPrevout.into_error(self));
        }

        Ok(signer_prevout_utxo.value)
    }

    /// Validate the signer outputs.
    ///
    /// Each sweep transaction has two signer outputs, the new UTXO with
    /// all of the signers' funds and an `OP_RETURN` TXO. This function
    /// validates both of them.
    async fn validate_signer_outputs<C>(&self, ctx: &C) -> Result<(), Error>
    where
        C: Context + Send + Sync,
    {
        let db = ctx.get_storage();
        let Some(signer_txo_output) = self.tx.output.first() else {
            return Err(BitcoinSignerOutputError::InvalidOpReturnOutput.into_error(self));
        };

        let script = signer_txo_output.script_pubkey.clone().into();

        if !db.is_signer_script_pub_key(&script).await? {
            return Err(BitcoinSignerOutputError::InvalidOpReturnOutput.into_error(self));
        }

        Ok(())
    }

    /// Validate each of the prevouts that coorespond to deposits. This
    /// should be every input except for the first one.
    async fn validate_deposits<C>(&self, ctx: &C) -> Result<Amount, Error>
    where
        C: Context + Send + Sync,
    {
        let db = ctx.get_storage();
        let signer_public_key = PublicKey::from_private_key(&ctx.config().signer.private_key);
        // 1. All deposit requests consumed by the bitcoin transaction are
        //    accepted by the signer.

        let mut deposit_amount = 0;

        for tx_in in self.tx.input.iter().skip(1) {
            let outpoint = tx_in.previous_output;
            let txid = outpoint.txid.into();
            let report_future = db.get_deposit_request_report(
                &self.chain_tip,
                &txid,
                tx_in.previous_output.vout,
                &signer_public_key,
            );

            let Some(report) = report_future.await? else {
                return Err(BitcoinDepositInputError::Unknown(outpoint).into_error(self));
            };

            deposit_amount += report.amount;

            report
                .validate(self.chain_tip_height)
                .map_err(|err| err.into_error(self))?;
        }

        Ok(Amount::from_sat(deposit_amount))
    }

    /// Validate the withdrawal UTXOs
    async fn validate_withdrawals<C>(&self, ctx: &C) -> Result<(), Error>
    where
        C: Context + Send + Sync,
    {
        let db = ctx.get_storage();

        if self.tx.output.len() != self.request_ids.len() + 2 {
            return Err(BitcoinWithdrawalOutputError::Unknown.into_error(self));
        }

        let withdrawal_iter = self.tx.output.iter().skip(2).zip(self.request_ids.iter());
        for (utxo, req_id) in withdrawal_iter {
            let Some(report) = db.get_withdrawal_request(req_id).await? else {
                return Err(BitcoinWithdrawalOutputError::Unknown.into_error(self));
            };

            report.validate(utxo).map_err(|err| err.into_error(self))?;
        }
        Ok(())
    }
}

/// The responses for validation of a sweep transaction on bitcoin.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Copy, Clone)]
pub enum BitcoinSignerInputError {
    /// The signers' input TXO is not locked with a scriptPubKey that the
    /// signer knows about.
    #[error("signers' UTXO locked with incorrect scriptPubKey")]
    InvalidPrevout,
    /// The signer is not part of the signer set that generated the public
    /// key locking the input.
    #[error("the signer is not part of the signing set for the aggregate public key")]
    CannotSignUtxo,
    /// The transaction is missing inputs...
    #[error("the transaction is missing inputs")]
    MissingInputs,
    /// The signers' input TXO is not locked with a scriptPubKey that the
    /// signer knows about.
    #[error("we do not have a record of the transaction pointed to by the signers' prevout")]
    PrevoutTxMissing,
    /// We have a record of the transaction pointed to by the signer
    /// prevout, but output at the specified index is unknown.
    #[error("the transaction is missing inputs")]
    PrevoutMissingFromSourceTx,
}

/// The responses for validation of a sweep transaction on bitcoin.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Copy, Clone)]
pub enum BitcoinDepositInputError {
    /// The assessed exceeds the max-fee in the deposit request.
    #[error("the assessed fee for a deposit would exceed their max-fee; {0}")]
    AssessedFeeTooHigh(OutPoint),
    /// The signer is not part of the signer set that generated the
    /// aggregate public key used to lock the deposit funds.
    ///
    /// TODO: For v1 every signer should be able to sign for all deposits,
    /// but for v2 this will not be the case. So we'll need to decide
    /// whether a particular deposit cannot be signed by a particular
    /// signers means that the entire transaction is rejected from that
    /// signer.
    #[error("the signer is not part of the signing set for the aggregate public key; {0}")]
    CannotSignUtxo(OutPoint),
    /// The deposit transaction has been confirmed on a bitcoin block
    /// that is not part of the canonical bitcoin blockchain.
    #[error("deposit transaction not on canonical bitcoin blockchain; {0}")]
    TxNotOnBestChain(OutPoint),
    /// The deposit UTXO has already been spent.
    #[error("deposit transaction used as input in confirmed sweep transaction; {0}")]
    DepositUtxoSpent(OutPoint),
    /// Given the current time and block height, it would be imprudent to
    /// attempt to sweep in a deposit request with the given lock-time.
    #[error("lock-time expiration is too soon; {0}")]
    LockTimeExpiry(OutPoint),
    /// The signer does not have a record of their vote on the deposit
    /// request in their database.
    #[error("the signer does not have a record of their vote on the deposit request; {0}")]
    NoVote(OutPoint),
    /// The signer has rejected the deposit request.
    #[error("the signer has not accepted the deposit request; {0}")]
    RejectedRequest(OutPoint),
    /// The signer does not have a record of the deposit request in their
    /// database.
    #[error("the signer does not have a record of the deposit request; {0}")]
    Unknown(OutPoint),
    /// The locktime in the reclaim script is in time units and that is not
    /// supported. This shouldn't happen, since we will not put it in our
    /// database is this is the case.
    #[error("the deposit locktime is denoted in time and that is not supported; {0}")]
    UnsupportedLockTime(OutPoint),
}

/// The responses for validation of a sweep transaction on bitcoin.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Copy, Clone)]
pub enum BitcoinSignerOutputError {
    /// The signers' UTXO is not locked with the latest aggregate public
    /// key.
    #[error("signers' UTXO locked with incorrect scriptPubKey")]
    InvalidSignerUtxo,
    /// All UTXOs must be either the signers, an OP_RETURN UTXO with zero
    /// amount, or a UTXO servicing a withdrawal request.
    #[error("one of the UTXOs is unexpected")]
    InvalidUtxo,
    /// The OP_RETURN UTXO must have an amount of zero and include the
    /// expected signer bitmap, and merkle tree.
    #[error("signers' OP_RETURN output does not match what is expected")]
    InvalidOpReturnOutput,
}

/// The responses for validation of a sweep transaction on bitcoin.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Copy, Clone)]
pub enum BitcoinWithdrawalOutputError {
    /// The assessed exceeds the max-fee in the withdrawal request.
    #[error("the assessed fee for a withdrawal would exceed their max-fee")]
    AssessedWithdrawalFeeTooHigh,
    /// One of the output amounts does not match the amount in the withdrawal request.
    #[error("the amount in the withdrawal request does not match the amount in our records")]
    IncorrectWithdrawalAmount,
    /// One of the output amounts does not match the amount in the withdrawal request.
    #[error("the scriptPubKey for a withdrawal UTXO does not match the one in our records")]
    IncorrectWithdrawalRecipient,
    /// The OP_RETURN UTXO must have an amount of zero and include the
    /// expected signer bitmap, and merkle tree.
    #[error("signers' OP_RETURN output does not match what is expected")]
    InvalidOpReturnOutput,
    /// The signer does not have a record of the deposit request in our
    /// database.
    #[error("the signer does not have a record of the deposit request")]
    NoWithdrawalRequestVote,
    /// The signer has rejected the deposit request.
    #[error("the signer has not accepted the deposit request")]
    RejectedWithdrawalRequest,
    /// One of the output amounts does not match the amount in the withdrawal request.
    #[error("the signer does not have a record of the withdrawal request")]
    Unknown,
}

/// The responses for validation of a sweep transaction on bitcoin.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Copy, Clone)]
pub enum BitcoinSweepErrorMsg {
    /// The error has something to do with the inputs.
    #[error("the assessed fee for a deposit would exceed their max-fee")]
    Deposit(#[from] BitcoinDepositInputError),
    /// The error has something to do with the inputs.
    #[error("the assessed fee for a deposit would exceed their max-fee")]
    SignerInput(#[from] BitcoinSignerInputError),
    /// The error has something to do with the inputs.
    #[error("the assessed fee for a deposit would exceed their max-fee")]
    SignerOutput(#[from] BitcoinSignerOutputError),
    /// The error has something to do with the outputs.
    #[error("the assessed fee for a withdrawal would exceed their max-fee")]
    Withdrawal(#[from] BitcoinWithdrawalOutputError),
}

/// A struct for a bitcoin validation error containing all the necessary
/// context.
#[derive(Debug)]
pub struct BitcoinValidationError {
    /// The specific error that happened during validation.
    pub error: BitcoinSweepErrorMsg,
    /// The additional information that was used when trying to
    /// validate the complete-deposit contract call. This includes the
    /// public key of the signer that was attempting to generate the
    /// `complete-deposit` transaction.
    pub context: BitcoinTxContext,
}

impl std::fmt::Display for BitcoinValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO(191): Add the other variables to the error message.
        self.error.fmt(f)
    }
}

impl std::error::Error for BitcoinValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl BitcoinDepositInputError {
    fn into_error(self, ctx: &BitcoinTxContext) -> Error {
        Error::BitcoinValidation(Box::new(BitcoinValidationError {
            error: BitcoinSweepErrorMsg::Deposit(self),
            context: ctx.clone(),
        }))
    }
}

impl BitcoinSignerInputError {
    fn into_error(self, ctx: &BitcoinTxContext) -> Error {
        Error::BitcoinValidation(Box::new(BitcoinValidationError {
            error: BitcoinSweepErrorMsg::SignerInput(self),
            context: ctx.clone(),
        }))
    }
}

impl BitcoinSignerOutputError {
    fn into_error(self, ctx: &BitcoinTxContext) -> Error {
        Error::BitcoinValidation(Box::new(BitcoinValidationError {
            error: BitcoinSweepErrorMsg::SignerOutput(self),
            context: ctx.clone(),
        }))
    }
}

impl BitcoinWithdrawalOutputError {
    fn into_error(self, ctx: &BitcoinTxContext) -> Error {
        Error::BitcoinValidation(Box::new(BitcoinValidationError {
            error: BitcoinSweepErrorMsg::Withdrawal(self),
            context: ctx.clone(),
        }))
    }
}

/// An enum for the confirmation status of a deposit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositRequestStatus {
    /// We have a record of the deposit request transaction, and it has
    /// been confirmed on the canonical bitcoin blockchain. We have not
    /// spent these funds. The integer is the height of the block
    /// confirming the deposit request.
    Confirmed(u64),
    /// We have a record of the deposit request being included as an input
    /// in another bitcoin transaction that has been confirmed on the
    /// canonical bitcoin blockchain.
    Spent,
    /// We have a record of the deposit request transaction, and it has not
    /// been confirmed on the canonical bitcoin blockchain.
    ///
    /// Usually we will almost certainly have a record of a deposit
    /// request, and we require that the deposit transaction be confirmed
    /// before we write it to our database. But the deposit transaction can
    /// be affected by a bitcoin reorg, where it is no longer confirmed on
    /// the canonical bitcoin blockchain. If this happens when we query for
    /// the status then it will come back as unconfirmed.
    Unconfirmed,
}

/// A struct for the status report summary of a deposit request for use
/// in validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositRequestReport {
    /// The deposit UTXO outpoint that uniquely identifies the deposit.
    pub outpoint: OutPoint,
    /// The confirmation status of the deposit request transaction.
    pub status: DepositRequestStatus,
    /// Whether this signer was part of the signing set associated with the
    /// deposited funds. If the signer is not part of the signing set, then
    /// we do not do a check of whether we will accept it otherwise.
    ///
    /// This will only be `None` if we do not have a record of the deposit
    /// request.
    pub can_sign: Option<bool>,
    /// Whether this signer accepted the deposit request or not. This
    /// should only be `None` if we do not have a record of the deposit
    /// request or if we cannot sign for the deposited funds.
    pub is_accepted: Option<bool>,
    /// The deposit amount
    pub amount: u64,
    /// The lock_time in the reclaim script
    pub lock_time: LockTime,
}

impl DepositRequestReport {
    fn validate(self, chain_tip_height: u64) -> Result<(), BitcoinDepositInputError> {
        let confirmed_block_height = match self.status {
            // Deposit requests are only written to the database after they
            // have been confirmed, so this means that we have a record of
            // the request, but it has not been confirmed on the canonical
            // bitcoin blockchain.
            DepositRequestStatus::Unconfirmed => {
                return Err(BitcoinDepositInputError::TxNotOnBestChain(self.outpoint));
            }
            // This means that we have a record of the deposit UTXO being
            // spent in a sweep transaction that has been confirmed on the
            // canonical bitcoin blockchain.
            DepositRequestStatus::Spent => {
                return Err(BitcoinDepositInputError::DepositUtxoSpent(self.outpoint));
            }
            // The deposit has been confirmed on the canonical bitcoin
            // blockchain and remains unspent by us.
            DepositRequestStatus::Confirmed(block_height) => block_height,
        };

        match self.can_sign {
            // Although, we have a record for the deposit request, we
            // haven't voted on it ourselves yet, so we do not know if we
            // can sign for it.
            None => return Err(BitcoinDepositInputError::NoVote(self.outpoint)),
            // We know that we cannot sign for the deposit because it is
            // locked with a public key where the current signer is not
            // part of the signing set.
            Some(false) => return Err(BitcoinDepositInputError::CannotSignUtxo(self.outpoint)),
            // Yay.
            Some(true) => (),
        }
        // If we are here then can_sign is Some(true) so is_accepted is
        // Some(_). Let's check whether we rejected this deposit.
        if self.is_accepted != Some(true) {
            return Err(BitcoinDepositInputError::RejectedRequest(self.outpoint));
        }

        // We only sweep a deposit if the depositor cannot reclaim the
        // deposit within the next DEPOSIT_LOCKTIME_BLOCK_BUFFER blocks.
        let deposit_age = chain_tip_height.saturating_sub(confirmed_block_height);

        match self.lock_time {
            LockTime::Blocks(height) => {
                let max_age = height.value().saturating_sub(DEPOSIT_LOCKTIME_BLOCK_BUFFER) as u64;
                if deposit_age >= max_age {
                    return Err(BitcoinDepositInputError::LockTimeExpiry(self.outpoint));
                }
            }
            LockTime::Time(_) => {
                return Err(BitcoinDepositInputError::UnsupportedLockTime(self.outpoint))
            }
        }

        Ok(())
    }
}

/// An enum for the confirmation status of a deposit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalRequestStatus {
    /// We have a record of the withdrawal request event, and it has been
    /// confirmed on the canonical Stacks blockchain. It remains
    /// unfulfilled.
    Confirmed,
    /// We have a record of the withdrawal request event, and it has been
    /// confirmed on the canonical Stacks blockchain. It has been
    /// fulfilled.
    Fulfilled,
}

/// A struct for the status report summary of a withdrawal request for use
/// in validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalRequestReport {
    /// The identifier for the withdrawal request
    pub id: QualifiedRequestId,
    /// The confirmation status of the deposit request transaction.
    pub status: WithdrawalRequestStatus,
    /// Whether this signer was part of the signing set associated with the
    /// deposited funds. If the signer is not part of the signing set, then
    /// we do not do a check of whether we will accept it otherwise.
    ///
    /// This should only be None if we do not have a record of the deposit
    /// request.
    pub amount: u64,
    /// Whether this signer accepted the deposit request or not. This
    /// should only be None if we do not have a record of the deposit
    /// request or if we cannot sign for the deposited funds.
    pub recipient: ScriptPubKey,
    /// request or if we cannot sign for the deposited funds.
    pub max_fee: u64,
}

impl WithdrawalRequestReport {
    fn validate(self, utxo: &TxOut) -> Result<(), BitcoinWithdrawalOutputError> {
        match self.status {
            WithdrawalRequestStatus::Fulfilled => {
                return Err(BitcoinWithdrawalOutputError::Unknown);
            }
            WithdrawalRequestStatus::Confirmed => (),
        };

        if self.amount != utxo.value.to_sat() {
            return Err(BitcoinWithdrawalOutputError::IncorrectWithdrawalAmount);
        }

        if self.recipient.deref() != &utxo.script_pubkey {
            return Err(BitcoinWithdrawalOutputError::IncorrectWithdrawalRecipient);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    #[derive(Debug)]
    struct DepositReportErrorMapping {
        report: DepositRequestReport,
        error: Option<BitcoinDepositInputError>,
        chain_tip_height: u64,
    }

    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Unconfirmed,
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::TxNotOnBestChain(OutPoint::null())),
        chain_tip_height: 2,
    } ; "deposit-reorged")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Spent,
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::DepositUtxoSpent(OutPoint::null())),
        chain_tip_height: 2,
    } ; "deposit-spent")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: None,
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::NoVote(OutPoint::null())),
        chain_tip_height: 2,
    } ; "deposit-no-vote")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(false),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::CannotSignUtxo(OutPoint::null())),
        chain_tip_height: 2,
    } ; "cannot-sign-for-deposit")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(true),
            is_accepted: Some(false),
            amount: 0,
            lock_time: LockTime::from_height(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::RejectedRequest(OutPoint::null())),
        chain_tip_height: 2,
    } ; "rejected-deposit")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(DEPOSIT_LOCKTIME_BLOCK_BUFFER + 1),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::LockTimeExpiry(OutPoint::null())),
        chain_tip_height: 2,
    } ; "lock-time-expires-soon-1")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(DEPOSIT_LOCKTIME_BLOCK_BUFFER + 2),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::LockTimeExpiry(OutPoint::null())),
        chain_tip_height: 2,
    } ; "lock-time-expires-soon-2")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_512_second_intervals(u16::MAX),
            outpoint: OutPoint::null(),
        },
        error: Some(BitcoinDepositInputError::UnsupportedLockTime(OutPoint::null())),
        chain_tip_height: 2,
    } ; "lock-time-in-time-units-2")]
    #[test_case(DepositReportErrorMapping {
        report: DepositRequestReport {
            status: DepositRequestStatus::Confirmed(0),
            can_sign: Some(true),
            is_accepted: Some(true),
            amount: 0,
            lock_time: LockTime::from_height(DEPOSIT_LOCKTIME_BLOCK_BUFFER + 3),
            outpoint: OutPoint::null(),
        },
        error: None,
        chain_tip_height: 2,
    } ; "happy-path")]
    fn deposit_report_validation(mapping: DepositReportErrorMapping) {
        match mapping.error {
            Some(expected_error) => {
                let error = mapping
                    .report
                    .validate(mapping.chain_tip_height)
                    .unwrap_err();

                assert_eq!(error, expected_error);
            }
            None => mapping.report.validate(mapping.chain_tip_height).unwrap(),
        }
    }
}
