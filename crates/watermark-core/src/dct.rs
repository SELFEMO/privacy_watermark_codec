use std::sync::OnceLock;

use image::RgbaImage;

const BLOCK: usize = 8;
const COEFF_A: (usize, usize) = (2, 3);
const COEFF_B: (usize, usize) = (3, 2);

pub fn read_bit(image: &RgbaImage, block_x: u32, block_y: u32) -> bool {
    let block = read_luma_block(image, block_x, block_y);
    let coeffs = forward_dct(&block);
    coeffs[COEFF_A.1][COEFF_A.0] >= coeffs[COEFF_B.1][COEFF_B.0]
}

pub fn write_bit(image: &mut RgbaImage, block_x: u32, block_y: u32, bit: bool, strength: f32) {
    let original_luma = read_luma_block(image, block_x, block_y);
    let mut coeffs = forward_dct(&original_luma);
    let a = coeffs[COEFF_A.1][COEFF_A.0];
    let b = coeffs[COEFF_B.1][COEFF_B.0];
    let difference = a - b;

    // 通过调整一对中频系数的相对大小编码比特，可以避开低频可见失真和高频压缩损失。
    // Encoding in the ordering of two mid-frequency coefficients avoids visible low-frequency distortion and fragile high frequencies.
    let target_difference = if bit { strength } else { -strength };
    if (bit && difference < strength) || (!bit && difference > -strength) {
        let correction = (target_difference - difference) / 2.0;
        coeffs[COEFF_A.1][COEFF_A.0] += correction;
        coeffs[COEFF_B.1][COEFF_B.0] -= correction;
    }

    let modified_luma = inverse_dct(&coeffs);
    apply_luma_delta(image, block_x, block_y, &original_luma, &modified_luma);
}

fn read_luma_block(image: &RgbaImage, block_x: u32, block_y: u32) -> [[f32; BLOCK]; BLOCK] {
    let mut block = [[0_f32; BLOCK]; BLOCK];
    let start_x = block_x * BLOCK as u32;
    let start_y = block_y * BLOCK as u32;

    // 使用行、列迭代器直接访问目标元素，既消除不必要的范围索引，也避免改变像素遍历顺序。
    // Iterating over rows and cells directly removes needless range indexing without changing pixel traversal order.
    for (y, row) in block.iter_mut().enumerate() {
        for (x, luma) in row.iter_mut().enumerate() {
            let pixel = image.get_pixel(start_x + x as u32, start_y + y as u32);
            let r = pixel[0] as f32;
            let g = pixel[1] as f32;
            let b = pixel[2] as f32;
            *luma = 0.299 * r + 0.587 * g + 0.114 * b;
        }
    }
    block
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

    // 同步遍历原始与修改后的亮度块，可保证每个增量严格作用于相同坐标。
    // Zipping the original and modified luma blocks guarantees that each delta is applied to the matching coordinate.
    for (y, (original_row, modified_row)) in original.iter().zip(modified.iter()).enumerate() {
        for (x, (&original_luma, &modified_luma)) in
            original_row.iter().zip(modified_row.iter()).enumerate()
        {
            let px = start_x + x as u32;
            let py = start_y + y as u32;
            let pixel = image.get_pixel_mut(px, py);
            let delta = modified_luma - original_luma;

            // 同量调整 RGB 三通道可近似只改变亮度，并尽量保持原有色度不变。
            // Applying the same delta to RGB changes luminance while approximately preserving the original chroma.
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as f32 + delta).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

fn forward_dct(input: &[[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    let transform = transform_matrix();
    let mut horizontal = [[0_f32; BLOCK]; BLOCK];
    let mut output = [[0_f32; BLOCK]; BLOCK];

    // 可分离二维 DCT 将复杂度从每块 4096 次乘加降到约 1024 次，视频逐帧处理才具有可用性能。
    // The separable 2-D DCT cuts each block from roughly 4096 multiply-adds to about 1024, making frame-by-frame video practical.
    for (input_row, horizontal_row) in input.iter().zip(horizontal.iter_mut()) {
        for (u, horizontal_value) in horizontal_row.iter_mut().enumerate() {
            *horizontal_value = input_row
                .iter()
                .zip(transform[u].iter())
                .map(|(&sample, &basis)| (sample - 128.0) * basis)
                .sum();
        }
    }

    for (v, output_row) in output.iter_mut().enumerate() {
        for (u, output_value) in output_row.iter_mut().enumerate() {
            *output_value = 0.25
                * transform[v]
                    .iter()
                    .zip(horizontal.iter())
                    .map(|(&basis, horizontal_row)| basis * horizontal_row[u])
                    .sum::<f32>();
        }
    }
    output
}

fn inverse_dct(input: &[[f32; BLOCK]; BLOCK]) -> [[f32; BLOCK]; BLOCK] {
    let transform = transform_matrix();
    let mut vertical = [[0_f32; BLOCK]; BLOCK];
    let mut output = [[0_f32; BLOCK]; BLOCK];

    for (y, vertical_row) in vertical.iter_mut().enumerate() {
        for (u, vertical_value) in vertical_row.iter_mut().enumerate() {
            *vertical_value = transform
                .iter()
                .zip(input.iter())
                .map(|(transform_row, input_row)| transform_row[y] * input_row[u])
                .sum();
        }
    }

    for (y, output_row) in output.iter_mut().enumerate() {
        for (x, output_value) in output_row.iter_mut().enumerate() {
            *output_value = 0.25
                * transform
                    .iter()
                    .zip(vertical[y].iter())
                    .map(|(transform_row, &vertical_value)| transform_row[x] * vertical_value)
                    .sum::<f32>()
                + 128.0;
        }
    }
    output
}

fn transform_matrix() -> &'static [[f32; BLOCK]; BLOCK] {
    static MATRIX: OnceLock<[[f32; BLOCK]; BLOCK]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let mut matrix = [[0_f32; BLOCK]; BLOCK];

        // 直接遍历矩阵元素可以满足 Clippy 的严格检查，同时保留 frequency/position 的数学含义。
        // Iterating over matrix cells directly satisfies strict Clippy checks while preserving the frequency/position semantics.
        for (frequency, row) in matrix.iter_mut().enumerate() {
            for (position, value) in row.iter_mut().enumerate() {
                *value = alpha(frequency) * cosine(position, frequency);
            }
        }
        matrix
    })
}

#[inline]
fn alpha(index: usize) -> f32 {
    if index == 0 {
        1.0 / 2_f32.sqrt()
    } else {
        1.0
    }
}

#[inline]
fn cosine(position: usize, frequency: usize) -> f32 {
    (((2 * position + 1) * frequency) as f32 * std::f32::consts::PI / 16.0).cos()
}
