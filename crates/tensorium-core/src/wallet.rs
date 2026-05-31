use bech32::{ToBase32, Variant};
use k256::{
    ecdsa::{
        signature::{Signer, Verifier},
        Signature, SigningKey, VerifyingKey,
    },
    SecretKey,
};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::block::{Transaction, TxInput};

const ADDRESS_HRP: &str = "txm";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WalletKeypair {
    pub private_key_hex: String,
    pub public_key_hex: String,
    pub address: Address,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Address(pub String);

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("invalid private key")]
    InvalidPrivateKey,
    #[error("address encode failed")]
    AddressEncode,
    #[error("signature encode failed")]
    SignatureEncode,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignatureScript {
    pub public_key_hex: String,
    pub signature_hex: String,
}

impl WalletKeypair {
    pub fn generate() -> Self {
        let signing_key = SigningKey::random(&mut OsRng);
        Self::from_signing_key(signing_key)
    }

    pub fn from_private_key_hex(private_key_hex: &str) -> Result<Self, WalletError> {
        let private_key_bytes =
            hex::decode(private_key_hex).map_err(|_| WalletError::InvalidPrivateKey)?;
        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        Ok(Self::from_signing_key(SigningKey::from(secret_key)))
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let private_key_hex = hex::encode(signing_key.to_bytes());
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(true);
        let public_key_bytes = public_key.as_bytes();
        let public_key_hex = hex::encode(public_key_bytes);
        let address = Address::from_public_key(public_key_bytes)
            .expect("compressed secp256k1 public key can be encoded as address");

        Self {
            private_key_hex,
            public_key_hex,
            address,
        }
    }

    pub fn sign_transaction(&self, tx: &mut Transaction) -> Result<(), WalletError> {
        let private_key_bytes =
            hex::decode(&self.private_key_hex).map_err(|_| WalletError::InvalidPrivateKey)?;
        let secret_key = SecretKey::from_slice(&private_key_bytes)
            .map_err(|_| WalletError::InvalidPrivateKey)?;
        let signing_key = SigningKey::from(secret_key);
        let signature_hash = tx.signature_hash();
        let signature: Signature = signing_key.sign(&signature_hash.0);
        let script = SignatureScript {
            public_key_hex: self.public_key_hex.clone(),
            signature_hex: hex::encode(signature.to_der().as_bytes()),
        };
        let script_bytes = serde_json::to_vec(&script).map_err(|_| WalletError::SignatureEncode)?;
        for input in &mut tx.inputs {
            input.signature_script = script_bytes.clone();
        }
        tx.refresh_id();
        Ok(())
    }
}

impl Address {
    pub fn from_public_key(public_key_bytes: &[u8]) -> Result<Self, WalletError> {
        let digest = Sha256::digest(public_key_bytes);
        let payload = &digest[..20];
        let encoded = bech32::encode(ADDRESS_HRP, payload.to_base32(), Variant::Bech32)
            .map_err(|_| WalletError::AddressEncode)?;
        Ok(Self(encoded))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn verify_transaction_input(
    tx: &Transaction,
    input: &TxInput,
    expected_address: &str,
) -> Result<(), WalletError> {
    let script: SignatureScript = serde_json::from_slice(&input.signature_script)
        .map_err(|_| WalletError::InvalidSignature)?;
    let public_key_bytes =
        hex::decode(&script.public_key_hex).map_err(|_| WalletError::InvalidPublicKey)?;
    let address = Address::from_public_key(&public_key_bytes)?;
    if address.as_str() != expected_address {
        return Err(WalletError::InvalidSignature);
    }

    let verifying_key = VerifyingKey::from_sec1_bytes(&public_key_bytes)
        .map_err(|_| WalletError::InvalidPublicKey)?;
    let signature_bytes =
        hex::decode(&script.signature_hex).map_err(|_| WalletError::InvalidSignature)?;
    let signature =
        Signature::from_der(&signature_bytes).map_err(|_| WalletError::InvalidSignature)?;
    verifying_key
        .verify(&tx.signature_hash().0, &signature)
        .map_err(|_| WalletError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_txm_address() {
        let keypair = WalletKeypair::generate();
        assert!(keypair.address.as_str().starts_with("txm1"));
        assert_eq!(keypair.private_key_hex.len(), 64);
        assert_eq!(keypair.public_key_hex.len(), 66);
    }

    #[test]
    fn restores_same_address_from_private_key() {
        let keypair = WalletKeypair::generate();
        let restored = WalletKeypair::from_private_key_hex(&keypair.private_key_hex).unwrap();
        assert_eq!(restored.address, keypair.address);
        assert_eq!(restored.public_key_hex, keypair.public_key_hex);
    }

    #[test]
    fn signs_and_verifies_payment_transaction() {
        use crate::block::{OutPoint, TxOutput};
        use crate::hash::Hash256;

        let keypair = WalletKeypair::generate();
        let mut tx = Transaction::payment(
            vec![TxInput {
                previous_output: OutPoint {
                    txid: Hash256::ZERO,
                    output_index: 0,
                },
                signature_script: Vec::new(),
            }],
            vec![TxOutput {
                value_atoms: 42,
                address: keypair.address.as_str().to_owned(),
            }],
        );

        keypair.sign_transaction(&mut tx).unwrap();
        verify_transaction_input(&tx, &tx.inputs[0], keypair.address.as_str()).unwrap();
    }
}
