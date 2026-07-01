use std::path::Path;

use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::{
    crypto::{derive_key, DEFAULT_PBKDF2_ITERATIONS},
    dct,
    error::{CoreError, Result},
    fingerprint::{difference_hash, hamming_distance},
    hamming,
    keyfile::{KeySource, WatermarkKey},
    payload::{
        bits_to_bytes, bytes_to_bits, create_encrypted_body, open_encrypted_body, Header,
        InnerPayload, HEADER_BITS,
    },
};

const BLOCK_SIZE: u32 = 8;
const HEADER_MODULUS: u32 = 5;
const MIN_WIDTH: u32 = 256;
const MIN_HEIGHT: u32 = 256;
const MIN_PSNR_DB: f64 = 40.0;
const ROUTE_STEPS: [u16; 24] = [
    257, 263, 269, 271, 277, 281, 283, 293, 307, 311, 313, 317, 331, 337, 347, 349,
    353, 359, 367, 373, 379, 383, 389, 397,
];

#[derive(Debug, Clone)]
pub struct EmbedOptions {
    pub text: String,
    pub key: WatermarkKey,
    pub strength: f32,
    pub media_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedReport {
    pub output_path: String,
    pub width: u32,
    pub height: u32,
    pub psnr: f64,
    pub payload_bytes: usize,
    pub header_min_votes: usize,
    pub body_min_votes: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    Intact,
    Uncertain,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractReport {
    pub text: String,
    pub integrity: IntegrityStatus,
    pub fingerprint_distance: u32,
    pub corrected_codewords: usize,
    pub original_width: u32,
    pub original_height: u32,
    pub current_width: u32,
    pub current_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicWatermarkHeader {
    pub salt_hex: String,
    pub body_len: u32,
    pub route_step: u16,
    pub strength: f32,
}

pub fn embed_image_file(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &EmbedOptions,
) -> Result<EmbedReport> {
    if options.text.trim().is_empty() {
        return Err(CoreError::InvalidArgument("水印文本不能为空".into()));
    }
    if !(3.0..=20.0).contains(&options.strength) {
        return Err(CoreError::InvalidArgument("嵌入强度必须位于 3 到 20 之间".into()));
    }

    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();
    let original = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    validate_dimensions(&original)?;

    let fingerprint = difference_hash(&original);
    let payload = InnerPayload::new(
        options.text.clone(),
        fingerprint,
        original.width(),
        original.height(),
        &options.media_kind,
    );
    let body = create_encrypted_body(&payload, &options.key)?;
    let body_bits = hamming::encode_bytes(&body);
    let (route_step, header_min_votes, body_min_votes) = select_route(
        original.width(),
        original.height(),
        body_bits.len(),
        &options.key.salt,
    )?;
    let header = Header {
        salt: options.key.salt,
        body_len: body.len() as u32,
        route_step,
        strength_x10: (options.strength * 10.0).round() as u8,
    };
    let header_bits = bytes_to_bits(&header.to_bytes());

    info!(
        input = %input_path.display(),
        width = original.width(),
        height = original.height(),
        payload_bytes = body.len(),
        body_bits = body_bits.len(),
        route_step,
        "开始嵌入图片水印"
    );

    let original_rgba = original.to_rgba8();
    let mut watermarked = original_rgba.clone();
    embed_bits(
        &mut watermarked,
        &header_bits,
        &body_bits,
        route_step,
        &options.key.salt,
        options.strength,
    );

    let psnr = calculate_psnr(&original_rgba, &watermarked);
    if psnr < MIN_PSNR_DB {
        return Err(CoreError::InvalidArgument(format!(
            "当前参数得到的 PSNR 为 {psnr:.2} dB，低于 {MIN_PSNR_DB:.0} dB。请降低嵌入强度或缩短文本"
        )));
    }

    DynamicImage::ImageRgba8(watermarked)
        .save_with_format(output_path, ImageFormat::Png)
        .map_err(|source| CoreError::ImageSave {
            path: output_path.to_path_buf(),
            source,
        })?;

    info!(
        output = %output_path.display(),
        psnr,
        "图片水印嵌入完成"
    );

    Ok(EmbedReport {
        output_path: output_path.display().to_string(),
        width: original.width(),
        height: original.height(),
        psnr,
        payload_bytes: body.len(),
        header_min_votes,
        body_min_votes,
    })
}

pub fn probe_embedded_header_file(
    input_path: impl AsRef<Path>,
) -> Result<Option<PublicWatermarkHeader>> {
    let input_path = input_path.as_ref();
    let image = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    if image.width() < MIN_WIDTH || image.height() < MIN_HEIGHT {
        return Ok(None);
    }

    let rgba = image.to_rgba8();
    let header_bits = match collect_header_bits(&rgba) {
        Ok(bits) => bits,
        Err(CoreError::HeaderNotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    let header_bytes = bits_to_bytes(&header_bits);
    let header = match Header::from_bytes(&header_bytes) {
        Ok(header) => header,
        Err(CoreError::HeaderNotFound) => return Ok(None),
        Err(error) => return Err(error),
    };

    // 无密钥扫描只验证公开头部的魔数与 CRC，避免为了“检测”而尝试破解加密正文。
    // Keyless scanning validates only the public magic and CRC header, avoiding any attempt to break the encrypted body for detection.
    Ok(Some(PublicWatermarkHeader {
        salt_hex: bytes_to_lower_hex(&header.salt),
        body_len: header.body_len,
        route_step: header.route_step,
        strength: header.strength_x10 as f32 / 10.0,
    }))
}

pub fn extract_image_file(
    input_path: impl AsRef<Path>,
    key_source: &KeySource,
) -> Result<ExtractReport> {
    let input_path = input_path.as_ref();
    let image = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    validate_dimensions(&image)?;
    let rgba = image.to_rgba8();

    let header_bits = collect_header_bits(&rgba)?;
    let header_bytes = bits_to_bytes(&header_bits);
    let header = Header::from_bytes(&header_bytes)?;
    let body_encoded_len = header.body_len as usize * 14;
    let available_body_blocks = count_body_blocks(rgba.width(), rgba.height());
    // 在分配投票数组前先按当前图片容量限制声明长度，避免恶意伪造水印头造成超大内存分配。
    // Validate the declared payload against current image capacity before allocation to prevent malicious headers from forcing huge buffers.
    if body_encoded_len > available_body_blocks {
        return Err(CoreError::PayloadCorrupted);
    }
    let body_bits = collect_body_bits(&rgba, body_encoded_len, header.route_step, &header.salt)?;
    let decoded = hamming::decode_bits(&body_bits, header.body_len as usize)
        .ok_or(CoreError::PayloadCorrupted)?;

    let key = resolve_key(key_source, &header.salt)?;
    let payload = open_encrypted_body(&decoded.bytes, &key)?;
    let current_fingerprint = difference_hash(&image);
    let fingerprint_distance = hamming_distance(payload.fingerprint()?, current_fingerprint);
    let dimensions_changed = payload.width != image.width() || payload.height != image.height();
    let integrity = classify_integrity(fingerprint_distance, dimensions_changed);

    info!(
        input = %input_path.display(),
        corrected_codewords = decoded.corrected_codewords,
        fingerprint_distance,
        dimensions_changed,
        ?integrity,
        "图片水印解码完成"
    );

    Ok(ExtractReport {
        text: payload.text,
        integrity,
        fingerprint_distance,
        corrected_codewords: decoded.corrected_codewords,
        original_width: payload.width,
        original_height: payload.height,
        current_width: image.width(),
        current_height: image.height(),
    })
}

fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn resolve_key(source: &KeySource, embedded_salt: &[u8; 16]) -> Result<WatermarkKey> {
    match source {
        KeySource::KeyFile(file) => {
            let key = file.to_watermark_key()?;
            if &key.salt != embedded_salt {
                return Err(CoreError::SaltMismatch);
            }
            Ok(key)
        }
        KeySource::CustomPassword(password) => Ok(WatermarkKey {
            mode: crate::keyfile::KeyMode::Custom,
            salt: *embedded_salt,
            derived_key: derive_key(password.as_bytes(), embedded_salt, DEFAULT_PBKDF2_ITERATIONS),
            iterations: DEFAULT_PBKDF2_ITERATIONS,
        }),
    }
}

fn validate_dimensions(image: &DynamicImage) -> Result<()> {
    if image.width() < MIN_WIDTH || image.height() < MIN_HEIGHT {
        return Err(CoreError::ImageTooSmall);
    }
    Ok(())
}

fn embed_bits(
    image: &mut RgbaImage,
    header_bits: &[bool],
    body_bits: &[bool],
    route_step: u16,
    salt: &[u8; 16],
    strength: f32,
) {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let body_offset = salt_offset(salt, body_bits.len());

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if is_header_block(bx, by) {
                let index = header_index(bx, by);
                dct::write_bit(image, bx, by, header_bits[index], strength);
            } else {
                let index = body_index(bx, by, route_step, body_offset, body_bits.len());
                dct::write_bit(image, bx, by, body_bits[index], strength);
            }
        }
    }
}

fn collect_header_bits(image: &RgbaImage) -> Result<Vec<bool>> {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let mut ones = vec![0_usize; HEADER_BITS];
    let mut totals = vec![0_usize; HEADER_BITS];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if is_header_block(bx, by) {
                let index = header_index(bx, by);
                totals[index] += 1;
                ones[index] += usize::from(dct::read_bit(image, bx, by));
            }
        }
    }
    if totals.contains(&0) {
        return Err(CoreError::HeaderNotFound);
    }
    Ok(ones
        .into_iter()
        .zip(totals)
        .map(|(one, total)| one * 2 >= total)
        .collect())
}

fn collect_body_bits(
    image: &RgbaImage,
    bit_len: usize,
    route_step: u16,
    salt: &[u8; 16],
) -> Result<Vec<bool>> {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let body_offset = salt_offset(salt, bit_len);
    let mut ones = vec![0_usize; bit_len];
    let mut totals = vec![0_usize; bit_len];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if !is_header_block(bx, by) {
                let index = body_index(bx, by, route_step, body_offset, bit_len);
                totals[index] += 1;
                ones[index] += usize::from(dct::read_bit(image, bx, by));
            }
        }
    }
    if totals.contains(&0) {
        return Err(CoreError::PayloadCorrupted);
    }
    Ok(ones
        .into_iter()
        .zip(totals)
        .map(|(one, total)| one * 2 >= total)
        .collect())
}

