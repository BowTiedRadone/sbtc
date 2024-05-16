use bitcoin::absolute::LockTime;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::SECP256K1;
use bitcoin::sighash::Prevouts;
use bitcoin::sighash::SighashCache;
use bitcoin::taproot::LeafVersion;
use bitcoin::taproot::NodeInfo;
use bitcoin::taproot::Signature;
use bitcoin::taproot::TaprootSpendInfo;
use bitcoin::transaction::Version;
use bitcoin::Address;
use bitcoin::Amount;
use bitcoin::OutPoint;
use bitcoin::ScriptBuf;
use bitcoin::Sequence;
use bitcoin::TapLeafHash;
use bitcoin::TapSighash;
use bitcoin::TapSighashType;
use bitcoin::Transaction;
use bitcoin::TxIn;
use bitcoin::TxOut;
use bitcoin::Witness;
use bitcoin::XOnlyPublicKey;
use secp256k1::Keypair;
use secp256k1::Message;

use crate::error::Error;
use crate::packaging::compute_optimal_packages;
use crate::packaging::Weighted;

#[derive(Debug, Clone, Copy)]
pub struct SignerBtcState {
    /// The outstanding signer UTXO.
    pub utxo: SignerUtxo,
    /// The current market fee rate in sat/vByte.
    pub fee_rate: u64,
    /// The current public key of the signers
    pub public_key: XOnlyPublicKey,
}

#[derive(Debug)]
pub struct SbtcRequests {
    /// Accepted and pending deposit requests.
    pub deposits: Vec<DepositRequest>,
    /// Accepted and pending withdrawal requests.
    pub withdrawals: Vec<WithdrawalRequest>,
    /// Summary of the Signers' UTXO and information necessary for
    /// constructing their next UTXO.
    pub signer_state: SignerBtcState,
    /// The minimum acceptable number of votes for any given request.
    pub accept_threshold: u32,
    /// The total number of signers.
    pub num_signers: u32,
}

impl SbtcRequests {
    /// Construct the next transaction package given requests and the
    /// signers' UTXO.
    ///
    /// This function can fail if the output amounts are greater than the
    /// input amounts.
    pub fn construct_transactions(&self) -> Result<Vec<UnsignedTransaction>, Error> {
        if self.deposits.is_empty() && self.withdrawals.is_empty() {
            tracing::info!("No deposits or withdrawals so no BTC transaction");
            return Ok(Vec::new());
        }

        let withdrawals = self.withdrawals.iter().map(Request::Withdrawal);
        let deposits = self.deposits.iter().map(Request::Deposit);

        // Create a list of requests where each request can be approved on its own.
        let items = deposits.chain(withdrawals);

        compute_optimal_packages(items, self.reject_capacity())
            .scan(self.signer_state, |state, requests| {
                let tx = UnsignedTransaction::new(requests, state);
                if let Ok(tx_ref) = tx.as_ref() {
                    state.utxo = tx_ref.new_signer_utxo();
                }
                Some(tx)
            })
            .collect()
    }

    fn reject_capacity(&self) -> u32 {
        self.num_signers.saturating_sub(self.accept_threshold)
    }
}

#[derive(Debug)]
pub struct DepositRequest {
    /// The UTXO to be spent by the signers.
    pub outpoint: OutPoint,
    /// The max fee amount to use for the BTC deposit transaction.
    pub max_fee: u64,
    /// How each of the signers voted for the transaction.
    pub signer_bitmap: Vec<bool>,
    /// The amount of sats in the deposit UTXO.
    pub amount: u64,
    /// The deposit script used so that the signers' can spend funds.
    pub deposit_script: ScriptBuf,
    /// The redeem script for the deposit.
    pub redeem_script: ScriptBuf,
    /// The public key used for the key-spend path of the taproot script.
    ///
    /// Note that taproot Schnorr public keys are slightly different from
    /// the usual compressed public keys since they use only the x-coordinate
    /// with the y-coordinate assumed to be even. This means they use
    /// 32 bytes instead of the 33 byte public keys used before where the
    /// additional byte indicated the y-coordinate's parity.
    pub taproot_public_key: XOnlyPublicKey,
    /// The public key used in the deposit script. The signers public key
    /// is a Schnorr public key.
    pub signers_public_key: XOnlyPublicKey,
}

impl DepositRequest {
    /// Returns the number of signers who voted against this request.
    fn votes_against(&self) -> u32 {
        self.signer_bitmap.iter().map(|vote| !vote as u32).sum()
    }

