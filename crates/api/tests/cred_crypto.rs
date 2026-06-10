//! P1-T11: Credential encryption integration tests.

use api::credentials::{CredentialCrypto, PlaintextCredential};

fn crypto() -> CredentialCrypto {
    CredentialCrypto::new(vec![0xABu8; 32], 1).unwrap()
}

#[test]
fn encrypt_decrypt_round_trips_to_original_plaintext() {
    let original = b"coinbase_api_key=secret&api_secret=topsecret";
    let creds = PlaintextCredential::new(original.to_vec());
    let enc = crypto().encrypt(&creds).unwrap();
    let dec = crypto().decrypt(&enc).unwrap();
    assert_eq!(dec.bytes, original);
}

#[test]
fn two_encryptions_of_same_plaintext_produce_different_ciphertext() {
    let creds = PlaintextCredential::new(b"same text".to_vec());
    let c = crypto();
    let enc1 = c.encrypt(&creds).unwrap();
    let enc2 = c.encrypt(&creds).unwrap();
    assert_ne!(
        enc1.ciphertext, enc2.ciphertext,
        "random nonce must produce different ciphertext"
    );
    assert_ne!(enc1.nonce, enc2.nonce, "nonces must differ");
}

#[test]
fn plaintext_debug_output_is_redacted() {
    let creds = PlaintextCredential::new(b"my_secret_api_key_123".to_vec());
    let debug = format!("{creds:?}");
    assert!(
        !debug.contains("my_secret_api_key_123"),
        "Debug output must not expose credential bytes"
    );
    assert!(debug.contains("REDACTED"));
}

#[test]
fn verify_then_store_calls_verifier_before_encrypting() {
    let creds = PlaintextCredential::new(b"test_key".to_vec());
    let c = crypto();
    let mut verifier_called = false;
    let result = c.verify_then_store(&creds, |bytes| {
        verifier_called = true;
        assert_eq!(bytes, b"test_key");
        Ok(())
    });
    assert!(result.is_ok());
    assert!(verifier_called, "verifier closure must be called");
}

#[test]
fn verify_then_store_does_not_store_when_verifier_fails() {
    let creds = PlaintextCredential::new(b"bad_key".to_vec());
    let c = crypto();
    let result = c.verify_then_store(&creds, |_| {
        Err(api::credentials::crypto::CredentialError::Decrypt)
    });
    assert!(result.is_err(), "must not encrypt when verifier fails");
}
