use chrono::Utc;
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{decrypt, encrypt, NONCE_LEN, SALT_LEN},
    error::{CoreError, Result},
    fingerprint::PartitionFingerprint,
    keyfile::WatermarkKey,
};

pub const LEGACY_MAGIC: [u8; 4] = *b"PWW1";
pub const LEGACY_FORMAT: u8 = 1;
pub const LEGACY_HEADER_LEN: usize = 32;
pub const LEGACY_HEADER_BITS: usize = LEGACY_HEADER_LEN * 8;
pub const ROBUST_MAGIC: [u8; 4] = *b"PWW2";
pub const ROBUST_FORMAT: u8 = 2;
pub const ROBUST_HEADER_LEN: usize = 40;
pub const ROBUST_HEADER_BITS: usize = ROBUST_HEADER_LEN * 8;
pub const ROBUST_PILOT: [u8; 4] = *b"RG21";
const COMPACT_MAGIC: [u8; 4] = *b"PCP2";
const FLAG_VIDEO: u8 = 0x01;

#[derive(Debug, Clone)]
pub struct LegacyHeader {
    pub salt: [u8; SALT_LEN],
    pub body_len: u32,
    pub route_step: u16,
    pub strength_x10: u8,
}

#[derive(Debug, Clone)]
pub struct RobustHeader {
    pub salt: [u8; SALT_LEN],
    pub body_len: u32,
    pub original_width: u16,
    pub original_height: u16,
    pub strength_x10: u8,
    pub flags: u8,
    pub qim_profile: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemporalPayload {
    pub clip_id_hex: String,
    pub frame_index: u32,
    pub frame_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerPayload {
    pub text: String,
    pub fingerprint_hex: String,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
    pub media_kind: String,
    #[serde(default)]
    pub partition_fingerprints: Vec<PartitionFingerprint>,
    #[serde(default)]
    pub temporal: Option<TemporalPayload>,
}

impl InnerPayload {
    pub fn new(
        text: String,
        fingerprint: u64,
        width: u32,
        height: u32,
        media_kind: &str,
        partition_fingerprints: Vec<PartitionFingerprint>,
        temporal: Option<TemporalPayload>,
    ) -> Self {
        Self {
            text,
            fingerprint_hex: format!("{fingerprint:016x}"),
            width,
            height,
            created_at: Utc::now().to_rfc3339(),
            media_kind: media_kind.to_owned(),
            partition_fingerprints,
            temporal,
        }
    }

    pub fn fingerprint(&self) -> Result<u64> {
        u64::from_str_radix(&self.fingerprint_hex, 16).map_err(|_| CoreError::PayloadCorrupted)
    }
}

impl LegacyHeader {
    pub fn to_bytes(&self) -> [u8; LEGACY_HEADER_LEN] {
        let mut bytes = [0_u8; LEGACY_HEADER_LEN];
        bytes[0..4].copy_from_slice(&LEGACY_MAGIC);
        bytes[4] = LEGACY_FORMAT;
        bytes[5..21].copy_from_slice(&self.salt);
        bytes[21..25].copy_from_slice(&self.body_len.to_le_bytes());
        bytes[25..27].copy_from_slice(&self.route_step.to_le_bytes());
        bytes[27] = self.strength_x10;
        // 先完成只读 CRC 计算再写回缓冲区，避免同一表达式产生重叠的可变与不可变借用。
        // Computing the CRC before writing it back avoids overlapping mutable and immutable borrows in one expression.
        let checksum = crc32(&bytes[..28]);
        bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != LEGACY_HEADER_LEN
            || &bytes[0..4] != LEGACY_MAGIC.as_slice()
            || bytes[4] != LEGACY_FORMAT
        {
            return Err(CoreError::HeaderNotFound);
        }
        let expected = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        if crc32(&bytes[..28]) != expected {
            return Err(CoreError::HeaderNotFound);
        }
        let header = Self {
            salt: bytes[5..21].try_into().unwrap(),
            body_len: u32::from_le_bytes(bytes[21..25].try_into().unwrap()),
            route_step: u16::from_le_bytes(bytes[25..27].try_into().unwrap()),
            strength_x10: bytes[27],
        };
        if header.body_len < (NONCE_LEN + 16 + 4) as u32
            || header.body_len > 65_536
            || header.route_step < 3
        {
            return Err(CoreError::HeaderNotFound);
        }
        Ok(header)
    }
}

impl RobustHeader {
    pub fn to_bytes(&self) -> [u8; ROBUST_HEADER_LEN] {
        let mut bytes = [0_u8; ROBUST_HEADER_LEN];
        bytes[0..4].copy_from_slice(&ROBUST_MAGIC);
        bytes[4] = ROBUST_FORMAT;
        bytes[5] = self.flags;
        bytes[6..22].copy_from_slice(&self.salt);
        bytes[22..26].copy_from_slice(&self.body_len.to_le_bytes());
        bytes[26..28].copy_from_slice(&self.original_width.to_le_bytes());
        bytes[28..30].copy_from_slice(&self.original_height.to_le_bytes());
        bytes[30] = self.strength_x10;
        bytes[31..35].copy_from_slice(&ROBUST_PILOT);
        // 复用既有保留字节记录 QIM 配置，是为了在不改变 PWW2 头长度和空间路由的情况下升级抗旋转能力，并让旧文件的零值配置自然保持兼容。
        // Reusing the existing reserved byte for the QIM profile upgrades rotation resilience without changing the PWW2 header length or spatial route, while legacy files naturally keep profile zero.
        bytes[35] = self.qim_profile;
        // 先完成只读 CRC 计算再写回缓冲区，避免同一表达式产生重叠的可变与不可变借用。
        // Computing the CRC before writing it back avoids overlapping mutable and immutable borrows in one expression.
        let checksum = crc32(&bytes[..36]);
        bytes[36..40].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != ROBUST_HEADER_LEN
            || &bytes[0..4] != ROBUST_MAGIC.as_slice()
            || bytes[4] != ROBUST_FORMAT
            || &bytes[31..35] != ROBUST_PILOT.as_slice()
        {
            return Err(CoreError::HeaderNotFound);
        }
        let expected = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
        if crc32(&bytes[..36]) != expected {
            return Err(CoreError::HeaderNotFound);
        }
        let header = Self {
            flags: bytes[5],
            salt: bytes[6..22].try_into().unwrap(),
            body_len: u32::from_le_bytes(bytes[22..26].try_into().unwrap()),
            original_width: u16::from_le_bytes(bytes[26..28].try_into().unwrap()),
            original_height: u16::from_le_bytes(bytes[28..30].try_into().unwrap()),
            strength_x10: bytes[30],
            qim_profile: bytes[35],
        };
        if header.body_len < (NONCE_LEN + 16 + 4) as u32
            || header.body_len > 65_536
            || header.original_width < 256
            || header.original_height < 256
            || header.flags & !FLAG_VIDEO != 0
            || header.qim_profile > crate::dwt_qim::QIM_PROFILE_PYRAMID
        {
            return Err(CoreError::HeaderNotFound);
        }
        Ok(header)
    }

    pub fn is_video(&self) -> bool {
        self.flags & FLAG_VIDEO != 0
    }
}

pub fn robust_flags(temporal: Option<&TemporalPayload>) -> u8 {
    if temporal.is_some() { FLAG_VIDEO } else { 0 }
}

pub fn create_encrypted_body(payload: &InnerPayload, key: &WatermarkKey) -> Result<Vec<u8>> {
    let plaintext = encode_compact_payload(payload)?;
    let (nonce, ciphertext) = encrypt(&plaintext, &key.derived_key)?;
    let mut body = Vec::with_capacity(NONCE_LEN + ciphertext.len() + 4);
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&ciphertext);
    body.extend_from_slice(&crc32(&body).to_le_bytes());
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
    if plaintext.starts_with(&COMPACT_MAGIC) {
        decode_compact_payload(&plaintext)
    } else {
        // 保留 JSON 明文回退是为了让升级后的程序仍能读取旧 PWW1 媒体，而不是强迫用户重新编码历史文件。
        // Retaining the JSON plaintext fallback keeps old PWW1 media readable after the upgrade instead of forcing users to re-encode historical files.
        serde_json::from_slice(&plaintext).map_err(|_| CoreError::DecryptionFailed)
    }
}

fn encode_compact_payload(payload: &InnerPayload) -> Result<Vec<u8>> {
    let text = payload.text.as_bytes();
    if text.len() > u16::MAX as usize || payload.width > u16::MAX as u32 || payload.height > u16::MAX as u32 {
        return Err(CoreError::InvalidArgument("水印文本或图像尺寸超出协议范围".into()));
    }
    let temporal = payload.temporal.as_ref();
    let mut bytes = Vec::with_capacity(48 + text.len() + payload.partition_fingerprints.len() * 9);
    bytes.extend_from_slice(&COMPACT_MAGIC);
    bytes.push(robust_flags(temporal));
    bytes.push(payload.partition_fingerprints.len().min(u8::MAX as usize) as u8);
    bytes.extend_from_slice(&(payload.width as u16).to_le_bytes());
    bytes.extend_from_slice(&(payload.height as u16).to_le_bytes());
    bytes.extend_from_slice(&payload.fingerprint()?.to_le_bytes());
    bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
    bytes.extend_from_slice(text);
    for partition in &payload.partition_fingerprints {
        let hash = u64::from_str_radix(&partition.hash_hex, 16).map_err(|_| CoreError::PayloadCorrupted)?;
        bytes.push(partition.index as u8);
        bytes.extend_from_slice(&hash.to_le_bytes());
    }
    if let Some(temporal) = temporal {
        let clip = decode_fixed_hex::<16>(&temporal.clip_id_hex)?;
        if temporal.frame_count == 0 || temporal.frame_index >= temporal.frame_count {
            return Err(CoreError::InvalidArgument("视频帧序号超出总帧数".into()));
        }
        bytes.extend_from_slice(&clip);
        bytes.extend_from_slice(&temporal.frame_index.to_le_bytes());
        bytes.extend_from_slice(&temporal.frame_count.to_le_bytes());
    }
    Ok(bytes)
}

fn decode_compact_payload(bytes: &[u8]) -> Result<InnerPayload> {
    let mut cursor = 4_usize;
    let flags = take_u8(bytes, &mut cursor)?;
    let partition_count = take_u8(bytes, &mut cursor)? as usize;
    let width = take_u16(bytes, &mut cursor)? as u32;
    let height = take_u16(bytes, &mut cursor)? as u32;
    let fingerprint = take_u64(bytes, &mut cursor)?;
    let text_len = take_u16(bytes, &mut cursor)? as usize;
    let text_slice = take(bytes, &mut cursor, text_len)?;
    let text = String::from_utf8(text_slice.to_vec()).map_err(|_| CoreError::PayloadCorrupted)?;
    let mut partitions = Vec::with_capacity(partition_count);
    for _ in 0..partition_count {
        let index = take_u8(bytes, &mut cursor)? as usize;
        let hash = take_u64(bytes, &mut cursor)?;
        let gx = index as u32 % 4;
        let gy = index as u32 / 4;
        let x = gx * width / 4;
        let y = gy * height / 4;
        let next_x = (gx + 1) * width / 4;
        let next_y = (gy + 1) * height / 4;
        partitions.push(PartitionFingerprint {
            index,
            x,
            y,
            width: next_x.saturating_sub(x).max(1),
            height: next_y.saturating_sub(y).max(1),
            hash_hex: format!("{hash:016x}"),
        });
    }
    let temporal = if flags & FLAG_VIDEO != 0 {
        let clip: [u8; 16] = take(bytes, &mut cursor, 16)?.try_into().unwrap();
        let frame_index = take_u32(bytes, &mut cursor)?;
        let frame_count = take_u32(bytes, &mut cursor)?;
        if frame_count == 0 || frame_index >= frame_count {
            return Err(CoreError::PayloadCorrupted);
        }
        Some(TemporalPayload {
            clip_id_hex: bytes_to_hex(&clip),
            frame_index,
            frame_count,
        })
    } else {
        None
    };
    if cursor != bytes.len() || width < 256 || height < 256 || flags & !FLAG_VIDEO != 0 {
        return Err(CoreError::PayloadCorrupted);
    }
    Ok(InnerPayload {
        text,
        fingerprint_hex: format!("{fingerprint:016x}"),
        width,
        height,
        created_at: String::new(),
        media_kind: if temporal.is_some() { "video_frame".into() } else { "image".into() },
        partition_fingerprints: partitions,
        temporal,
    })
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
        .map(|chunk| chunk.iter().fold(0_u8, |value, bit| (value << 1) | u8::from(*bit)))
        .collect()
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = cursor.checked_add(len).ok_or(CoreError::PayloadCorrupted)?;
    let value = bytes.get(*cursor..end).ok_or(CoreError::PayloadCorrupted)?;
    *cursor = end;
    Ok(value)
}

fn take_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8> {
    Ok(take(bytes, cursor, 1)?[0])
}
fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16> {
    Ok(u16::from_le_bytes(take(bytes, cursor, 2)?.try_into().unwrap()))
}
fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    Ok(u32::from_le_bytes(take(bytes, cursor, 4)?.try_into().unwrap()))
}
fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    Ok(u64::from_le_bytes(take(bytes, cursor, 8)?.try_into().unwrap()))
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    if value.len() != N * 2 {
        return Err(CoreError::InvalidArgument("视频片段标识长度无效".into()));
    }
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| CoreError::InvalidArgument("视频片段标识不是十六进制".into()))?;
    }
    Ok(output)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyfile::{KeyMode, WatermarkKey};

    #[test]
    fn robust_header_preserves_qim_profile_and_accepts_legacy_zero() {
        let mut header = RobustHeader {
            salt: [3; 16],
            body_len: 64,
            original_width: 640,
            original_height: 480,
            strength_x10: 100,
            flags: 0,
            qim_profile: crate::dwt_qim::QIM_PROFILE_PYRAMID,
        };
        let decoded = RobustHeader::from_bytes(&header.to_bytes()).unwrap();
        assert_eq!(decoded.qim_profile, crate::dwt_qim::QIM_PROFILE_PYRAMID);

        header.qim_profile = crate::dwt_qim::QIM_PROFILE_LEGACY;
        let legacy = RobustHeader::from_bytes(&header.to_bytes()).unwrap();
        assert_eq!(legacy.qim_profile, crate::dwt_qim::QIM_PROFILE_LEGACY);
    }

    #[test]
    fn compact_temporal_payload_round_trip() {
        let key = WatermarkKey::random(KeyMode::Independent);
        let payload = InnerPayload::new(
            "timeline".into(),
            42,
            640,
            480,
            "video_frame",
            Vec::new(),
            Some(TemporalPayload {
                clip_id_hex: "00112233445566778899aabbccddeeff".into(),
                frame_index: 7,
                frame_count: 30,
            }),
        );
        let opened = open_encrypted_body(&create_encrypted_body(&payload, &key).unwrap(), &key).unwrap();
        assert_eq!(opened.text, payload.text);
        assert_eq!(opened.temporal, payload.temporal);
    }
}
