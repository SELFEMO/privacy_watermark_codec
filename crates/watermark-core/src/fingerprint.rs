use image::{imageops::FilterType, DynamicImage};

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

pub fn hamming_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}
