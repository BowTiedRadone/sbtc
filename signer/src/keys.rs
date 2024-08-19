//! This module contains the `PublicKey` and `PrivateKey` types used
//! throughout this crate. These types allow for easy conversion between
//! the various crypto libraries and crates used here: rust-secp256k1,
//! stacks-common, and p256k1. All three crates depend on the libsecp256k1
//! C library under the hood.
//!
//! ## PublicKey conversions to-from p256k1 types.
//!
//! Every `PublicKey` is a valid `p256k1::point::Point` because a
//! `PublicKey` is a point on the secp256k1 curve. But the
//! `p256k1::point::Point` type can represent any point on the curve,
//! including the identity point (also called the point at infinity), but
//! that point is not a valid public key.
//!
//! ## PrivateKey conversions to-from p256k1 types
//!
//! Every `PrivateKey` is a valid `p256k1::scalar::Scalar`, but not the
//! other way around. This is because zero is an invalid `PrivateKey` but
//! it is a valid `p256k1::scalar::Scalar`. The `secp256k1::SecretKey` type
//! that `PrivateKey` wraps, uses `secp256k1_ec_seckey_verify` (from the
//! libsecp256k1 C library) under the hood[1] and that function rejects
//! zero as a valid secret key[2]. The `p256k1::scalar::Scalar` type just
//! checks that the underlying number is less than the order of the
//! secp256k1 curve[3][4], which `secp256k1::SecretKey` also does.
//!
//! [^1]: https://github.com/rust-bitcoin/rust-secp256k1/blob/789f3844c7613584b4ee223e06c730019118b3a0/src/key.rs#L215-L231
//! [^2]: https://github.com/bitcoin-core/secp256k1/blob/3fdf146bad042a17f6b2f490ef8bd9d8e774cdbd/include/secp256k1.h#L682-L697
//! [^3]: https://github.com/Trust-Machines/p256k1/blob/3ecb941c1af13741d52335ef911693b6d6fda94b/p256k1/src/scalar.rs#L245-L257
//! [^4]: https://github.com/bitcoin-core/secp256k1/blob/3fdf146bad042a17f6b2f490ef8bd9d8e774cdbd/src/scalar.h#L31-L36

use bitcoin::ScriptBuf;
use bitcoin::TapTweakHash;
use secp256k1::Parity;
use secp256k1::SECP256K1;
use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;

/// The public key type for the secp256k1 elliptic curve.
#[derive(Copy, Clone, Debug, PartialOrd, Ord, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey(secp256k1::PublicKey);

impl From<&secp256k1::PublicKey> for PublicKey {
    fn from(value: &secp256k1::PublicKey) -> Self {
        Self(*value)
    }
}

impl From<&PublicKey> for secp256k1::PublicKey {
    fn from(value: &PublicKey) -> Self {
        value.0
    }
}

impl From<secp256k1::PublicKey> for PublicKey {
    fn from(value: secp256k1::PublicKey) -> Self {
        Self(value)
    }
}

impl From<PublicKey> for secp256k1::PublicKey {
    fn from(value: PublicKey) -> Self {
        value.0
    }
}

impl From<&PublicKey> for secp256k1::XOnlyPublicKey {
    fn from(value: &PublicKey) -> Self {
        value.0.x_only_public_key().0
    }
}

