use aes_gcm::{Aes256Gcm, aead::{Aead, KeyInit, generic_array::GenericArray}};

/// Encrypts `plaintext` with AES-256-GCM using a fresh random 12-byte nonce.
/// Returns `"enc:<hex(nonce || ciphertext)>"`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("Invalid key length"))?;
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
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("Invalid key length"))?;
    let nonce = GenericArray::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed (wrong key or corrupted data)"))?;
    String::from_utf8(plaintext)
        .map_err(|e| anyhow::anyhow!("Decrypted value is not UTF-8: {e}"))
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
    if s.len() % 2 != 0 {
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