    /// Create a TxIn object with witness data for the deposit script of
    /// the given request. Only a valid signature is needed to satisfy the
    /// deposit script.
    fn as_tx_input(&self, signature: Signature) -> TxIn {
        TxIn {
            previous_output: self.outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0),
            witness: self.construct_witness_data(signature),
        }
    }

    /// Construct the deposit UTXO associated with this deposit request.
    fn as_tx_out(&self) -> TxOut {
        let ver = LeafVersion::TapScript;
        let merkle_root = self.construct_taproot_info(ver).merkle_root();

        TxOut {
            value: Amount::from_sat(self.amount),
            script_pubkey: ScriptBuf::new_p2tr(SECP256K1, self.taproot_public_key, merkle_root),
        }
    }

    /// Construct the witness data for the taproot script of the deposit.
    ///
    /// Deposit UTXOs are taproot spend what a "null" key spend path,
    /// a deposit script-path spend, and a redeem script-path spend. This
    /// function creates the witness data for the deposit script-path
    /// spend where the script takes only one piece of data as input, the
    /// signature. The deposit script is:
    ///
    ///   <data> OP_DROP OP_DUP OP_HASH160 <pubkey_hash> OP_EQUALVERIFY OP_CHECKSIG
    ///
    /// where <data> is the stacks deposit address and <pubkey_hash> is
    /// given by self.signers_public_key. The public key used for key-path
    /// spending is self.taproot_public_key, and is supposed to be a dummy
    /// public key.
    pub fn construct_witness_data(&self, signature: Signature) -> Witness {
        let ver = LeafVersion::TapScript;
        let taproot = self.construct_taproot_info(ver);

        // TaprootSpendInfo::control_block returns None if the key given,
        // (script, version), is not in the tree. But this key is definitely
        // in the tree (see the variable leaf1 in the `construct_taproot_info`
        // function).
        let control_block = taproot
            .control_block(&(self.deposit_script.clone(), ver))
            .expect("We just inserted the deposit script into the tree");

        let witness_data = [
            signature.to_vec(),
            self.signers_public_key.serialize().to_vec(),
            self.deposit_script.to_bytes(),
            control_block.serialize(),
        ];
        Witness::from_slice(&witness_data)
    }

    /// Constructs the taproot spending information for the UTXO associated
    /// with this deposit request.
    fn construct_taproot_info(&self, ver: LeafVersion) -> TaprootSpendInfo {
        // For such a simple tree, we construct it by hand.
        let leaf1 = NodeInfo::new_leaf_with_ver(self.deposit_script.clone(), ver);
        let leaf2 = NodeInfo::new_leaf_with_ver(self.redeem_script.clone(), ver);

        // A Result::Err is returned by NodeInfo::combine if the depth of
        // our taproot tree exceeds the maximum depth of taproot trees,
        // which is 128. We have two nodes so the depth is 1 so this will
        // never panic.
        let node =
            NodeInfo::combine(leaf1, leaf2).expect("This tree depth greater than max of 128");

        TaprootSpendInfo::from_node_info(SECP256K1, self.taproot_public_key, node)
    }
}

#[derive(Debug)]
pub struct WithdrawalRequest {
    /// The amount of BTC, in sats, to withdraw.
    pub amount: u64,
    /// The max fee amount to use for the sBTC deposit transaction.
    pub max_fee: u64,
    /// The address to spend the output.
    pub address: Address,
    /// How each of the signers voted for the transaction.
    pub signer_bitmap: Vec<bool>,
}

impl WithdrawalRequest {
    /// Returns the number of signers who voted against this request.
    fn votes_against(&self) -> u32 {
        self.signer_bitmap.iter().map(|vote| !vote as u32).sum()
    }