impl From<&PublicKey> for p256k1::point::Point {
    /// This implementation takes the full 65 byte serialization of the
    /// public key and breaks it into it's x-coordinate and y-coordinate
    /// parts, and then maps those coordinates into a Point.
    ///
    /// # Notes
    ///
    /// An uncompressed serialization of the secp256k1::PublicKey type is
    /// 65 bytes. The first byte denotes that the following slice is an
    /// uncompressed public key on the secp256k1 curve, the next 32 bytes
    /// are for the x-coordinate, and the remaining 32-bytes are for the
    /// y-coordinate.
    fn from(value: &PublicKey) -> Self {
        // We start by serializing the full key into it's x- and
        // y-coordinates.
        let full_key: [u8; 65] = value.0.serialize_uncompressed();

        // Let's copy over the various slices. The copy cannot panic
        // because we know that the lengths of each of the slices match.
        let mut x_part = [0; 32];
        let mut y_part = [0; 32];
        x_part.copy_from_slice(&full_key[1..33]);
        y_part.copy_from_slice(&full_key[33..]);

        // Okay, now for conversion to the p256k1 types. Under the hood
        // here `p256k1::field::Element::from` tries to reduce the input to
        // the order of the secp256k1 curve[1][2], but we do not need worry
        // since we have a valid point on the curve.
        //
        // [^1]: https://github.com/Trust-Machines/p256k1/blob/3ecb941c1af13741d52335ef911693b6d6fda94b/p256k1/src/field.rs#L268-L279
        // [^2]: https://github.com/bitcoin-core/secp256k1/blob/v0.3.0/src/field.h#L78-L79
        let x_element = p256k1::field::Element::from(x_part);
        let y_element = p256k1::field::Element::from(y_part);
        // You would think that you couldn't always convert two elements
        // into a Point, but `p256k1::point::Point::from` uses
        // `secp256k1_gej_set_ge` under the hood, which I believe does any
        // reduction. But still, we have a valid public key, so no
        // reductions should be necessary.
        p256k1::point::Point::from((x_element, y_element))
    }
}

impl From<PublicKey> for p256k1::point::Point {
    fn from(value: PublicKey) -> Self {
        Self::from(&value)
    }
}

/// This should only error when the `p256k1::point::Point` is the identity
/// point (also called the at infinity).
impl TryFrom<&p256k1::point::Point> for PublicKey {
    type Error = Error;
    fn try_from(value: &p256k1::point::Point) -> Result<Self, Self::Error> {
        let x_data = value.x().to_bytes();

        let pk = secp256k1::XOnlyPublicKey::from_slice(&x_data).map_err(Error::InvalidPublicKey)?;
        let parity = if value.has_even_y() {
            Parity::Even
        } else {
            Parity::Odd
        };

        let public_key = secp256k1::PublicKey::from_x_only_public_key(pk, parity);
        Ok(Self(public_key))
    }
}

impl From<&PublicKey> for p256k1::keys::PublicKey {
    fn from(value: &PublicKey) -> Self {
        p256k1::keys::PublicKey::try_from(value.0.serialize().as_slice())
            .expect("BUG: rust-secp265k1 public keys should map to p256k1 public keys")
    }
}

impl From<&p256k1::keys::PublicKey> for PublicKey {
    fn from(value: &p256k1::keys::PublicKey) -> Self {
        secp256k1::PublicKey::from_slice(&value.to_bytes())
            .map(Self)
            .expect("BUG: p256k1 public keys should map to rust-secp265k1 public keys")
    }
}

impl From<PublicKey> for p256k1::keys::PublicKey {
    fn from(value: PublicKey) -> Self {
        Self::from(&value)
    }
}

impl From<p256k1::keys::PublicKey> for PublicKey {
    fn from(value: p256k1::keys::PublicKey) -> Self {
        Self::from(&value)
    }
}

/// Under the hood stacks-common wraps the rust-secp256k1 types, so these
/// implementations should always map correctly.
impl From<&PublicKey> for stacks_common::util::secp256k1::Secp256k1PublicKey {
    fn from(value: &PublicKey) -> Self {
        Self::from_slice(&value.0.serialize())
            .expect("BUG: rust-secp256k1 types should map to their stacks secp256k1 type")
    }
}

impl From<&stacks_common::util::secp256k1::Secp256k1PublicKey> for PublicKey {
    fn from(value: &stacks_common::util::secp256k1::Secp256k1PublicKey) -> Self {
        let key = secp256k1::PublicKey::from_slice(&value.to_bytes_compressed())
            .expect("BUG: stacks secp256k1 type should map to the rust-secp256k1 types");
        Self(key)
    }
}

impl PublicKey {
    /// Creates a public key directly from a slice.
    pub fn from_slice(data: &[u8]) -> Result<Self, Error> {
        secp256k1::PublicKey::from_slice(data)
            .map(Self)
            .map_err(Error::InvalidPublicKey)
    }

    /// Serializes the key as a byte-encoded pair of values in compressed
    /// form. In compressed form the y-coordinate is represented by only a
    /// single bit, as x determines it up to one bit.
    pub fn serialize(&self) -> [u8; 33] {
        self.0.serialize()
    }

