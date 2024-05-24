//! Test utilities for signer message

use bitcoin::hashes::Hash;
use fake::Fake;
use rand::seq::SliceRandom;

impl super::SignerMessage {
    /// Construct a random message
    pub fn random<R: rand::RngCore + ?Sized>(rng: &mut R) -> Self {
        fake::Faker.fake_with_rng(rng)
    }

    /// Construct a random message with the given payload type
    pub fn random_with_payload_type<
        P: Into<super::Payload> + fake::Dummy<fake::Faker>,
        R: rand::RngCore + ?Sized,
    >(
        rng: &mut R,
    ) -> Self {
        let payload = dummy_payload::<P, _>(&fake::Faker, rng);
        Self::random_with_payload(rng, payload)
    }

    /// Construct a random message with the given payload
    fn random_with_payload<R: rand::RngCore + ?Sized>(
        rng: &mut R,
        payload: super::Payload,
    ) -> Self {
        let mut block_hash_data = [0; 32];
        rng.fill_bytes(&mut block_hash_data);
        let block_hash = bitcoin::BlockHash::from_slice(&block_hash_data).unwrap();

        payload.to_message(block_hash)
    }
}

impl fake::Dummy<fake::Faker> for super::Payload {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        let variants = [
            dummy_payload::<super::SignerDepositDecision, _>,
            dummy_payload::<super::SignerWithdrawDecision, _>,
            dummy_payload::<super::WstsMessage, _>,
        ];

        variants.choose(rng).unwrap()(config, rng)
    }
}

impl fake::Dummy<fake::Faker> for super::SignerMessage {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        let payload: super::Payload = config.fake_with_rng(rng);

        Self::random_with_payload(rng, payload)
    }
}

impl fake::Dummy<fake::Faker> for super::SignerDepositDecision {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        Self {
            output_index: config.fake_with_rng(rng),
            txid: dummy_txid(config, rng),
            accepted: config.fake_with_rng(rng),
        }
    }
}

impl fake::Dummy<fake::Faker> for super::BitcoinTransactionSignRequest {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        Self { tx: dummy_tx(config, rng) }
    }
}

impl fake::Dummy<fake::Faker> for super::BitcoinTransactionSignAck {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        Self { txid: dummy_txid(config, rng) }
    }
}

impl fake::Dummy<fake::Faker> for super::WstsMessage {
    fn dummy_with_rng<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> Self {
        let dkg_end_begin = wsts::net::DkgEndBegin {
            dkg_id: config.fake_with_rng(rng),
            signer_ids: config.fake_with_rng(rng),
            key_ids: config.fake_with_rng(rng),
        };

        Self(wsts::net::Message::DkgEndBegin(dkg_end_begin))
    }
}

fn dummy_payload<P: Into<super::Payload> + fake::Dummy<fake::Faker>, R: rand::RngCore + ?Sized>(
    config: &fake::Faker,
    rng: &mut R,
) -> super::Payload {
    config.fake_with_rng::<P, _>(rng).into()
}

fn dummy_txid<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> bitcoin::Txid {
    let bytes: [u8; 32] = config.fake_with_rng(rng);
    bitcoin::Txid::from_byte_array(bytes)
}

fn dummy_tx<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> bitcoin::Transaction {
    let max_input_size = 50;
    let max_output_size = 50;

    let input_size = (rng.next_u32() % max_input_size) as usize;
    let output_size = (rng.next_u32() % max_output_size) as usize;

    let input = std::iter::repeat_with(|| dummy_txin(config, rng))
        .take(input_size)
        .collect();
    let output = std::iter::repeat_with(|| dummy_txout(config, rng))
        .take(output_size)
        .collect();

    bitcoin::Transaction {
        version: bitcoin::transaction::Version::ONE,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input,
        output,
    }
}

fn dummy_txin<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> bitcoin::TxIn {
    bitcoin::TxIn {
        previous_output: bitcoin::OutPoint::new(dummy_txid(config, rng), config.fake_with_rng(rng)),
        sequence: bitcoin::Sequence::ZERO,
        script_sig: bitcoin::ScriptBuf::new(),
        witness: bitcoin::witness::Witness::new(),
    }
}

fn dummy_txout<R: rand::RngCore + ?Sized>(config: &fake::Faker, rng: &mut R) -> bitcoin::TxOut {
    bitcoin::TxOut {
        value: bitcoin::Amount::from_sat(config.fake_with_rng(rng)),
        script_pubkey: bitcoin::ScriptBuf::new(),
    }
}