    /// Withdrawal UTXOs pay to the given address
    fn as_tx_output(&self) -> TxOut {
        TxOut {
            value: Amount::from_sat(self.amount),
            script_pubkey: self.address.script_pubkey(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Request<'a> {
    Deposit(&'a DepositRequest),
    Withdrawal(&'a WithdrawalRequest),
}

impl<'a> Request<'a> {
    pub fn as_withdrawal(&self) -> Option<&'a WithdrawalRequest> {
        match self {
            Request::Withdrawal(req) => Some(req),
            _ => None,
        }
    }

    pub fn as_deposit(&self) -> Option<&'a DepositRequest> {
        match self {
            Request::Deposit(req) => Some(req),
            _ => None,
        }
    }
}

impl<'a> Weighted for Request<'a> {
    fn weight(&self) -> u32 {
        match self {
            Self::Deposit(req) => req.votes_against(),
            Self::Withdrawal(req) => req.votes_against(),
        }
    }
}

/// An object for using UTXOs associated with the signers' peg wallet.
///
/// This object is useful for transforming the UTXO into valid input and
/// output in another transaction. Some notes:
///
/// * This struct assumes that the spend script for each signer UTXO uses
///   taproot. This is necessary because the signers collectively generate
///   Schnorr signatures, which requires taproot.
/// * The taproot script for each signer UTXO is a key-spend only script.
#[derive(Debug, Clone, Copy)]
pub struct SignerUtxo {
    /// The outpoint of the signers' UTXO
    pub outpoint: OutPoint,
    /// The amount associated with that UTXO
    pub amount: u64,
    /// The public key used to create the key-spend only taproot script.
    pub public_key: XOnlyPublicKey,
}

impl SignerUtxo {
    /// Create a TxIn object for the signers' UTXO
    ///
    /// The signers' UTXO is always a key-spend only taproot UTXO, so a
    /// valid signature is all that is needed to spend it.
    fn as_tx_input(&self, signature: &Signature) -> TxIn {
        TxIn {
            previous_output: self.outpoint,
            sequence: Sequence::ZERO,
            witness: Witness::p2tr_key_spend(signature),
            script_sig: ScriptBuf::new(),
        }
    }

    /// Construct the UTXO associated with this outpoint.
    fn as_tx_output(&self) -> TxOut {
        Self::new_tx_output(self.public_key, self.amount)
    }

    /// Construct the new signers' UTXO
    ///
    /// The signers' UTXO is always a key-spend only taproot UTXO.
    fn new_tx_output(public_key: XOnlyPublicKey, sats: u64) -> TxOut {
        let secp = Secp256k1::new();

        TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: ScriptBuf::new_p2tr(&secp, public_key, None),
        }
    }
}

/// Given a set of requests, create a BTC transaction that can be signed.
///
/// This BTC transaction in this struct has correct amounts but no witness
/// data for its UTXO inputs.
#[derive(Debug)]
pub struct UnsignedTransaction<'a> {
    /// The requests used to construct the transaction.
    pub requests: Vec<Request<'a>>,
    /// The BTC transaction that needs to be signed.
    pub tx: Transaction,
    /// The public key used for the public key of the signers' UTXO output.
    pub signer_public_key: XOnlyPublicKey,
    /// The amount of fees changed to each request.
    pub fee_per_request: u64,
    /// The signers' UTXO used as inputs to this transaction.
    pub signer_utxo: SignerBtcState,
}

/// A struct containing Taproot-tagged hashes used for computing taproot
/// signature hashes.
#[derive(Debug)]
pub struct SignatureHashes<'a> {
    /// The sighash of the signers' input UTXO for the transaction.
    pub signers: TapSighash,
    /// Each deposit request is associated with a UTXO input for the peg-in
    /// transaction. This field contains digests/signature hashes that need
    /// Schnorr signatures and the associated deposit request for each hash.
    pub deposits: Vec<(&'a DepositRequest, TapSighash)>,
}

impl<'a> UnsignedTransaction<'a> {
    /// Construct an unsigned transaction.
    ///
    /// This function can fail if the output amounts are greater than the
    /// input amounts.
    ///
    /// The returned BTC transaction has the following properties:
    ///   1. The amounts for each output has taken fees into consideration.
    ///   2. The signer input UTXO is the first input.
    ///   3. The signer output UTXO is the first output.
    ///   4. Each input needs a signature in the witness data.
    ///   5. There is no witness data for deposit UTXOs.
    pub fn new(requests: Vec<Request<'a>>, state: &SignerBtcState) -> Result<Self, Error> {
        // Construct a transaction base. This transaction's inputs have
        // witness data with dummy signatures so that our virtual size
        // estimates are accurate. Later we will update the fees and
        // remove the witness data.
        let mut tx = Self::new_transaction(&requests, state)?;
        // We now compute the fee that each request must pay given the
        // size of the transaction and the fee rate. Once we have the fee
        // we adjust the output amounts accordingly.
        let fee = Self::compute_request_fee(&tx, state.fee_rate);
        Self::adjust_amounts(&mut tx, fee);

        // Now we can reset the witness data.
        Self::reset_witness_data(&mut tx);

        Ok(Self {
            tx,
            requests,
            signer_public_key: state.public_key,
            fee_per_request: fee,
            signer_utxo: *state,
        })
    }

    /// Construct a "stub" BTC transaction from the given requests.
    ///
    /// The returned BTC transaction is signed with dummy signatures, so it
    /// has the same virtual size as a proper transaction. Note that the
    /// output amounts haven't been adjusted for fees.
    ///
    /// An Err is returned if the amounts withdrawn is greater than the sum
    /// of all the input amounts.
    fn new_transaction(reqs: &[Request], state: &SignerBtcState) -> Result<Transaction, Error> {
        let signature = Self::generate_dummy_signature();

        let deposits = reqs
            .iter()
            .filter_map(|req| Some(req.as_deposit()?.as_tx_input(signature)));
        let withdrawals = reqs
            .iter()
            .filter_map(|req| Some(req.as_withdrawal()?.as_tx_output()));

        let signer_input = state.utxo.as_tx_input(&signature);
        let signer_output_sats = Self::compute_signer_amount(reqs, state)?;
        let signer_output = SignerUtxo::new_tx_output(state.public_key, signer_output_sats);

        Ok(Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: std::iter::once(signer_input).chain(deposits).collect(),
            output: std::iter::once(signer_output).chain(withdrawals).collect(),
        })
    }

    /// Create the new SignerUtxo for this transaction.
    fn new_signer_utxo(&self) -> SignerUtxo {
        SignerUtxo {
            outpoint: OutPoint {
                txid: self.tx.compute_txid(),
                vout: 0,
            },
            amount: self.tx.output[0].value.to_sat(),
            public_key: self.signer_public_key,
        }
    }