    /// Creates a new public key from a [`Private`] and the global
    /// [`SECP256K1`] context.
    pub fn from_private_key(key: &PrivateKey) -> Self {
        Self(secp256k1::PublicKey::from_secret_key_global(&key.0))
    }

    /// Combine many keys into one aggregate key
    pub fn combine_keys<'a, I>(keys: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = &'a PublicKey>,
    {
        let keys: Vec<&secp256k1::PublicKey> = keys.into_iter().map(|key| &key.0).collect();
        secp256k1::PublicKey::combine_keys(&keys)
            .map(Self)
            .map_err(Error::InvalidAggregateKey)
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// We expect the compressed public key bytes from the database
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for PublicKey {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let bytes = <[u8; 33] as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(PublicKey::from_slice(&bytes)?)
    }
}

impl sqlx::Type<sqlx::Postgres> for PublicKey {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <[u8; 33] as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

/// We write the compressed public key bytes to the database
impl<'r> sqlx::Encode<'r, sqlx::Postgres> for PublicKey {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> sqlx::encode::IsNull {
        let bytes = self.serialize();
        <[u8; 33] as sqlx::Encode<'r, sqlx::Postgres>>::encode_by_ref(&bytes, buf)
    }
}

impl sqlx::postgres::PgHasArrayType for PublicKey {
    fn array_type_info() -> sqlx::postgres::PgTypeInfo {
        <[u8; 33] as sqlx::postgres::PgHasArrayType>::array_type_info()
    }
}

#[cfg(feature = "testing")]
impl fake::Dummy<fake::Faker> for PublicKey {
    fn dummy_with_rng<R: rand::Rng + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        Self(secp256k1::PublicKey::from_secret_key_global(
            &secp256k1::SecretKey::new(rng),
        ))
    }
}

/// A private key type for the secp256k1 elliptic curve.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrivateKey(secp256k1::SecretKey);

impl From<secp256k1::SecretKey> for PrivateKey {
    fn from(value: secp256k1::SecretKey) -> Self {
        Self(value)
    }
}

impl From<PrivateKey> for secp256k1::SecretKey {
    fn from(value: PrivateKey) -> Self {
        value.0
    }
}

/// This should only error when the `p256k1::scalar::Scalar` is zero.
impl TryFrom<&p256k1::scalar::Scalar> for PrivateKey {
    type Error = Error;
    fn try_from(value: &p256k1::scalar::Scalar) -> Result<Self, Self::Error> {
        secp256k1::SecretKey::from_slice(&value.to_bytes())
            .map(Self)
            .map_err(Error::InvalidPrivateKey)
    }
}

impl From<&PrivateKey> for p256k1::scalar::Scalar {
    fn from(value: &PrivateKey) -> Self {
        p256k1::scalar::Scalar::from(value.0.secret_bytes())
    }
}

impl From<PrivateKey> for p256k1::scalar::Scalar {
    fn from(value: PrivateKey) -> Self {
        Self::from(&value)
    }
}

impl PrivateKey {
    /// Create a new one
    pub fn new<R: rand::Rng + ?Sized>(rng: &mut R) -> Self {
        Self(secp256k1::SecretKey::new(rng))
    }
    /// Creates a private key directly from a slice.
    pub fn from_slice(data: &[u8]) -> Result<Self, Error> {
        secp256k1::SecretKey::from_slice(data)
            .map(Self)
            .map_err(Error::InvalidPrivateKey)
    }

    /// Returns the secret key as a byte array.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.secret_bytes()
    }

    /// Constructs an ECDSA signature for `message` using the global
    /// [`SECP256K1`] context and returns it in "low S" form.
    pub fn sign_ecdsa(&self, msg: &secp256k1::Message) -> secp256k1::ecdsa::Signature {
        let mut sig = SECP256K1.sign_ecdsa(msg, &self.0);
        sig.normalize_s();
        sig
    }

    /// Constructs a recoverable ECDSA signature for `message` using the
    /// global [`SECP256K1`] context.
    pub fn sign_ecdsa_recoverable(
        &self,
        msg: &secp256k1::Message,
    ) -> secp256k1::ecdsa::RecoverableSignature {
        SECP256K1.sign_ecdsa_recoverable(msg, &self.0)
    }
}

