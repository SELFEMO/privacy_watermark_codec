use image::{ImageBuffer, Rgba};
use tempfile::TempDir;
use watermark_core::{
    embed_image_file, extract_image_file, EmbedOptions, IntegrityStatus, KeyMode, KeySource,
    WatermarkKey,
};

#[test]
fn image_roundtrip_extracts_text_and_keeps_psnr() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.png");
    let output = temp.path().join("output.png");
    let image = ImageBuffer::from_fn(1024, 768, |x, y| {
        let noise = ((x.wrapping_mul(37) ^ y.wrapping_mul(91)) & 31) as u8;
        Rgba([
            ((x * 255 / 1023) as u8).saturating_add(noise / 3),
            ((y * 255 / 767) as u8).saturating_add(noise / 4),
            (((x + y) * 255 / 1790) as u8).saturating_add(noise / 5),
            255,
        ])
    });
    image.save(&input).unwrap();

    let key = WatermarkKey::random(KeyMode::Independent);
    let report = embed_image_file(
        &input,
        &output,
        &EmbedOptions {
            text: "版权测试 / Copyright test".into(),
            key: key.clone(),
            strength: 8.0,
            media_kind: "image".into(),
        },
    )
    .unwrap();
    assert!(report.psnr >= 40.0);

    let extracted = extract_image_file(&output, &KeySource::KeyFile(key.to_key_file())).unwrap();
    assert_eq!(extracted.text, "版权测试 / Copyright test");
    assert_eq!(extracted.integrity, IntegrityStatus::Intact);
}
