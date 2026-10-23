use image::RgbaImage;

use crate::sync::AffineTransform;

const BLOCK: usize = 8;
pub const QIM_PROFILE_LEGACY: u8 = 0;
pub const QIM_PROFILE_PYRAMID: u8 = 1;
const LEGACY_COEFF_A: (usize, usize) = (5, 1);
const LEGACY_COEFF_B: (usize, usize) = (1, 5);
const PYRAMID_COEFF_A: (usize, usize) = (1, 0);
const PYRAMID_COEFF_B: (usize, usize) = (0, 1);
const PYRAMID_STEP_MULTIPLIER: f32 = 0.58;

#[derive(Debug, Clone, Copy)]
pub struct BitObservation {
    pub bit: bool,
    pub confidence: f32,
}

pub fn read_bit(image: &RgbaImage, block_x: u32, block_y: u32, step: f32) -> BitObservation {
    read_bit_with_profile(image, block_x, block_y, step, QIM_PROFILE_LEGACY)
}

pub fn read_bit_with_profile(
    image: &RgbaImage,
    block_x: u32,
    block_y: u32,
    step: f32,
    profile: u8,
) -> BitObservation {
    // 不能仅凭块接近纯白或纯黑就把它当作旋转补边，因为正常照片、JPEG 压缩和缩放重采样都可能产生这类有效载荷块；补边应在几何层按边界连通性处理。
    // A block must not be treated as rotation padding merely because it is nearly white or black, since normal photos, JPEG compression, and resize resampling can all produce valid payload blocks of that kind; padding belongs in the geometry layer where boundary connectivity is known.
    observe_luma_block(read_luma_block(image, block_x, block_y), step, profile)
}

pub(crate) fn read_transformed_bit(
    image: &RgbaImage,
    transform: &AffineTransform,
    pixel_x: u32,
    pixel_y: u32,
    step: f32,
    profile: u8,
) -> Option<BitObservation> {
    if pixel_x.checked_add(BLOCK as u32)? > transform.output_width()
        || pixel_y.checked_add(BLOCK as u32)? > transform.output_height()
    {
        return None;
    }
    let mut block = [[0.0_f32; BLOCK]; BLOCK];
    for (y, row) in block.iter_mut().enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            *value = transform.sample_luma(
                image,
                (pixel_x + x as u32) as f32,
                (pixel_y + y as u32) as f32,
            )?;
        }
    }
    Some(observe_luma_block(block, step, profile))
}

pub fn write_bit(
    image: &mut RgbaImage,
    block_x: u32,
    block_y: u32,
    bit: bool,
    step: f32,
) {
    write_bit_with_profile(image, block_x, block_y, bit, step, QIM_PROFILE_LEGACY);
}

pub fn write_bit_with_profile(
    image: &mut RgbaImage,
    block_x: u32,
    block_y: u32,
    bit: bool,
    step: f32,
    profile: u8,
) {
    let original = read_luma_block(image, block_x, block_y);
    let mut coeffs = forward_profile(original, profile);
    let (coeff_a, coeff_b) = coefficient_pair(profile);
    let difference = coeffs[coeff_a.1][coeff_a.0] - coeffs[coeff_b.1][coeff_b.0];
    let step = effective_step(step, profile);
    let normalized = difference / step;
    let rounded = normalized.round() as i32;
    let wanted_parity = i32::from(bit);
    let target_integer = if rounded.rem_euclid(2) == wanted_parity {
        rounded
    } else {
        let lower = rounded - 1;
        let upper = rounded + 1;
        if (normalized - lower as f32).abs() <= (normalized - upper as f32).abs() {
            lower
        } else {
            upper
        }
    };
    let target_difference = target_integer as f32 * step;
    let correction = (target_difference - difference) / 2.0;

    // 在配置所选的一对方向平衡系数上执行奇偶 QIM，是为了让新媒体使用粗尺度方向细节抵抗旋转和重采样，同时仍能按原一级高频配置读取历史媒体。
    // Applying parity QIM to a profile-selected balanced directional pair lets new media use coarse-scale details against rotation and resampling while preserving the original one-level high-frequency reader for historical media.
    coeffs[coeff_a.1][coeff_a.0] += correction;
    coeffs[coeff_b.1][coeff_b.0] -= correction;

    let modified = inverse_profile(coeffs, profile);
    apply_luma_delta(image, block_x, block_y, &original, &modified);
}