    /// Constructs the set of digests that need to be signed before broadcasting
    /// the transaction.
    ///
    /// # Notes
    ///
    /// This function uses the fact certain invariants about this struct are
    /// upheld. They are
    /// 1. The first input to the Transaction in the `tx` field is the signers'
    ///    UTXO.
    /// 2. The other inputs to the Transaction in the `tx` field are ordered
    ///    the same order as DepositRequests in the `requests` field.
    ///
    /// Other noteworthy assumptions is that the signers' UTXO is always a
    /// key-spend path only taproot UTXO.
    pub fn construct_digests(&self) -> Result<SignatureHashes, Error> {
        let deposit_requests = self.requests.iter().filter_map(Request::as_deposit);
        let deposit_utxos = deposit_requests.clone().map(DepositRequest::as_tx_out);
        // All of the transaction's inputs are used to constuct the sighash
        // That is eventually signed
        let input_utxos: Vec<TxOut> = std::iter::once(self.signer_utxo.utxo.as_tx_output())
            .chain(deposit_utxos)
            .collect();

        let prevouts = Prevouts::All(input_utxos.as_slice());
        let sighash_type = TapSighashType::Default;
        let mut sighasher = SighashCache::new(&self.tx);
        // The signers' UTXO is always the first input in the transaction.
        // Moreover, the signers can only spend this UTXO using the taproot
        // key-spend path of UTXO.
        let signer_sighash =
            sighasher.taproot_key_spend_signature_hash(0, &prevouts, sighash_type)?;
        // Each deposit UTXO is spendable by using the script path spend
        // of the taproot address. These UTXO inputs are after the sole
        // signer UTXO input.
        let deposit_sighashes = deposit_requests
            .enumerate()
            .map(|(input_index, deposit)| {
                let index = input_index + 1;
                let script = deposit.deposit_script.as_script();
                let leaf_hash = TapLeafHash::from_script(script, LeafVersion::TapScript);

                sighasher
                    .taproot_script_spend_signature_hash(index, &prevouts, leaf_hash, sighash_type)
                    .map(|sighash| (deposit, sighash))
                    .map_err(Error::from)
            })
            .collect::<Result<_, _>>()?;

        // Combine the them all together to get an ordered list of taproot
        // signature hashes.
        Ok(SignatureHashes {
            signers: signer_sighash,
            deposits: deposit_sighashes,
        })
    }

    /// Compute the fee that each deposit and withdrawal request must pay
    /// for the transaction given the fee rate
    ///
    /// If each deposit and withdrawal associated with this transaction
    /// paid the fees returned by this function then the fee rate for the
    /// entire transaction will be at least as much as the fee rate.
    ///
    /// Note that each deposit and withdrawal pays an equal amount for the
    /// transaction. To compute this amount we divide the total fee by the
    /// number of requests in the transaction.
    fn compute_request_fee(tx: &Transaction, fee_rate: u64) -> u64 {
        let tx_fee = tx.vsize() as u64 * fee_rate;
        let num_requests = (tx.input.len() + tx.output.len()).saturating_sub(2) as u64;
        tx_fee.div_ceil(num_requests)
    }

    /// Compute the final amount for the signers' UTXO given the current
    /// UTXO amount and the incoming requests.
    ///
    /// This amount does not take into account fees.
    fn compute_signer_amount(reqs: &[Request], state: &SignerBtcState) -> Result<u64, Error> {
        let amount = reqs
            .iter()
            .fold(state.utxo.amount as i64, |amount, req| match req {
                Request::Deposit(req) => amount + req.amount as i64,
                Request::Withdrawal(req) => amount - req.amount as i64,
            });

        // This should never happen
        if amount < 0 {
            tracing::error!("Transaction deposits greater than the inputs!");
            return Err(Error::InvalidAmount(amount));
        }

        Ok(amount as u64)
    }

    /// Adjust the amounts for each output given the fee.
    ///
    /// This function adjusts each output by the given fee amount. The
    /// signers' UTXOs amount absorbs the fee on-chain that the depositors
    /// are supposed to pay. This amount must be accounted for when
    /// minting sBTC.
    fn adjust_amounts(tx: &mut Transaction, fee: u64) {
        // Since the first input and first output correspond to the signers'
        // UTXOs, we subtract them when computing the number of requests.
        let num_requests = (tx.input.len() + tx.output.len()).saturating_sub(2) as u64;
        // This is a bizarre case that should never happen.
        if num_requests == 0 {
            tracing::warn!("No deposit or withdrawal related inputs in the transaction");
            return;
        }

        // The first output is the signer's UTXO. To determine the correct
        // amount for this UTXO deduct the fee payable by the depositors
        // from the currently set amount. This deduction is reflected in
        // the amount of sBTC minted to each depositor.
        if let Some(utxo_out) = tx.output.first_mut() {
            let deposit_fees = fee * (tx.input.len() - 1) as u64;
            let signers_amount = utxo_out.value.to_sat().saturating_sub(deposit_fees);
            utxo_out.value = Amount::from_sat(signers_amount);
        }
        // We now update the remaining withdrawal amounts to account for fees.
        tx.output.iter_mut().skip(1).for_each(|tx_out| {
            tx_out.value = Amount::from_sat(tx_out.value.to_sat().saturating_sub(fee));
        });
    }

