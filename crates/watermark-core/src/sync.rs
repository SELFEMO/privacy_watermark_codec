use image::{imageops, imageops::FilterType, DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::dct;

const BLOCK_SIZE: u32 = 8;
const SYNC_RADIUS: i32 = 1;
const SCALE_CANDIDATES: [f32; 9] = [1.0, 1.125, 1.25, 1.333_333_4, 1.5, 2.0, 0.888_888_9, 0.8, 0.666_666_7];
const ROTATION_CANDIDATES: [u16; 4] = [0, 90, 180, 270];
const SYNC_PATTERNS: [[bool; 9]; 9] = [
    [true, true, false, true, false, false, true, true, true],
    [true, false, true, false, true, true, false, false, true],
    [false, true, true, true, true, false, false, true, false],
    [true, false, false, true, true, false, false, true, true],
    [false, true, false, true, true, true, false, true, false],
    [true, true, false, false, true, false, true, false, true],
    [false, true, true, false, false, true, true, true, false],
    [true, false, true, true, false, true, false, true, false],
    [false, false, true, true, true, false, true, false, true],
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRegistration {
    pub rotation_degrees: u16,
    pub scale: f32,
    pub score: f32,
}

impl SyncRegistration {
    pub fn identity(score: f32) -> Self {
        Self {
            rotation_degrees: 0,
            scale: 1.0,
            score,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegistrationCandidate {
    pub image: DynamicImage,
    pub registration: SyncRegistration,
}

pub fn embed_template(image: &mut RgbaImage, strength: f32) {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if let Some(bit) = sync_bit(bx, by, blocks_x, blocks_y) {
                // 同步模板使用固定、非密钥图案，是为了在解密前仍能估计旋转和缩放候选，而不会泄露水印正文。
                // The sync template is fixed and non-secret so rotation/scale candidates can be estimated before decryption without exposing payload text.
                dct::write_bit(image, bx, by, bit, strength.max(10.0));
            }
        }
    }
}

pub fn is_sync_block(bx: u32, by: u32, blocks_x: u32, blocks_y: u32) -> bool {
    sync_bit(bx, by, blocks_x, blocks_y).is_some()
}

pub fn sync_score(image: &RgbaImage) -> f32 {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let mut matches = 0_usize;
    let mut total = 0_usize;

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if let Some(expected) = sync_bit(bx, by, blocks_x, blocks_y) {
                total += 1;
                matches += usize::from(dct::read_bit(image, bx, by) == expected);
            }
        }
    }

    if total == 0 { 0.0 } else { matches as f32 / total as f32 }
}

pub fn registration_candidates(image: &DynamicImage) -> Vec<RegistrationCandidate> {
    let mut candidates = Vec::new();
    for rotation in ROTATION_CANDIDATES {
        let rotated = rotate_image(image, rotation);
        for scale in SCALE_CANDIDATES {
            let candidate = scale_image(&rotated, scale);
            if candidate.width() < 256 || candidate.height() < 256 {
                continue;
            }
            let score = sync_score(&candidate.to_rgba8());
            candidates.push(RegistrationCandidate {
                image: candidate,
                registration: SyncRegistration {
                    rotation_degrees: rotation,
                    scale,
                    score,
                },
            });
        }
    }

    // 先尝试同步模板得分最高的候选，可以在旋转或等比例缩放后减少盲目穷举造成的解码耗时。
    // Trying the highest sync-template score first reduces blind decoding cost after rotation or isotropic scaling.
    candidates.sort_by(|left, right| {
        right
            .registration
            .score
            .partial_cmp(&left.registration.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

fn rotate_image(image: &DynamicImage, degrees: u16) -> DynamicImage {
    let rgba = image.to_rgba8();
    match degrees {
        90 => DynamicImage::ImageRgba8(imageops::rotate90(&rgba)),
        180 => DynamicImage::ImageRgba8(imageops::rotate180(&rgba)),
        270 => DynamicImage::ImageRgba8(imageops::rotate270(&rgba)),
        _ => DynamicImage::ImageRgba8(rgba),
    }
}

fn scale_image(image: &DynamicImage, scale: f32) -> DynamicImage {
    if (scale - 1.0).abs() < f32::EPSILON {
        return image.clone();
    }
    let width = ((image.width() as f32) * scale).round().clamp(1.0, u32::MAX as f32) as u32;
    let height = ((image.height() as f32) * scale).round().clamp(1.0, u32::MAX as f32) as u32;
    image.resize_exact(width, height, FilterType::CatmullRom)
}

fn sync_bit(bx: u32, by: u32, blocks_x: u32, blocks_y: u32) -> Option<bool> {
    let anchors = anchor_points(blocks_x, blocks_y);
    for (anchor_index, (cx, cy)) in anchors.iter().enumerate() {
        let dx = bx as i32 - *cx as i32;
        let dy = by as i32 - *cy as i32;
        if dx.abs() <= SYNC_RADIUS && dy.abs() <= SYNC_RADIUS {
            let pattern_index = ((dy + SYNC_RADIUS) * 3 + dx + SYNC_RADIUS) as usize;
            return Some(SYNC_PATTERNS[anchor_index][pattern_index]);
        }
    }
    None
}

fn anchor_points(blocks_x: u32, blocks_y: u32) -> [(u32, u32); 9] {
    let left = 4_u32.min(blocks_x.saturating_sub(1));
    let top = 4_u32.min(blocks_y.saturating_sub(1));
    let right = blocks_x.saturating_sub(5).max(left);
    let bottom = blocks_y.saturating_sub(5).max(top);
    let center_x = blocks_x / 2;
    let center_y = blocks_y / 2;
    [
        (left, top),
        (right, top),
        (left, bottom),
        (right, bottom),
        (center_x, center_y),
        (center_x, top),
        (center_x, bottom),
        (left, center_y),
        (right, center_y),
    ]
}