fn observe_luma_block(
    block: [[f32; BLOCK]; BLOCK],
    step: f32,
    profile: u8,
) -> BitObservation {
    let coeffs = forward_profile(block, profile);
    let (coeff_a, coeff_b) = coefficient_pair(profile);
    let difference = coeffs[coeff_a.1][coeff_a.0] - coeffs[coeff_b.1][coeff_b.0];
    let normalized = difference / effective_step(step, profile);
    let nearest = normalized.round();
    let bit = (nearest as i32).rem_euclid(2) == 1;
    let confidence = (1.0 - (normalized - nearest).abs() * 2.0).clamp(0.05, 1.0);
    BitObservation { bit, confidence }
}

fn effective_step(step: f32, profile: u8) -> f32 {
    if profile == QIM_PROFILE_PYRAMID {
        // 0.58 而不是更大的乘数，是因为公开头仍用固定 HEADER_STEP、自动降强度只降正文；过大的金字塔步长会让典型照片卡在 40 dB 门槛之下无法保存。
        // 0.58 rather than a larger multiplier because the public header still uses a fixed HEADER_STEP while auto-backoff only reduces body strength; a larger pyramid step leaves typical photos below the 40 dB save gate.
        (step * PYRAMID_STEP_MULTIPLIER).max(1.0)
    } else {
        step.max(1.0)
    }
}

fn coefficient_pair(profile: u8) -> ((usize, usize), (usize, usize)) {
    if profile == QIM_PROFILE_PYRAMID {
        // 最粗尺度的水平与垂直细节在缩放、插值和轻微旋转后仍保留稳定方向能量，因此比原一级高频载波更适合几何恢复。
        // Coarsest-scale horizontal and vertical details retain stable directional energy after resizing, interpolation, and small rotations, making them better suited to geometric recovery than the original one-level high-frequency carrier.
        (PYRAMID_COEFF_A, PYRAMID_COEFF_B)
    } else {
        (LEGACY_COEFF_A, LEGACY_COEFF_B)
    }
}

fn forward_profile(
    values: [[f32; BLOCK]; BLOCK],
    profile: u8,
) -> [[f32; BLOCK]; BLOCK] {
    if profile == QIM_PROFILE_PYRAMID {
        forward_haar_pyramid(values)
    } else {
        forward_haar_level(values)
    }
}

fn inverse_profile(
    values: [[f32; BLOCK]; BLOCK],
    profile: u8,
) -> [[f32; BLOCK]; BLOCK] {
    if profile == QIM_PROFILE_PYRAMID {
        inverse_haar_pyramid(values)
    } else {
        inverse_haar_level(values)
    }
}

fn read_luma_block(image: &RgbaImage, block_x: u32, block_y: u32) -> [[f32; BLOCK]; BLOCK] {
    let mut block = [[0.0_f32; BLOCK]; BLOCK];
    let start_x = block_x * BLOCK as u32;
    let start_y = block_y * BLOCK as u32;
    for (y, row) in block.iter_mut().enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            let pixel = image.get_pixel(start_x + x as u32, start_y + y as u32);
            *value = 0.299 * pixel[0] as f32
                + 0.587 * pixel[1] as f32
                + 0.114 * pixel[2] as f32;
        }
    }
    block
}