    /// Helper function for generating dummy Schnorr signatures.
    fn generate_dummy_signature() -> Signature {
        let key_pair = Keypair::new_global(&mut rand::rngs::OsRng);

        Signature {
            signature: key_pair.sign_schnorr(Message::from_digest([0; 32])),
            sighash_type: bitcoin::TapSighashType::Default,
        }
    }

    /// We originally populated the witness with dummy data to get an
    /// accurate estimate of the "virtual size" of the transaction. This
    /// function resets the witness data to be empty.
    fn reset_witness_data(tx: &mut Transaction) {
        tx.input
            .iter_mut()
            .for_each(|tx_in| tx_in.witness = Witness::new());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use super::*;
    use bitcoin::blockdata::opcodes;
    use bitcoin::CompressedPublicKey;
    use bitcoin::KnownHrp;
    use bitcoin::PublicKey;
    use bitcoin::Txid;
    use rand::distributions::Distribution;
    use secp256k1::SecretKey;
    use test_case::test_case;

    const PUBLIC_KEY1: &'static str =
        "032e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af";

    const XONLY_PUBLIC_KEY0: &'static str =
        "ff12471208c14bd580709cb2358d98975247d8765f92bc25eab3b2763ed605f8";

    const XONLY_PUBLIC_KEY1: &'static str =
        "2e58afe51f9ed8ad3cc7897f634d881fdbe49a81564629ded8156bebd2ffd1af";

    fn generate_x_only_public_key() -> XOnlyPublicKey {
        let secret_key = SecretKey::new(&mut rand::rngs::OsRng);
        secret_key.x_only_public_key(SECP256K1).0
    }

    fn generate_address() -> Address {
        let secret_key = SecretKey::new(&mut rand::rngs::OsRng);
        let pk = CompressedPublicKey(secret_key.public_key(SECP256K1));

        Address::p2wpkh(&pk, KnownHrp::Regtest)
    }

    fn generate_outpoint(amount: u64, vout: u32) -> OutPoint {
        let mut rng = rand::rngs::OsRng;
        let sats: u64 = rand::distributions::Uniform::new(1, 500_000_000).sample(&mut rng);

        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![
                TxOut {
                    value: Amount::from_sat(sats),
                    script_pubkey: ScriptBuf::new(),
                },
                TxOut {
                    value: Amount::from_sat(amount),
                    script_pubkey: ScriptBuf::new(),
                },
            ],
        };

        OutPoint { txid: tx.compute_txid(), vout }
    }

