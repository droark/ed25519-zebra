const OID: ObjectIdentifier = ObjectIdentifier::new("1.3.101.112");  // RFC 8410
const ALGORITHM_ID: AlgorithmIdentifier = AlgorithmIdentifier {
        oid: OID,
        parameters: None,
    };

use std::convert::TryFrom;
use curve25519_dalek::{constants, scalar::Scalar};
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha512};
use pkcs8::{AlgorithmIdentifier, FromPrivateKey, ObjectIdentifier, PrivateKeyDocument, PrivateKeyInfo, ToPrivateKey};

#[cfg(any(feature = "pem", feature = "std"))]
use pkcs8::PrivateKeyDocument;

use crate::{Error, Signature, VerificationKey, VerificationKeyBytes};

/// An Ed25519 signing key.
///
/// This is also called a secret key by other implementations.
#[derive(Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(from = "SerdeHelper"))]
#[cfg_attr(feature = "serde", serde(into = "SerdeHelper"))]
pub struct SigningKey {
    seed: [u8; 32],
    s: Scalar,
    prefix: [u8; 32],
    vk: VerificationKey,
}

impl core::fmt::Debug for SigningKey {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        fmt.debug_struct("SigningKey")
            .field("seed", &hex::encode(&self.seed))
            .field("s", &self.s)
            .field("prefix", &hex::encode(&self.prefix))
            .field("vk", &self.vk)
            .finish()
    }
}

impl<'a> From<&'a SigningKey> for VerificationKey {
    fn from(sk: &'a SigningKey) -> VerificationKey {
        sk.vk
    }
}

impl<'a> From<&'a SigningKey> for VerificationKeyBytes {
    fn from(sk: &'a SigningKey) -> VerificationKeyBytes {
        sk.vk.into()
    }
}

impl AsRef<[u8]> for SigningKey {
    fn as_ref(&self) -> &[u8] {
        &self.seed[..]
    }
}

impl From<SigningKey> for [u8; 32] {
    fn from(sk: SigningKey) -> [u8; 32] {
        sk.seed
    }
}

impl TryFrom<&[u8]> for SigningKey {
    type Error = Error;
    fn try_from(slice: &[u8]) -> Result<SigningKey, Error> {
        if slice.len() == 32 {
            let mut bytes = [0u8; 32];
            bytes[..].copy_from_slice(slice);
            Ok(bytes.into())
        } else {
            Err(Error::InvalidSliceLength)
        }
    }
}

impl From<[u8; 32]> for SigningKey {
    #[allow(non_snake_case)]
    fn from(seed: [u8; 32]) -> SigningKey {
        // Expand the seed to a 64-byte array with SHA512.
        let h = Sha512::digest(&seed[..]);

        // Convert the low half to a scalar with Ed25519 "clamping"
        let s = {
            let mut scalar_bytes = [0u8; 32];
            scalar_bytes[..].copy_from_slice(&h.as_slice()[0..32]);
            scalar_bytes[0] &= 248;
            scalar_bytes[31] &= 127;
            scalar_bytes[31] |= 64;
            Scalar::from_bits(scalar_bytes)
        };

        // Extract and cache the high half.
        let prefix = {
            let mut prefix = [0u8; 32];
            prefix[..].copy_from_slice(&h.as_slice()[32..64]);
            prefix
        };

        // Compute the public key as A = [s]B.
        let A = &s * &constants::ED25519_BASEPOINT_TABLE;

        SigningKey {
            seed,
            s,
            prefix,
            vk: VerificationKey {
                minus_A: -A,
                A_bytes: VerificationKeyBytes(A.compress().to_bytes()),
            },
        }
    }
}

impl<'a> TryFrom<PrivateKeyInfo<'a>> for SigningKey {
    type Error = Error;
    fn try_from(pki: PrivateKeyInfo) -> Result<Self, Error> {
        if pki.algorithm == ALGORITHM_ID {
            SigningKey::try_from(pki.private_key)
        } else {
            Err(Error::MalformedSecretKey)
        }
    }
}

impl ToPrivateKey for SigningKey {
    fn to_pkcs8_der(&self) -> PrivateKeyDocument {
        // In RFC 8410, the octet string containing the private key is encapsulated by
        // another octet string. Just add octet string bytes to the key.
        let octetstring_bytes_string = "0420";
        let mut octetstring_array = [0u8; 2];
        hex::decode_to_slice(octetstring_bytes_string, &mut octetstring_array as &mut [u8]).ok();

        let mut final_key = [0 as u8; 34];
        let mut key_byte_sequence =  octetstring_array.iter()
            .chain(self.seed.iter());
        let _bytes_written = key_byte_sequence
            .by_ref()
            .zip(final_key.as_mut())
            .fold(0, | cnt, (item, slot) | {
                *slot = item.clone(); cnt+1
            });

        PrivateKeyInfo {
            algorithm: ALGORITHM_ID,
            private_key: &final_key,
        }.into()
    }
}

impl FromPrivateKey for SigningKey {
    fn from_pkcs8_private_key_info(pki: PrivateKeyInfo<'_>) -> Result<Self, pkcs8::Error> {
        // Split off the extra octet string bytes.
        let (octetstring_prefix, private_key) = pki.private_key.split_at(2);
        if hex::encode(octetstring_prefix).ne("0420") {
            Err(pkcs8::Error::Decode)
        }
        else {
            SigningKey::try_from(private_key).map_err(|_| pkcs8::Error::Decode)
        }
    }
}

#[cfg(feature = "pem")]
impl From<PrivateKeyDocument> for SigningKey {
    fn from(doc: PrivateKeyDocument) -> SigningKey {
        let pki = doc.unwrap();
        pki.private_key.try_into().expect("Ed25519 private key wasn't 32 bytes")
    }
}

#[cfg(feature = "pem")]
impl From<SigningKey> for PublicKeyDocument {
    fn from(sk: SigningKey) -> Result<PublicKeyDocument, Error> {
        let pki = PrivateKeyInfo::try_from(sk.seed).unwrap();
        PublicKeyDocument::try_from(pki)
    }
}

impl zeroize::Zeroize for SigningKey {
    fn zeroize(&mut self) {
        self.seed.zeroize();
        self.s.zeroize()
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct SerdeHelper([u8; 32]);

impl From<SerdeHelper> for SigningKey {
    fn from(helper: SerdeHelper) -> SigningKey {
        helper.0.into()
    }
}

impl From<SigningKey> for SerdeHelper {
    fn from(sk: SigningKey) -> Self {
        Self(sk.into())
    }
}

impl SigningKey {
    /// Generate a new signing key.
    pub fn new<R: RngCore + CryptoRng>(mut rng: R) -> SigningKey {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes[..]);
        bytes.into()
    }

    /// Create a signature on `msg` using this key.
    #[allow(non_snake_case)]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let r = Scalar::from_hash(Sha512::default().chain(&self.prefix[..]).chain(msg));

        let R_bytes = (&r * &constants::ED25519_BASEPOINT_TABLE)
            .compress()
            .to_bytes();

        let k = Scalar::from_hash(
            Sha512::default()
                .chain(&R_bytes[..])
                .chain(&self.vk.A_bytes.0[..])
                .chain(msg),
        );

        let s_bytes = (r + k * self.s).to_bytes();

        Signature { R_bytes, s_bytes }
    }
}