fn count_body_blocks(width: u32, height: u32) -> usize {
    let blocks_x = width / BLOCK_SIZE;
    let blocks_y = height / BLOCK_SIZE;
    let mut count = 0_usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            count += usize::from(!is_header_block(bx, by));
        }
    }
    count
}

fn select_route(
    width: u32,
    height: u32,
    body_bit_len: usize,
    salt: &[u8; 16],
) -> Result<(u16, usize, usize)> {
    let blocks_x = width / BLOCK_SIZE;
    let blocks_y = height / BLOCK_SIZE;
    let total_blocks = (blocks_x * blocks_y) as usize;
    let required_blocks = HEADER_BITS * 2 + body_bit_len;
    if total_blocks < required_blocks {
        return Err(CoreError::InsufficientCapacity {
            required_blocks,
            available_blocks: total_blocks,
        });
    }

    let header_counts = route_counts(blocks_x, blocks_y, HEADER_BITS, None, 0);
    let header_min = *header_counts.iter().min().unwrap_or(&0);
    if header_min < 2 {
        return Err(CoreError::InsufficientCapacity {
            required_blocks,
            available_blocks: total_blocks,
        });
    }

    let offset = salt_offset(salt, body_bit_len);
    for step in ROUTE_STEPS {
        let body_counts = route_counts(blocks_x, blocks_y, body_bit_len, Some(step), offset);
        let body_min = *body_counts.iter().min().unwrap_or(&0);
        if body_min >= 1 {
            debug!(step, body_min, "找到满足容量要求的载荷路由");
            return Ok((step, header_min, body_min));
        }
    }

    Err(CoreError::InsufficientCapacity {
        required_blocks,
        available_blocks: total_blocks,
    })
}