    /// Create a new deposit request depositing from a random public key.
    fn create_deposit(amount: u64, max_fee: u64, votes_against: usize) -> DepositRequest {
        let public_key = PublicKey::from_str(PUBLIC_KEY1).unwrap();
        DepositRequest {
            outpoint: generate_outpoint(amount, 1),
            max_fee,
            signer_bitmap: std::iter::repeat(false).take(votes_against).collect(),
            amount,
            deposit_script: ScriptBuf::builder()
                .push_slice([1, 2, 3, 4])
                .push_opcode(opcodes::all::OP_DROP)
                .push_opcode(opcodes::all::OP_DUP)
                .push_opcode(opcodes::all::OP_HASH160)
                .push_slice(public_key.pubkey_hash())
                .push_opcode(opcodes::all::OP_EQUALVERIFY)
                .push_opcode(opcodes::all::OP_CHECKSIG)
                .into_script(),
            redeem_script: ScriptBuf::new(),
            taproot_public_key: XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY0).unwrap(),
            signers_public_key: generate_x_only_public_key(),
        }
    }

    /// Create a new withdrawal request withdrawing to a random address.
    fn create_withdrawal(amount: u64, max_fee: u64, votes_against: usize) -> WithdrawalRequest {
        WithdrawalRequest {
            max_fee,
            signer_bitmap: std::iter::repeat(false).take(votes_against).collect(),
            amount,
            address: generate_address(),
        }
    }

    #[test_case(&[false, false, true, false, true, true, true], 3; "case 1")]
    #[test_case(&[false, false, true, true, true, true, true], 2; "case 2")]
    #[test_case(&[true, true, true, true, true, true, true], 0; "case 3")]
    fn test_deposit_votes_against(signer_bitmap: &[bool], expected: u32) {
        let deposit = DepositRequest {
            outpoint: OutPoint::null(),
            max_fee: 0,
            signer_bitmap: signer_bitmap.to_vec(),
            amount: 100_000,
            deposit_script: ScriptBuf::new(),
            redeem_script: ScriptBuf::new(),
            taproot_public_key: XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap(),
            signers_public_key: XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap(),
        };

        assert_eq!(deposit.votes_against(), expected);
    }

    /// Some functions call functions that "could" panic. Check that they
    /// don't.
    #[test]
    fn deposit_witness_data_no_error() {
        let deposit = DepositRequest {
            outpoint: OutPoint::null(),
            max_fee: 0,
            signer_bitmap: Vec::new(),
            amount: 100_000,
            deposit_script: ScriptBuf::from_bytes(vec![1, 2, 3]),
            redeem_script: ScriptBuf::new(),
            taproot_public_key: XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap(),
            signers_public_key: XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap(),
        };

        let sig = Signature::from_slice(&[0u8; 64]).unwrap();
        let witness = deposit.construct_witness_data(sig);
        assert!(witness.tapscript().is_some());

        let sig = UnsignedTransaction::generate_dummy_signature();
        let tx_in = deposit.as_tx_input(sig);

        // The deposits are taproot spend and do not have a script. The
        // actual spend script and input data gets put in the witness data
        assert!(tx_in.script_sig.is_empty());
    }

    /// The first input and output are related to the signers' UTXO.
    #[test]
    fn the_first_input_and_output_is_signers() {
        let requests = SbtcRequests {
            deposits: vec![create_deposit(123456, 0, 0)],
            withdrawals: vec![create_withdrawal(1000, 0, 0), create_withdrawal(2000, 0, 0)],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: generate_outpoint(5500, 0),
                    amount: 5500,
                    public_key: generate_x_only_public_key(),
                },
                fee_rate: 0,
                public_key: generate_x_only_public_key(),
            },
            num_signers: 10,
            accept_threshold: 0,
        };

        // This should all be in one transaction since there are no votes
        // against any of the requests.
        let mut transactions = requests.construct_transactions().unwrap();
        assert_eq!(transactions.len(), 1);

        let unsigned_tx = transactions.pop().unwrap();
        assert_eq!(unsigned_tx.tx.input.len(), 2);

        // Let's make sure the first input references the UTXO from the
        // signer_state variable.
        let signers_utxo_input = unsigned_tx.tx.input.first().unwrap();
        let old_outpoint = requests.signer_state.utxo.outpoint;
        assert_eq!(signers_utxo_input.previous_output.txid, old_outpoint.txid);
        assert_eq!(signers_utxo_input.previous_output.vout, old_outpoint.vout);

        // We had two withdrawal requests so there should be 1 + 2 outputs
        assert_eq!(unsigned_tx.tx.output.len(), 3);

        // The signers' UTXO, the first one, contains the balance of all
        // deposits and withdrawals. It's also a P2TR script.
        let signers_utxo_output = unsigned_tx.tx.output.first().unwrap();
        assert_eq!(
            signers_utxo_output.value.to_sat(),
            5500 + 123456 - 1000 - 2000
        );
        assert!(signers_utxo_output.script_pubkey.is_p2tr());

        // All the other UTXOs are P2WPKH outputs.
        unsigned_tx.tx.output.iter().skip(1).for_each(|output| {
            assert!(output.script_pubkey.is_p2wpkh());
        });

        // The new UTXO should be using the signer public key from the
        // signer state.
        let new_utxo = unsigned_tx.new_signer_utxo();
        assert_eq!(new_utxo.public_key, requests.signer_state.public_key);
    }

    /// Deposit requests add to the signers' UTXO.
    #[test]
    fn deposits_increase_signers_utxo_amount() {
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: vec![
                create_deposit(123456, 0, 0),
                create_deposit(789012, 0, 0),
                create_deposit(345678, 0, 0),
            ],
            withdrawals: Vec::new(),
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: OutPoint::null(),
                    amount: 55,
                    public_key,
                },
                fee_rate: 0,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 0,
        };

        // This should all be in one transaction since there are no votes
        // against any of the requests.
        let mut transactions = requests.construct_transactions().unwrap();
        assert_eq!(transactions.len(), 1);

        // The transaction should have one output corresponding to the
        // signers' UTXO
        let unsigned_tx = transactions.pop().unwrap();
        assert_eq!(unsigned_tx.tx.output.len(), 1);

        // The new amount should be the sum of the old amount plus the deposits.
        let new_amount: u64 = unsigned_tx
            .tx
            .output
            .iter()
            .map(|out| out.value.to_sat())
            .sum();
        assert_eq!(new_amount, 55 + 123456 + 789012 + 345678)
    }

    /// Withdrawal requests remove funds from the signers' UTXO.
    #[test]
    fn withdrawals_decrease_signers_utxo_amount() {
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: Vec::new(),
            withdrawals: vec![
                create_withdrawal(1000, 0, 0),
                create_withdrawal(2000, 0, 0),
                create_withdrawal(3000, 0, 0),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: OutPoint::null(),
                    amount: 9500,
                    public_key,
                },
                fee_rate: 0,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 0,
        };

        let mut transactions = requests.construct_transactions().unwrap();
        assert_eq!(transactions.len(), 1);

        let unsigned_tx = transactions.pop().unwrap();
        assert_eq!(unsigned_tx.tx.output.len(), 4);

        let signer_utxo = unsigned_tx.tx.output.first().unwrap();
        assert_eq!(signer_utxo.value.to_sat(), 9500 - 1000 - 2000 - 3000);
    }

    /// We chain transactions so that we have a single signer UTXO at the end.
    #[test]
    fn returned_txs_form_a_tx_chain() {
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: vec![
                create_deposit(1234, 0, 1),
                create_deposit(5678, 0, 1),
                create_deposit(9012, 0, 2),
            ],
            withdrawals: vec![
                create_withdrawal(1000, 0, 1),
                create_withdrawal(2000, 0, 1),
                create_withdrawal(3000, 0, 1),
                create_withdrawal(4000, 0, 2),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: generate_outpoint(300_000, 0),
                    amount: 300_000,
                    public_key,
                },
                fee_rate: 0,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 8,
        };

        let transactions = requests.construct_transactions().unwrap();
        more_asserts::assert_gt!(transactions.len(), 1);

        transactions.windows(2).for_each(|unsigned| {
            let utx0 = &unsigned[0];
            let utx1 = &unsigned[1];

            let previous_output1 = utx1.tx.input[0].previous_output;
            assert_eq!(utx0.tx.compute_txid(), previous_output1.txid);
            assert_eq!(previous_output1.vout, 0);
        })
    }

    /// Check that each deposit and withdrawal is included as an input or
    /// deposit in the transaction package.
    #[test]
    fn requests_in_unsigned_transaction_are_in_btc_tx() {
        // The requests in the UnsignedTransaction correspond to
        // inputs and outputs in the transaction
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: vec![
                create_deposit(1234, 0, 1),
                create_deposit(5678, 0, 1),
                create_deposit(9012, 0, 2),
                create_deposit(3456, 0, 1),
                create_deposit(7890, 0, 0),
            ],
            withdrawals: vec![
                create_withdrawal(1000, 0, 1),
                create_withdrawal(2000, 0, 1),
                create_withdrawal(3000, 0, 1),
                create_withdrawal(4000, 0, 2),
                create_withdrawal(5000, 0, 0),
                create_withdrawal(6000, 0, 0),
                create_withdrawal(7000, 0, 0),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: generate_outpoint(300_000, 0),
                    amount: 300_000,
                    public_key,
                },
                fee_rate: 0,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 8,
        };

        let transactions = requests.construct_transactions().unwrap();
        more_asserts::assert_gt!(transactions.len(), 1);

        // Create collections of identifiers for each deposit and withdrawal
        // request.
        let mut input_txs: BTreeSet<Txid> =
            requests.deposits.iter().map(|x| x.outpoint.txid).collect();
        let mut output_scripts: BTreeSet<String> = requests
            .withdrawals
            .iter()
            .map(|req| req.address.script_pubkey().to_hex_string())
            .collect();

        // Now we check that the counts of the withdrawals and deposits
        // line up.
        transactions.iter().for_each(|utx| {
            let num_inputs = utx.tx.input.len();
            let num_outputs = utx.tx.output.len();
            assert_eq!(utx.requests.len() + 2, num_inputs + num_outputs);

            let num_deposits = utx.requests.iter().filter_map(|x| x.as_deposit()).count();
            assert_eq!(utx.tx.input.len(), num_deposits + 1);

            let num_withdrawals = utx
                .requests
                .iter()
                .filter_map(|x| x.as_withdrawal())
                .count();
            assert_eq!(utx.tx.output.len(), num_withdrawals + 1);

            // Check that each deposit is referenced exactly once
            // We ship the first one since that is the signers' UTXO
            for tx_in in utx.tx.input.iter().skip(1) {
                assert!(input_txs.remove(&tx_in.previous_output.txid));
            }
            for tx_out in utx.tx.output.iter().skip(1) {
                assert!(output_scripts.remove(&tx_out.script_pubkey.to_hex_string()));
            }
        });

        assert!(input_txs.is_empty());
        assert!(output_scripts.is_empty());
    }

    /// Check the following:
    /// * The fees for each transaction is at least as large as the fee_rate
    ///   in the signers' state.
    /// * Each deposit and withdrawal request pays the same fee.
    /// * The total fees are equal to the number of request times the fee per
    ///   request amount.
    /// * Deposit requests pay fees too, but implicitly by the amounts
    ///   deducted from the signers.
    #[test]
    fn returned_txs_match_fee_rate() {
        // Each deposit and withdrawal has a max fee greater than the current market fee rate
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: vec![
                create_deposit(12340, 100_000, 1),
                create_deposit(56780, 100_000, 1),
                create_deposit(90120, 100_000, 2),
                create_deposit(34560, 100_000, 1),
                create_deposit(78900, 100_000, 0),
            ],
            withdrawals: vec![
                create_withdrawal(10000, 100_000, 1),
                create_withdrawal(20000, 100_000, 1),
                create_withdrawal(30000, 100_000, 1),
                create_withdrawal(40000, 100_000, 2),
                create_withdrawal(50000, 100_000, 0),
                create_withdrawal(60000, 100_000, 0),
                create_withdrawal(70000, 100_000, 0),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: generate_outpoint(300_000, 0),
                    amount: 300_000_000,
                    public_key,
                },
                fee_rate: 25,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 8,
        };

        // It's tough to match the outputs to the original request. We do
        // that here by matching the expected scripts, which are unique for
        // each public key. Since each public key is unique, this works.
        let mut withdrawal_amounts: BTreeMap<String, u64> = requests
            .withdrawals
            .iter()
            .map(|req| (req.address.script_pubkey().to_hex_string(), req.amount))
            .collect();

        let transactions = requests.construct_transactions().unwrap();
        more_asserts::assert_gt!(transactions.len(), 1);

        transactions
            .iter()
            .fold(requests.signer_state.utxo.amount, |signer_amount, utx| {
                for output in utx.tx.output.iter().skip(1) {
                    let original_amount = withdrawal_amounts
                        .remove(&output.script_pubkey.to_hex_string())
                        .unwrap();
                    assert_eq!(original_amount, output.value.to_sat() + utx.fee_per_request);
                }

                let output_amounts: u64 = utx.tx.output.iter().map(|out| out.value.to_sat()).sum();
                let input_amounts: u64 = utx
                    .requests
                    .iter()
                    .filter_map(Request::as_deposit)
                    .map(|dep| dep.amount)
                    .chain([signer_amount])
                    .sum();

                more_asserts::assert_gt!(input_amounts, output_amounts);
                more_asserts::assert_gt!(utx.requests.len(), 0);

                // Since there are often both deposits and withdrawal, the
                // following assertion checks that we capture the fees that
                // depositors must pay.
                let total_fees = utx.fee_per_request * utx.requests.len() as u64;
                assert_eq!(input_amounts, output_amounts + total_fees);

                let state = &requests.signer_state;
                let signed_vsize = UnsignedTransaction::new_transaction(&utx.requests, state)
                    .unwrap()
                    .vsize();

                // The unsigned transaction has all witness data removed,
                // so it should have a much smaller size than the "signed"
                // version returned from UnsignedTransaction::new_transaction.
                more_asserts::assert_lt!(utx.tx.vsize(), signed_vsize);
                // The final fee rate should still be greater than the market fee rate
                let fee_rate = (input_amounts - output_amounts) as f64 / signed_vsize as f64;
                more_asserts::assert_le!(requests.signer_state.fee_rate as f64, fee_rate);

                utx.new_signer_utxo().amount
            });
    }

    #[test_case(2; "Some deposits")]
    #[test_case(0; "No deposits")]
    fn unsigned_tx_digests(num_deposits: usize) {
        // Each deposit and withdrawal has a max fee greater than the current market fee rate
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: std::iter::repeat_with(|| create_deposit(123456, 100_000, 0))
                .take(num_deposits)
                .collect(),
            withdrawals: vec![
                create_withdrawal(10000, 100_000, 0),
                create_withdrawal(20000, 100_000, 0),
                create_withdrawal(30000, 100_000, 0),
                create_withdrawal(40000, 100_000, 0),
                create_withdrawal(50000, 100_000, 0),
                create_withdrawal(60000, 100_000, 0),
                create_withdrawal(70000, 100_000, 0),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: generate_outpoint(300_000, 0),
                    amount: 300_000_000,
                    public_key,
                },
                fee_rate: 25,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 8,
        };
        let mut transactions = requests.construct_transactions().unwrap();
        assert_eq!(transactions.len(), 1);

        let unsigned = transactions.pop().unwrap();
        let sighashes = unsigned.construct_digests().unwrap();

        assert_eq!(sighashes.deposits.len(), num_deposits)
    }

    /// If the signer's UTXO does not have enough to cover the requests
    /// then we return an error.
    #[test]
    fn negative_amounts_give_error() {
        let public_key = XOnlyPublicKey::from_str(XONLY_PUBLIC_KEY1).unwrap();
        let requests = SbtcRequests {
            deposits: Vec::new(),
            withdrawals: vec![
                create_withdrawal(1000, 0, 0),
                create_withdrawal(2000, 0, 0),
                create_withdrawal(3000, 0, 0),
            ],
            signer_state: SignerBtcState {
                utxo: SignerUtxo {
                    outpoint: OutPoint::null(),
                    amount: 3000,
                    public_key,
                },
                fee_rate: 0,
                public_key,
            },
            num_signers: 10,
            accept_threshold: 0,
        };

        let transactions = requests.construct_transactions();
        assert!(transactions.is_err());
    }
}