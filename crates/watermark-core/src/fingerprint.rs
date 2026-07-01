use image::{imageops::FilterType, DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};

pub const PARTITION_GRID: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionFingerprint {
    pub index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub hash_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TamperRegion {
    pub index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub distance: u32,
    pub status: String,
}

pub fn difference_hash(image: &DynamicImage) -> u64 {
    let gray = image.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut hash = 0_u64;
    let mut bit = 0_u32;
    for y in 0..8 {
        for x in 0..8 {
            if gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0] {
                hash |= 1_u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

pub fn partition_fingerprints(image: &DynamicImage) -> Vec<PartitionFingerprint> {
    let mut regions = Vec::with_capacity((PARTITION_GRID * PARTITION_GRID) as usize);
    let width = image.width();
    let height = image.height();

    for gy in 0..PARTITION_GRID {
        for gx in 0..PARTITION_GRID {
            let x = gx * width / PARTITION_GRID;
            let y = gy * height / PARTITION_GRID;
            let next_x = (gx + 1) * width / PARTITION_GRID;
            let next_y = (gy + 1) * height / PARTITION_GRID;
            let region_width = next_x.saturating_sub(x).max(1);
            let region_height = next_y.saturating_sub(y).max(1);
            let crop = image.view(x, y, region_width, region_height).to_image();
            let hash = difference_hash(&DynamicImage::ImageRgba8(crop));
            regions.push(PartitionFingerprint {
                index: regions.len(),
                x,
                y,
                width: region_width,
                height: region_height,
                hash_hex: format!("{hash:016x}"),
            });
        }
    }

    regions
}

pub fn compare_partitions(
    original: &[PartitionFingerprint],
    current: &[PartitionFingerprint],
) -> Vec<TamperRegion> {
    let mut regions = Vec::new();
    for (left, right) in original.iter().zip(current.iter()) {
        let Ok(original_hash) = u64::from_str_radix(&left.hash_hex, 16) else {
            continue;
        };
        let Ok(current_hash) = u64::from_str_radix(&right.hash_hex, 16) else {
            continue;
        };
        let distance = hamming_distance(original_hash, current_hash);
        let status = if distance >= 14 {
            "modified"
        } else if distance >= 8 {
            "uncertain"
        } else {
            "intact"
        };
        if status != "intact" {
            // 分区指纹只保存感知哈希而非原图内容，因此可用于定位疑似篡改区域，同时避免在水印载荷中泄露局部图像数据。
            // Partition fingerprints store perceptual hashes instead of pixels, so they can localize suspicious edits without leaking local image content.
            regions.push(TamperRegion {
                index: left.index,
                x: right.x,
                y: right.y,
                width: right.width,
                height: right.height,
                distance,
                status: status.into(),
            });
        }
    }
    regions
}

pub fn hamming_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}