fn forward_haar_level(mut values: [[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    forward_haar_region(&mut values, BLOCK);
    values
}

fn inverse_haar_level(mut values: [[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    inverse_haar_region(&mut values, BLOCK);
    values
}

fn forward_haar_pyramid(mut values: [[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    let mut size = BLOCK;
    while size >= 2 {
        forward_haar_region(&mut values, size);
        size /= 2;
    }
    values
}

fn inverse_haar_pyramid(mut values: [[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    let mut size = 2;
    while size <= BLOCK {
        inverse_haar_region(&mut values, size);
        size *= 2;
    }
    values
}

fn forward_haar_region(values: &mut [[f32; BLOCK]; BLOCK], size: usize) {
    let half = size / 2;
    let mut row_pass = *values;
    for y in 0..size {
        for pair in 0..half {
            let left = values[y][pair * 2];
            let right = values[y][pair * 2 + 1];
            row_pass[y][pair] = (left + right) * 0.5;
            row_pass[y][pair + half] = (left - right) * 0.5;
        }
    }
    for x in 0..size {
        for pair in 0..half {
            let top = row_pass[pair * 2][x];
            let bottom = row_pass[pair * 2 + 1][x];
            values[pair][x] = (top + bottom) * 0.5;
            values[pair + half][x] = (top - bottom) * 0.5;
        }
    }
}

fn inverse_haar_region(values: &mut [[f32; BLOCK]; BLOCK], size: usize) {
    let half = size / 2;
    let mut column_pass = *values;
    for x in 0..size {
        for pair in 0..half {
            let average = values[pair][x];
            let detail = values[pair + half][x];
            column_pass[pair * 2][x] = average + detail;
            column_pass[pair * 2 + 1][x] = average - detail;
        }
    }
    for y in 0..size {
        for pair in 0..half {
            let average = column_pass[y][pair];
            let detail = column_pass[y][pair + half];
            values[y][pair * 2] = average + detail;
            values[y][pair * 2 + 1] = average - detail;
        }
    }
}

fn apply_luma_delta(
    image: &mut RgbaImage,
    block_x: u32,
    block_y: u32,
    original: &[[f32; BLOCK]; BLOCK],
    modified: &[[f32; BLOCK]; BLOCK],
) {
    let start_x = block_x * BLOCK as u32;
    let start_y = block_y * BLOCK as u32;
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let pixel = image.get_pixel_mut(start_x + x as u32, start_y + y as u32);
            let delta = modified[y][x] - original[y][x];
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as f32 + delta)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{imageops::FilterType, Rgba, RgbaImage};

    #[test]
    fn legacy_qim_round_trip_recovers_both_bits() {
        let mut image = RgbaImage::from_pixel(16, 8, Rgba([110, 130, 150, 255]));
        write_bit(&mut image, 0, 0, false, 12.0);
        write_bit(&mut image, 1, 0, true, 12.0);
        assert!(!read_bit(&image, 0, 0, 12.0).bit);
        assert!(read_bit(&image, 1, 0, 12.0).bit);
    }

    #[test]
    fn pyramid_profile_round_trip_recovers_both_bits() {
        let mut image = RgbaImage::from_pixel(16, 8, Rgba([110, 130, 150, 255]));
        write_bit_with_profile(&mut image, 0, 0, false, 12.0, QIM_PROFILE_PYRAMID);
        write_bit_with_profile(&mut image, 1, 0, true, 12.0, QIM_PROFILE_PYRAMID);
        assert!(!read_bit_with_profile(&image, 0, 0, 12.0, QIM_PROFILE_PYRAMID).bit);
        assert!(read_bit_with_profile(&image, 1, 0, 12.0, QIM_PROFILE_PYRAMID).bit);
    }

    #[test]
    fn pyramid_profile_survives_half_scale_round_trip() {
        let mut image = RgbaImage::from_fn(128, 128, |x, y| {
            Rgba([
                ((x + y) % 200 + 20) as u8,
                ((x * 2) % 180 + 30) as u8,
                ((y * 2) % 180 + 30) as u8,
                255,
            ])
        });
        for block_y in 0..16 {
            for block_x in 0..16 {
                write_bit_with_profile(
                    &mut image,
                    block_x,
                    block_y,
                    (block_x + block_y) % 2 == 1,
                    16.0,
                    QIM_PROFILE_PYRAMID,
                );
            }
        }
        let reduced = image::imageops::resize(&image, 64, 64, FilterType::Triangle);
        let restored = image::imageops::resize(&reduced, 128, 128, FilterType::Triangle);
        let mut correct = 0_usize;
        for block_y in 0..16 {
            for block_x in 0..16 {
                let expected = (block_x + block_y) % 2 == 1;
                let actual = read_bit_with_profile(
                    &restored,
                    block_x,
                    block_y,
                    16.0,
                    QIM_PROFILE_PYRAMID,
                )
                .bit;
                correct += usize::from(actual == expected);
            }
        }
        // 通过缩小再恢复的测试保护新载波的核心目的：粗尺度方向细节应在常见分辨率重采样后保留大多数可判定位，而不是只在无损原图上往返成功。
        // The downscale-and-restore test protects the new carrier's purpose: coarse directional details should retain most decidable bits after common resolution resampling rather than only round-tripping on an untouched image.
        assert!(correct >= 230);
    }

    #[test]
    fn pyramid_transform_round_trip_preserves_samples() {
        let mut block = [[0.0_f32; BLOCK]; BLOCK];
        for (y, row) in block.iter_mut().enumerate() {
            for (x, value) in row.iter_mut().enumerate() {
                *value = (x * 13 + y * 17) as f32;
            }
        }
        let restored = inverse_haar_pyramid(forward_haar_pyramid(block));
        for y in 0..BLOCK {
            for x in 0..BLOCK {
                assert!((restored[y][x] - block[y][x]).abs() < 0.001);
            }
        }
    }
}
