use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit, generic_array::GenericArray},
};

/// Encrypts `plaintext` with AES-256-GCM using a fresh random 12-byte nonce.
/// Returns `"enc:<hex(nonce || ciphertext)>"`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("Invalid key length"))?;
    let nonce_arr: [u8; 12] = rand::random();
    // GenericArray length (U12) is inferred from Aes256Gcm::encrypt's expected nonce type.
    let nonce = GenericArray::from_slice(&nonce_arr);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("Encryption failed"))?;
    let mut combined = nonce_arr.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(format!("enc:{}", hex_encode(&combined)))
}

/// Decrypts a value produced by `encrypt`.
/// Values without the `"enc:"` prefix are returned unchanged (plaintext passthrough for migration).
pub fn decrypt(key: &[u8; 32], value: &str) -> anyhow::Result<String> {
    let Some(hex_str) = value.strip_prefix("enc:") else {
        return Ok(value.to_string());
    };
    let bytes = hex_decode(hex_str)?;
    if bytes.len() < 12 {
        anyhow::bail!("Encrypted value too short");
    }
    let (nonce_bytes, ciphertext) = bytes.split_at(12);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("Invalid key length"))?;
    let nonce = GenericArray::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed (wrong key or corrupted data)"))?;
    String::from_utf8(plaintext).map_err(|e| anyhow::anyhow!("Decrypted value is not UTF-8: {e}"))
}

/// Parses a 64-character lowercase hex string into a 32-byte AES-256 key.
pub fn parse_key(hex: &str) -> anyhow::Result<[u8; 32]> {
    hex_decode(hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("WALKER_ENCRYPTION_KEY must be 64 hex characters (32 bytes)"))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        anyhow::bail!("Odd-length hex string");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| anyhow::anyhow!("Invalid hex at position {i}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    #[test]
    fn roundtrip() {
        let key = test_key();
        let encrypted = encrypt(&key, "my_strava_client_secret").unwrap();
        assert!(encrypted.starts_with("enc:"));
        assert_eq!(
            decrypt(&key, &encrypted).unwrap(),
            "my_strava_client_secret"
        );
    }

    #[test]
    fn each_encrypt_produces_different_ciphertext() {
        let key = test_key();
        let enc1 = encrypt(&key, "same_value").unwrap();
        let enc2 = encrypt(&key, "same_value").unwrap();
        assert_ne!(enc1, enc2, "each call must use a fresh nonce");
    }

    #[test]
    fn plaintext_passthrough() {
        let key = test_key();
        // Values without the "enc:" prefix are returned as-is (migration compat).
        assert_eq!(
            decrypt(&key, "old_plaintext_token").unwrap(),
            "old_plaintext_token"
        );
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = [0x01u8; 32];
        let key2 = [0x02u8; 32];
        let encrypted = encrypt(&key1, "secret").unwrap();
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn corrupted_ciphertext_fails() {
        let key = test_key();
        let mut encrypted = encrypt(&key, "secret").unwrap();
        // Flip a nibble in the hex after the "enc:" prefix + nonce (first 24 chars).
        let pos = "enc:".len() + 24 + 2; // byte after nonce
        unsafe {
            let bytes = encrypted.as_bytes_mut();
            bytes[pos] ^= 0x01;
        }
        assert!(decrypt(&key, &encrypted).is_err());
    }

    #[test]
    fn empty_string_roundtrip() {
        let key = test_key();
        let encrypted = encrypt(&key, "").unwrap();
        assert_eq!(decrypt(&key, &encrypted).unwrap(), "");
    }

    #[test]
    fn parse_key_valid() {
        let hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let key = parse_key(hex).unwrap();
        assert_eq!(key[0], 0x01);
        assert_eq!(key[31], 0x20);
    }

    #[test]
    fn parse_key_wrong_length() {
        assert!(parse_key("0102").is_err());
        assert!(parse_key(&"aa".repeat(33)).is_err()); // 66 chars = 33 bytes
    }

    #[test]
    fn parse_key_invalid_hex() {
        assert!(parse_key(&"zz".repeat(32)).is_err());
    }
}
