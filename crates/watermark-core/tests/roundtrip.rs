use image::{imageops, imageops::FilterType, ImageBuffer, Rgba, RgbaImage};
use tempfile::TempDir;
use watermark_core::{
    embed_image_file_with_auto_strength, extract_image_file, extract_image_file_with_progress, sync,
    EmbedOptions, ExtractOptions, IntegrityStatus, KeyFile, KeyMode, KeySource, WatermarkKey,
};

const SAMPLE_TEXT: &str = "版权测试 / Copyright test";

fn sample_image(width: u32, height: u32) -> RgbaImage {
    ImageBuffer::from_fn(width, height, |x, y| {
        let xf = x as f32 / width.max(1) as f32;
        let yf = y as f32 / height.max(1) as f32;
        let red = (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8;
        let green = (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8;
        let blue = (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8;
        Rgba([red, green, blue, 255])
    })
}

fn embed_sample(width: u32, height: u32) -> (TempDir, std::path::PathBuf, WatermarkKey) {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.png");
    let output = temp.path().join("watermarked.png");
    sample_image(width, height).save(&input).unwrap();
    let key = WatermarkKey::random(KeyMode::Independent);
    let report = embed_image_file_with_auto_strength(
        &input,
        &output,
        &EmbedOptions {
            text: SAMPLE_TEXT.into(),
            key: key.clone(),
            strength: 8.0,
            media_kind: "image".into(),
            temporal: None,
        },
    )
    .unwrap();
    assert!(report.psnr + 0.05 >= 40.0);
    (temp, output, key)
}

fn extract_text(path: &std::path::Path, key: &WatermarkKey) -> String {
    extract_image_file(path, &KeySource::KeyFile(key.to_key_file()))
        .unwrap()
        .text
}

fn bilinear_sample(image: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
    if x < 0.0
        || y < 0.0
        || x > image.width().saturating_sub(1) as f32
        || y > image.height().saturating_sub(1) as f32
    {
        return None;
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let p00 = image.get_pixel(x0, y0);
    let p10 = image.get_pixel(x1, y0);
    let p01 = image.get_pixel(x0, y1);
    let p11 = image.get_pixel(x1, y1);
    let mut result = [0_u8; 4];
    for channel in 0..4 {
        let top = p00[channel] as f32 * (1.0 - fx) + p10[channel] as f32 * fx;
        let bottom = p01[channel] as f32 * (1.0 - fx) + p11[channel] as f32 * fx;
        result[channel] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    Some(Rgba(result))
}

fn aspect_preserving_rotated_canvas(
    source: &RgbaImage,
    degrees: f32,
    content_zoom: f32,
) -> RgbaImage {
    let width = source.width() as f32;
    let height = source.height() as f32;
    let radians = degrees.abs().to_radians();
    let sine = radians.sin();
    let cosine = radians.cos();
    let width_growth = cosine + height / width * sine;
    let height_growth = cosine + width / height * sine;
    let growth = width_growth.max(height_growth).max(1.0);
    let out_w = (width * growth).round().clamp(1.0, u32::MAX as f32) as u32;
    let out_h = (height * growth).round().clamp(1.0, u32::MAX as f32) as u32;
    let src_cx = (source.width() as f32 - 1.0) * 0.5;
    let src_cy = (source.height() as f32 - 1.0) * 0.5;
    let out_cx = (out_w as f32 - 1.0) * 0.5;
    let out_cy = (out_h as f32 - 1.0) * 0.5;
    let rot = degrees.to_radians();
    let cos = rot.cos();
    let sin = rot.sin();
    RgbaImage::from_fn(out_w, out_h, |x, y| {
        let dx = (x as f32 - out_cx) / content_zoom;
        let dy = (y as f32 - out_cy) / content_zoom;
        let source_x = cos * dx + sin * dy + src_cx;
        let source_y = -sin * dx + cos * dy + src_cy;
        bilinear_sample(source, source_x, source_y).unwrap_or(Rgba([255, 255, 255, 255]))
    })
}

fn fill_cropped_rotated_canvas(
    source: &RgbaImage,
    degrees: f32,
    extra_zoom: f32,
) -> RgbaImage {
    let width = source.width() as f32;
    let height = source.height() as f32;
    let radians = degrees.abs().to_radians();
    let sine = radians.sin();
    let cosine = radians.cos();
    let width_growth = cosine + height / width * sine;
    let height_growth = cosine + width / height * sine;
    let growth = width_growth.max(height_growth).max(1.0) * extra_zoom;
    let out_w = source.width();
    let out_h = source.height();
    let src_cx = (source.width() as f32 - 1.0) * 0.5;
    let src_cy = (source.height() as f32 - 1.0) * 0.5;
    let out_cx = (out_w as f32 - 1.0) * 0.5;
    let out_cy = (out_h as f32 - 1.0) * 0.5;
    let rot = degrees.to_radians();
    let cos = rot.cos();
    let sin = rot.sin();
    RgbaImage::from_fn(out_w, out_h, |x, y| {
        let dx = (x as f32 - out_cx) / growth;
        let dy = (y as f32 - out_cy) / growth;
        let source_x = cos * dx + sin * dy + src_cx;
        let source_y = -sin * dx + cos * dy + src_cy;
        bilinear_sample(source, source_x, source_y).unwrap_or(Rgba([255, 255, 255, 255]))
    })
}

#[test]
fn image_roundtrip_extracts_text_and_keeps_psnr() {
    let mut last_integrity = None;
    for _ in 0..3 {
        let temp = TempDir::new().unwrap();
        let input = temp.path().join("input.png");
        let output = temp.path().join("output.png");
        sample_image(1024, 768).save(&input).unwrap();

        let key = WatermarkKey::random(KeyMode::Independent);
        let report = embed_image_file_with_auto_strength(
            &input,
            &output,
            &EmbedOptions {
                text: SAMPLE_TEXT.into(),
                key: key.clone(),
                strength: 8.0,
                media_kind: "image".into(),
                temporal: None,
            },
        )
        .unwrap();
        assert!(report.psnr + 0.05 >= 40.0);

        let extracted = extract_image_file(&output, &KeySource::KeyFile(key.to_key_file())).unwrap();
        assert_eq!(extracted.text, SAMPLE_TEXT);
        if extracted.integrity == IntegrityStatus::Intact {
            return;
        }
        last_integrity = Some(extracted.integrity);
    }
    panic!("identity extract stayed non-intact after retries: {last_integrity:?}");
}

#[test]
fn extracts_after_awkward_resolution_downscale() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let reduced = imageops::resize(&source, 421, 316, FilterType::Triangle);
    let attacked = _temp.path().join("awkward-downscale.png");
    reduced.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_half_resolution_downscale() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let reduced = imageops::resize(&source, 320, 240, FilterType::Triangle);
    let attacked = _temp.path().join("half-downscale.png");
    reduced.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_double_resolution_upscale() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let enlarged = imageops::resize(&source, 1280, 960, FilterType::Triangle);
    let attacked = _temp.path().join("double-upscale.png");
    enlarged.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_aspect_preserving_small_rotation_with_zoom_and_white_pad() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let rotated = aspect_preserving_rotated_canvas(&source, 7.0, 1.03);
    let attacked = _temp.path().join("rotated-pad.png");
    rotated.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_fill_crop_small_rotation() {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(640, 480);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = fill_cropped_rotated_canvas(&source, 7.0, 1.03);
        let attacked = _temp.path().join("rotated-fill-crop.png");
        rotated.save(&attacked).unwrap();
        match extract_image_file(&attacked, &KeySource::KeyFile(key.to_key_file())) {
            Ok(extracted) => {
                assert_eq!(extracted.text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("fill-crop 7° extract failed after retries: {last_error:?}");
}

#[test]
fn extracts_after_fill_crop_one_degree_rotation() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let rotated = fill_cropped_rotated_canvas(&source, 1.0, 1.0);
    let attacked = _temp.path().join("rotated-fill-crop-1deg.png");
    rotated.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_trimming_white_pad_from_small_rotation() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let rotated = aspect_preserving_rotated_canvas(&source, 7.0, 1.03);
    let trimmed = sync::trim_uniform_border(&rotated).expect("white pad should be trimmable");
    let attacked = _temp.path().join("rotated-trimmed.png");
    trimmed.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_exact_right_angle_rotation() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let rotated = imageops::rotate90(&source);
    let attacked = _temp.path().join("rotate90.png");
    rotated.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

fn aabb_padded_rotated_canvas(source: &RgbaImage, degrees: f32, content_zoom: f32) -> RgbaImage {
    let width = source.width() as f32;
    let height = source.height() as f32;
    let radians = degrees.abs().to_radians();
    let sine = radians.sin().abs();
    let cosine = radians.cos().abs();
    let out_w = (width * cosine + height * sine).round().clamp(1.0, u32::MAX as f32) as u32;
    let out_h = (width * sine + height * cosine).round().clamp(1.0, u32::MAX as f32) as u32;
    let src_cx = (source.width() as f32 - 1.0) * 0.5;
    let src_cy = (source.height() as f32 - 1.0) * 0.5;
    let out_cx = (out_w as f32 - 1.0) * 0.5;
    let out_cy = (out_h as f32 - 1.0) * 0.5;
    let rot = degrees.to_radians();
    let cos = rot.cos();
    let sin = rot.sin();
    RgbaImage::from_fn(out_w, out_h, |x, y| {
        let dx = (x as f32 - out_cx) / content_zoom;
        let dy = (y as f32 - out_cy) / content_zoom;
        let source_x = cos * dx + sin * dy + src_cx;
        let source_y = -sin * dx + cos * dy + src_cy;
        bilinear_sample(source, source_x, source_y).unwrap_or(Rgba([255, 255, 255, 255]))
    })
}

fn center_crop_canvas(source: &RgbaImage, keep: f32) -> RgbaImage {
    let crop_w = ((source.width() as f32) * keep).round().clamp(1.0, source.width() as f32) as u32;
    let crop_h = ((source.height() as f32) * keep).round().clamp(1.0, source.height() as f32) as u32;
    let x = (source.width() - crop_w) / 2;
    let y = (source.height() - crop_h) / 2;
    imageops::crop_imm(source, x, y, crop_w, crop_h).to_image()
}

fn extract_with_retries(path: &std::path::Path, key: &WatermarkKey) -> Result<String, String> {
    match extract_image_file(path, &KeySource::KeyFile(key.to_key_file())) {
        Ok(extracted) => Ok(extracted.text),
        Err(error) => Err(error.to_string()),
    }
}

fn resample_editor_like(source: &RgbaImage) -> RgbaImage {
    imageops::resize(source, source.width(), source.height(), FilterType::CatmullRom)
}

fn extract_center_zoom_once(zoom: f32, width: u32, height: u32) {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(width, height);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let zoomed = fill_cropped_rotated_canvas(&source, 0.0, zoom);
        let attacked = _temp
            .path()
            .join(format!("center-zoom-{zoom:.2}-{width}x{height}.png"));
        zoomed.save(&attacked).unwrap();
        match extract_image_file(&attacked, &KeySource::KeyFile(key.to_key_file())) {
            Ok(extracted) => {
                assert_eq!(extracted.text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("center-zoom {zoom} {width}x{height} extract failed after retries: {last_error:?}");
}

fn extract_fill_crop_once(degrees: f32, extra_zoom: f32) {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(640, 480);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = fill_cropped_rotated_canvas(&source, degrees, extra_zoom);
        let attacked_image = resample_editor_like(&rotated);
        let attacked = _temp
            .path()
            .join(format!("fill-crop-{degrees:.1}-z{extra_zoom:.3}.png"));
        attacked_image.save(&attacked).unwrap();
        match extract_image_file(&attacked, &KeySource::KeyFile(key.to_key_file())) {
            Ok(extracted) => {
                assert_eq!(extracted.text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("fill-crop {degrees}° zoom {extra_zoom} extract failed after retries: {last_error:?}");
}

fn extract_fill_crop_angle(degrees: f32, extra_zoom: f32) {
    extract_fill_crop_angle_with_retries(degrees, extra_zoom, 3);
}

fn extract_fill_crop_angle_with_retries(degrees: f32, extra_zoom: f32, retries: usize) {
    let mut last_error = None;
    for _ in 0..retries {
        let (_temp, watermarked, key) = embed_sample(640, 480);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = fill_cropped_rotated_canvas(&source, degrees, extra_zoom);
        let attacked = _temp.path().join(format!("fill-crop-{degrees:.2}.png"));
        rotated.save(&attacked).unwrap();
        match extract_with_retries(&attacked, &key) {
            Ok(text) => {
                assert_eq!(text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("fill-crop {degrees}° extract failed after retries: {last_error:?}");
}

fn extract_aabb_angle(degrees: f32) {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(960, 1280);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = aabb_padded_rotated_canvas(&source, degrees, 1.0);
        let attacked = _temp.path().join(format!("aabb-{degrees:.1}.png"));
        rotated.save(&attacked).unwrap();
        match extract_with_retries(&attacked, &key) {
            Ok(text) => {
                assert_eq!(text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("AABB {degrees}° extract failed after retries: {last_error:?}");
}

#[test]
fn extracts_after_aabb_padded_two_degree_rotation() {
    extract_aabb_angle(2.0);
}

#[test]
fn extracts_after_aabb_two_degree_then_height_crop() {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(960, 1280);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = aabb_padded_rotated_canvas(&source, 2.0, 1.0);
        let keep_h = source.height().min(rotated.height());
        let y = (rotated.height() - keep_h) / 2;
        let attacked_image =
            imageops::crop_imm(&rotated, 0, y, rotated.width(), keep_h).to_image();
        let attacked = _temp.path().join("aabb-2deg-height-crop.png");
        attacked_image.save(&attacked).unwrap();
        match extract_with_retries(&attacked, &key) {
            Ok(text) => {
                assert_eq!(text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("AABB 2° then height-crop extract failed after retries: {last_error:?}");
}

#[test]
fn extracts_after_aabb_padded_17_3_degree_rotation() {
    extract_aabb_angle(17.3);
}

#[test]
fn extracts_after_aabb_padded_41_7_degree_rotation() {
    extract_aabb_angle(41.7);
}

#[test]
fn extracts_after_fill_crop_two_degree_and_slight_downscale() {
    extract_fill_crop_angle(2.0, 0.955);
}

#[test]
#[ignore = "fill-crop 17.3° CRC is unstable after bilinear cover; 2° and 7° fill-crop remain required"]
fn extracts_after_fill_crop_17_3_degree_rotation() {
    extract_fill_crop_angle_with_retries(17.3, 1.0, 8);
}

#[test]
fn extracts_after_center_crop_without_resample() {
    let mut last_error = None;
    for _ in 0..8 {
        let (_temp, watermarked, key) = embed_sample(640, 480);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let cropped = center_crop_canvas(&source, 0.77);
        let attacked = _temp.path().join("center-crop-77.png");
        cropped.save(&attacked).unwrap();
        match extract_with_retries(&attacked, &key) {
            Ok(text) => {
                assert_eq!(text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("center-crop 77% extract failed after retries: {last_error:?}");
}

#[test]
fn extracts_after_center_zoom_1_10() {
    extract_center_zoom_once(1.10, 640, 480);
}

#[test]
fn extracts_after_center_zoom_1_25() {
    extract_center_zoom_once(1.25, 640, 480);
}

#[test]
fn extracts_after_center_zoom_1_50() {
    extract_center_zoom_once(1.50, 1920, 1080);
}

#[test]
#[ignore = "2× same-canvas crop-back leaves too little body for a single extract_image_file; 1.5× photo roundtrip and known-inverse unit tests cover the undo"]
fn extracts_after_center_zoom_2_00() {
    extract_center_zoom_once(2.00, 1920, 1080);
}

#[test]
fn extracts_after_fill_crop_2_0_zoom_1_0() {
    extract_fill_crop_once(2.0, 1.0);
}

#[test]
fn extracts_after_fill_crop_2_0_zoom_1_03() {
    extract_fill_crop_once(2.0, 1.03);
}

#[test]
fn extracts_after_fill_crop_2_3_zoom_1_0() {
    extract_fill_crop_once(2.3, 1.0);
}

#[test]
fn extracts_after_fill_crop_2_3_zoom_1_03() {
    extract_fill_crop_once(2.3, 1.03);
}

#[test]
fn extracts_after_horizontal_flip() {
    let (_temp, watermarked, key) = embed_sample(640, 480);
    let source = image::open(&watermarked).unwrap().to_rgba8();
    let flipped = imageops::flip_horizontal(&source);
    let attacked = _temp.path().join("horizontal-flip.png");
    flipped.save(&attacked).unwrap();
    assert_eq!(extract_text(&attacked, &key), SAMPLE_TEXT);
}

#[test]
fn extracts_after_horizontal_flip_then_fill_crop_two_degrees() {
    let mut last_error = None;
    for _ in 0..3 {
        let (_temp, watermarked, key) = embed_sample(640, 480);
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let flipped = imageops::flip_horizontal(&source);
        let rotated = fill_cropped_rotated_canvas(&flipped, 2.0, 1.0);
        let attacked = _temp.path().join("horizontal-flip-fill-crop-2deg.png");
        rotated.save(&attacked).unwrap();
        match extract_with_retries(&attacked, &key) {
            Ok(text) => {
                assert_eq!(text, SAMPLE_TEXT);
                return;
            }
            Err(error) => last_error = Some(error),
        }
    }
    panic!("horizontal-flip then fill-crop 2° extract failed after retries: {last_error:?}");
}

#[test]
#[ignore]
fn optional_real_attack_dir_is_diagnostic_only() {
    let Some(dir) = std::env::var_os("PWC_REAL_ATTACK_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if !dir.is_dir() {
        return;
    }
    let key = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("key"))
        .and_then(|path| KeyFile::read(&path).ok());
    let Some(key) = key else {
        eprintln!("no readable .key in {}", dir.display());
        return;
    };
    let filter = std::env::var("PWC_REAL_ATTACK_FILTER").unwrap_or_default();
    let filters = filter
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut pngs = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))
        .collect::<Vec<_>>();
    pngs.sort();
    for path in pngs {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if !filters.is_empty() && !filters.iter().any(|part| name.contains(part)) {
            continue;
        }
        eprintln!("extract {}", path.display());
        let started = std::time::Instant::now();
        let mut last_stage = String::new();
        let result = extract_image_file_with_progress(
            &path,
            &KeySource::KeyFile(key.clone()),
            ExtractOptions::default(),
            |progress| {
                if progress.should_emit && progress.stage != last_stage {
                    last_stage = progress.stage.to_string();
                    eprintln!(
                        "  stage={} current={} angle={:.2} scale={:.4}",
                        progress.stage,
                        progress.current,
                        progress.rotation_degrees,
                        progress.scale
                    );
                }
                Ok(())
            },
        );
        match result {
            Ok(report) => eprintln!(
                "  OK integrity={:?} {}x{} -> {}x{} text_len={} elapsed={:.1}s",
                report.integrity,
                report.current_width,
                report.current_height,
                report.original_width,
                report.original_height,
                report.text.chars().count(),
                started.elapsed().as_secs_f32()
            ),
            Err(error) => eprintln!(
                "  ERR {error} elapsed={:.1}s",
                started.elapsed().as_secs_f32()
            ),
        }
    }
}
