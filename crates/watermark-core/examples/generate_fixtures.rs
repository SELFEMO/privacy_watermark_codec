use image::{Rgba, RgbaImage};
use watermark_core::{embed_image_file_with_auto_strength, EmbedOptions, KeyMode, WatermarkKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let destination = std::env::args()
        .nth(1)
        .ok_or("usage: generate_fixtures OUTPUT_DIRECTORY")?;
    let destination = std::path::PathBuf::from(destination);
    std::fs::create_dir_all(&destination)?;
    let key = WatermarkKey {
        mode: KeyMode::Independent,
        salt: [19; 16],
        derived_key: [71; 32],
        iterations: 210_000,
    };
    for (width, height) in [(640, 480), (960, 1280), (1920, 1080), (1024, 768)] {
        let temp = tempfile::TempDir::new()?;
        let input = temp.path().join("source.png");
        RgbaImage::from_fn(width, height, |x, y| {
            let xf = x as f32 / width as f32;
            let yf = y as f32 / height as f32;
            Rgba([
                (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                255,
            ])
        })
        .save(&input)?;
        // 合成夹具只生成一次并固定保存密文；回归不得通过重新生成随机密文来掩盖失败。
        // Synthetic fixtures are generated once and retain their ciphertext; regressions must not regenerate random ciphertext to mask failures.
        let report = embed_image_file_with_auto_strength(
            &input,
            destination.join(format!("{width}x{height}.png")),
            &EmbedOptions {
                text: "版权测试 / Copyright test".into(),
                key: key.clone(),
                strength: 8.0,
                media_kind: "image".into(),
                temporal: None,
            },
        )?;
        println!(
            "{width}x{height}: psnr={:.6}, body_votes={}",
            report.psnr, report.body_min_votes
        );
    }
    Ok(())
}
