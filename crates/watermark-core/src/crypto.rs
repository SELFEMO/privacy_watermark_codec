use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use pbkdf2::pbkdf2_hmac;
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{CoreError, Result};

pub const DEFAULT_PBKDF2_ITERATIONS: u32 = 210_000;
pub const SALT_LEN: usize = 16;
pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn derive_key(secret: &[u8], salt: &[u8; SALT_LEN], iterations: u32) -> [u8; KEY_LEN] {
    let mut key = [0_u8; KEY_LEN];

    // 使用较高迭代次数提高离线口令猜测成本，同时保持桌面端可接受的交互延迟。
    // A high iteration count raises the cost of offline password guessing while keeping desktop interaction practical.
    pbkdf2_hmac::<Sha256>(secret, salt, iterations, &mut key);
    key
}

pub fn encrypt(plaintext: &[u8], key: &[u8; KEY_LEN]) -> Result<([u8; NONCE_LEN], Vec<u8>)> {
    let nonce = random_bytes::<NONCE_LEN>();
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| CoreError::InvalidArgument("ChaCha20-Poly1305 密钥长度无效".into()))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| CoreError::DecryptionFailed)?;
    Ok((nonce, ciphertext))
}

pub fn decrypt(ciphertext: &[u8], nonce: &[u8; NONCE_LEN], key: &[u8; KEY_LEN]) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| CoreError::InvalidArgument("ChaCha20-Poly1305 密钥长度无效".into()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| CoreError::DecryptionFailed)?;
    Ok(Zeroizing::new(plaintext))
}
