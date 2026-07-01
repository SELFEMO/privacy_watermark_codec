use chrono::Utc;
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{decrypt, encrypt, NONCE_LEN, SALT_LEN},
    error::{CoreError, Result},
    keyfile::WatermarkKey,
};

pub const MAGIC: [u8; 4] = *b"PWW1";
pub const FORMAT_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 32;
pub const HEADER_BITS: usize = HEADER_LEN * 8;

#[derive(Debug, Clone)]
pub struct Header {
    pub salt: [u8; SALT_LEN],
    pub body_len: u32,
    pub route_step: u16,
    pub strength_x10: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerPayload {
    pub text: String,
    pub fingerprint_hex: String,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
    pub media_kind: String,
}

#[derive(Debug, Clone)]
pub struct EncryptedBody {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl InnerPayload {
    pub fn new(text: String, fingerprint: u64, width: u32, height: u32, media_kind: &str) -> Self {
        Self {
            text,
            fingerprint_hex: format!("{fingerprint:016x}"),
            width,
            height,
            created_at: Utc::now().to_rfc3339(),
            media_kind: media_kind.to_owned(),
        }
    }

    pub fn fingerprint(&self) -> Result<u64> {
        u64::from_str_radix(&self.fingerprint_hex, 16)
            .map_err(|_| CoreError::PayloadCorrupted)
    }
}

impl Header {
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0_u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = FORMAT_VERSION;
        bytes[5..21].copy_from_slice(&self.salt);
        bytes[21..25].copy_from_slice(&self.body_len.to_le_bytes());
        bytes[25..27].copy_from_slice(&self.route_step.to_le_bytes());
        bytes[27] = self.strength_x10;
        let crc = crc32(&bytes[..28]);
        bytes[28..32].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != HEADER_LEN || &bytes[0..4] != MAGIC.as_slice() || bytes[4] != FORMAT_VERSION {
            return Err(CoreError::HeaderNotFound);
        }
        let expected = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        if crc32(&bytes[..28]) != expected {
            return Err(CoreError::HeaderNotFound);
        }
        let salt = bytes[5..21].try_into().unwrap();
        let body_len = u32::from_le_bytes(bytes[21..25].try_into().unwrap());
        let route_step = u16::from_le_bytes(bytes[25..27].try_into().unwrap());
        let strength_x10 = bytes[27];
        if body_len < (NONCE_LEN + 16 + 4) as u32 || body_len > 65_536 || route_step < 3 {
            return Err(CoreError::HeaderNotFound);
        }
        Ok(Self {
            salt,
            body_len,
            route_step,
            strength_x10,
        })
    }
}

pub fn create_encrypted_body(payload: &InnerPayload, key: &WatermarkKey) -> Result<Vec<u8>> {
    let plaintext = serde_json::to_vec(payload)?;
    let (nonce, ciphertext) = encrypt(&plaintext, &key.derived_key)?;
    let mut body = Vec::with_capacity(NONCE_LEN + ciphertext.len() + 4);
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&ciphertext);
    let checksum = crc32(&body);
    body.extend_from_slice(&checksum.to_le_bytes());
    Ok(body)
}

pub fn open_encrypted_body(body: &[u8], key: &WatermarkKey) -> Result<InnerPayload> {
    if body.len() < NONCE_LEN + 16 + 4 {
        return Err(CoreError::PayloadCorrupted);
    }
    let data_len = body.len() - 4;
    let expected = u32::from_le_bytes(body[data_len..].try_into().unwrap());
    if crc32(&body[..data_len]) != expected {
        return Err(CoreError::PayloadCorrupted);
    }
    let nonce: [u8; NONCE_LEN] = body[..NONCE_LEN].try_into().unwrap();
    let plaintext = decrypt(&body[NONCE_LEN..data_len], &nonce, &key.derived_key)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreError::DecryptionFailed)
}

pub fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for &byte in bytes {
        for shift in (0..8).rev() {
            bits.push(((byte >> shift) & 1) == 1);
        }
    }
    bits
}

pub fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    bits.chunks_exact(8)
        .map(|chunk| {
            chunk.iter().fold(0_u8, |value, bit| (value << 1) | u8::from(*bit))
        })
        .collect()
}

fn crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}