/// This trait is used to provide a unifying interface for converting
/// different public key types to the `scriptPubKey` and the tweaked public
/// key associated with the signers. We represent the `scriptPubkey` using
/// rust-bitcoin's ScriptBuf type.
pub trait SignerScriptPubKey: Sized {
    /// Convert this type to the `scriptPubkey` used by the signers to lock
    /// their UTXO.
    fn signers_script_pubkey(&self) -> ScriptBuf;
    /// Construct the signers tweaked public key.
    fn signers_tweaked_pubkey(&self) -> Result<PublicKey, Error>;
}

impl SignerScriptPubKey for PublicKey {
    fn signers_script_pubkey(&self) -> ScriptBuf {
        secp256k1::XOnlyPublicKey::from(self).signers_script_pubkey()
    }
    /// Construct the signers tweaked public key.
    ///
    /// The implementation below is the same as the first step in the
    /// [`ScriptBuf::new_p2tr`] implementation, which we know does what we
    /// want.
    fn signers_tweaked_pubkey(&self) -> Result<PublicKey, Error> {
        let internal_key = secp256k1::XOnlyPublicKey::from(self);
        let tweak = TapTweakHash::from_key_and_tweak(internal_key, None).to_scalar();
        self.0
            .add_exp_tweak(SECP256K1, &tweak)
            .map(PublicKey)
            .map_err(Error::InvalidPublicKeyTweak)
    }
}

