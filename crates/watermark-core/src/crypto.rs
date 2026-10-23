use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use pbkdf2::pbkdf2_hmac;
#[cfg(not(test))]
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{CoreError, Result};

pub const DEFAULT_PBKDF2_ITERATIONS: u32 = 210_000;
pub const SALT_LEN: usize = 16;
pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    #[cfg(not(test))]
    OsRng.fill_bytes(&mut bytes);
    #[cfg(test)]
    {
        use rand::SeedableRng;
        // 固定每个测试线程的随机流，使失败可复现；此分支不进入生产构建，生产仍使用系统随机数。
        // Fix each test thread's random stream for reproducible failures; this branch is absent from production builds, which retain OS randomness.
        thread_local! {static RNG:std::cell::RefCell<rand::rngs::StdRng>=std::cell::RefCell::new(rand::rngs::StdRng::seed_from_u64(0x5179a3));}
        RNG.with(|rng| rng.borrow_mut().fill_bytes(&mut bytes));
    }
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

pub fn decrypt(
    ciphertext: &[u8],
    nonce: &[u8; NONCE_LEN],
    key: &[u8; KEY_LEN],
) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| CoreError::InvalidArgument("ChaCha20-Poly1305 密钥长度无效".into()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| CoreError::DecryptionFailed)?;
    Ok(Zeroizing::new(plaintext))
}