fn route_counts(
    blocks_x: u32,
    blocks_y: u32,
    bit_len: usize,
    route_step: Option<u16>,
    offset: usize,
) -> Vec<usize> {
    let mut counts = vec![0_usize; bit_len];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            match route_step {
                None if is_header_block(bx, by) => counts[header_index(bx, by)] += 1,
                Some(step) if !is_header_block(bx, by) => {
                    counts[body_index(bx, by, step, offset, bit_len)] += 1
                }
                _ => {}
            }
        }
    }
    counts
}

#[inline]
fn is_header_block(bx: u32, by: u32) -> bool {
    (bx + by * 3).is_multiple_of(HEADER_MODULUS)
}

#[inline]
fn header_index(bx: u32, by: u32) -> usize {
    (bx as usize * 17 + by as usize * 29) % HEADER_BITS
}

#[inline]
fn body_index(bx: u32, by: u32, step: u16, offset: usize, bit_len: usize) -> usize {
    (bx as usize + by as usize * step as usize + offset) % bit_len
}

fn salt_offset(salt: &[u8; 16], modulus: usize) -> usize {
    let seed = u64::from_le_bytes(salt[..8].try_into().unwrap())
        ^ u64::from_le_bytes(salt[8..].try_into().unwrap());
    (seed as usize) % modulus.max(1)
}

fn classify_integrity(distance: u32, dimensions_changed: bool) -> IntegrityStatus {
    if dimensions_changed || distance >= 14 {
        IntegrityStatus::Modified
    } else if distance >= 8 {
        IntegrityStatus::Uncertain
    } else {
        IntegrityStatus::Intact
    }
}

fn calculate_psnr(original: &RgbaImage, modified: &RgbaImage) -> f64 {
    let mut squared_error = 0_f64;
    let mut samples = 0_u64;
    for (left, right) in original.pixels().zip(modified.pixels()) {
        for channel in 0..3 {
            let delta = left[channel] as f64 - right[channel] as f64;
            squared_error += delta * delta;
            samples += 1;
        }
    }
    if squared_error == 0.0 {
        return f64::INFINITY;
    }
    let mse = squared_error / samples as f64;
    10.0 * ((255.0 * 255.0) / mse).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::HEADER_LEN;

    #[test]
    fn header_has_expected_length() {
        assert_eq!(HEADER_LEN * 8, HEADER_BITS);
    }
}