impl SignerScriptPubKey for secp256k1::XOnlyPublicKey {
    fn signers_script_pubkey(&self) -> ScriptBuf {
        ScriptBuf::new_p2tr(SECP256K1, *self, None)
    }
    /// The [`secp256k1::XOnlyPublicKey`] type has a tap_tweak public
    /// function that panics when adding the tweak leads to an invalid
    /// public key. Although it is extremely unlikely for the resulting
    /// public key to be invalid by chance, we still bubble this one up.
    fn signers_tweaked_pubkey(&self) -> Result<PublicKey, Error> {
        let tweak = TapTweakHash::from_key_and_tweak(*self, None).to_scalar();
        let (output_key, parity) = self
            .add_tweak(SECP256K1, &tweak)
            .map_err(Error::InvalidPublicKeyTweak)?;

        debug_assert!(self.tweak_add_check(SECP256K1, &output_key, parity, tweak));
        let pk = secp256k1::PublicKey::from_x_only_public_key(output_key, parity);
        Ok(PublicKey(pk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::rngs::OsRng;
    use secp256k1::SecretKey;
    use stacks_common::util::secp256k1::Secp256k1PrivateKey;
    use stacks_common::util::secp256k1::Secp256k1PublicKey;

    use test_case::test_case;

    struct Key<T>(T);

    impl Key<p256k1::keys::PublicKey> {
        fn new() -> Self {
            // Under the hood this uses a rand::thread_rng() for randomness.
            let private_key = Secp256k1PrivateKey::new();
            let pub_key = Secp256k1PublicKey::from_private(&private_key);
            let bytes = pub_key.to_bytes_compressed();
            Key(p256k1::keys::PublicKey::try_from(bytes.as_slice()).unwrap())
        }
    }

    impl Key<Secp256k1PublicKey> {
        fn new() -> Self {
            // Under the hood this uses a rand::thread_rng() for randomness.
            let private_key = Secp256k1PrivateKey::new();
            Key(Secp256k1PublicKey::from_private(&private_key))
        }
    }

    impl Key<secp256k1::PublicKey> {
        fn new<R: rand::Rng + ?Sized>(rng: &mut R) -> Self {
            let sk = SecretKey::new(rng);
            Key(secp256k1::PublicKey::from_secret_key_global(&sk))
        }
    }

    #[test]
    fn zero_valid_scalar_invalid_private_key() {
        let bytes = [0; 32];
        let scalar = p256k1::scalar::Scalar::from(bytes);
        assert!(PrivateKey::try_from(&scalar).is_err());
    }

    #[test]
    fn zero_x_valid_point_invalid_public_key() {
        let bytes = [0; 32];
        let scalar = p256k1::scalar::Scalar::from(bytes);
        let any_y = p256k1::scalar::Scalar::random(&mut OsRng);
        let point = p256k1::point::Point::from((scalar, any_y));
        assert!(PublicKey::try_from(&point).is_err());

        // This should map to the identity point (the point at infinity),
        // which is an invalid public key.
        let point = p256k1::point::Point::from(scalar);
        assert!(PublicKey::try_from(&point).is_err());
    }

    #[test]
    fn usually_scalar_invalid_p256k1_public_key() {
        let bytes = [0; 32];
        let scalar = p256k1::scalar::Scalar::from(bytes);
        assert!(p256k1::keys::PublicKey::new(&scalar).is_err());
    }

    #[test]
    fn usually_scalar_invalid_private_key() {
        let bytes = [0; 32];
        let scalar = p256k1::scalar::Scalar::from(bytes);
        assert!(PrivateKey::try_from(&scalar).is_err());
    }

    #[test_case(Key::<secp256k1::PublicKey>::new(&mut OsRng); "from a rust-secp256k1 PublicKey")]
    #[test_case(Key::<Secp256k1PublicKey>::new(); "from a stacks-common Secp256k1PublicKey")]
    #[test_case(Key::<p256k1::keys::PublicKey>::new(); "from a p256k1 PublicKey")]
    fn public_key_conversions_is_isomorphism<T>(source_key: Key<T>)
    where
        T: for<'a> From<&'a PublicKey> + PartialEq + std::fmt::Debug,
        PublicKey: for<'a> From<&'a T>,
    {
        let pubkey = PublicKey::from(&source_key.0);
        let invert_pubkey = T::from(&pubkey);

        assert_eq!(invert_pubkey, source_key.0);
        assert_eq!(PublicKey::from(&invert_pubkey), pubkey);
    }

    #[test]
    fn stacks_common_public_key_compressed() {
        let public_key = PublicKey::from_private_key(&PrivateKey::new(&mut OsRng));
        let key = stacks_common::util::secp256k1::Secp256k1PublicKey::from(&public_key);
        assert!(key.compressed())
    }

    #[test]
    fn selective_conversion_private_key() {
        // We test that we can go from a scalar to a PrivateKey with high
        // probability, and we can go back 100% of the time.
        let scalar = p256k1::scalar::Scalar::random(&mut OsRng);
        if scalar.to_bytes() == [0u8; 32] {
            return;
        }

        let private_key = PrivateKey::try_from(&scalar).unwrap();
        let from_pk = p256k1::scalar::Scalar::from(&private_key);
        assert_eq!(from_pk, scalar);

        let pk = PrivateKey(SecretKey::new(&mut OsRng));
        let scalar = p256k1::scalar::Scalar::from(&pk);
        let from_scalar = PrivateKey::try_from(&scalar).unwrap();

        assert_eq!(pk, from_scalar);
    }

    // If we have a XOnlyPublicKey and a PublicKey that represent the same
    // x-coordinate on the curve, then the associated signer
    // `scriptPubKey`s must match.
    #[test]
    fn x_only_key_and_secp256k1_pubkey_same_script() {
        let secret_key = SecretKey::new(&mut OsRng);
        let x_part = secret_key.x_only_public_key(SECP256K1).0;
        // It doesn't matter what the parity bit is.
        let pk = secp256k1::PublicKey::from_x_only_public_key(x_part, Parity::Even);
        let public_key = PublicKey(pk);

        assert_eq!(
            public_key.signers_script_pubkey(),
            x_part.signers_script_pubkey()
        );
    }

    #[test]
    fn tap_tweak_equivalence() {
        let private_key = PrivateKey::new(&mut OsRng);
        let public_key = PublicKey::from_private_key(&private_key);
        let tweaked_aggregate_key1 = wsts::compute::tweaked_public_key(&public_key.into(), None);
        let tweaked_aggregate_key2 = public_key.signers_tweaked_pubkey().unwrap();

        let tweaked_aggregate_key1_bytes = tweaked_aggregate_key1.x().to_bytes();
        let tweaked_aggregate_key2_bytes =
            tweaked_aggregate_key2.0.x_only_public_key().0.serialize();
        assert_eq!(tweaked_aggregate_key1_bytes, tweaked_aggregate_key2_bytes);
    }
}