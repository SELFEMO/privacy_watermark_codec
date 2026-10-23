use std::{
    cell::Cell,
    cmp::Ordering,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering},
        mpsc, OnceLock,
    },
    thread,
    time::Duration,
};

use image::{imageops, DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    bch,
    crypto::{derive_key, DEFAULT_PBKDF2_ITERATIONS},
    dct, dwt_qim,
    error::{CoreError, Result},
    fingerprint::{
        compare_partitions, difference_hash, hamming_distance, partition_fingerprints,
        PartitionFingerprint, TamperRegion,
    },
    keyfile::{KeySource, WatermarkKey},
    payload::{
        bits_to_bytes, bytes_to_bits, create_encrypted_body, open_encrypted_body, robust_flags,
        InnerPayload, LegacyHeader, RobustHeader, TemporalPayload, LEGACY_HEADER_BITS,
        ROBUST_HEADER_BITS, ROBUST_MAGIC, ROBUST_PILOT,
    },
    sync::{self, SyncRegistration},
};

const BLOCK_SIZE: u32 = 8;
const HEADER_PHASE_MULTIPLIER_X: usize = 1;
const HEADER_PHASE_MULTIPLIER_Y: usize = 31;
const HEADER_STEP: f32 = 16.0;
const LEGACY_HEADER_MODULUS: u32 = 5;
const MIN_WIDTH: u32 = 256;
const MIN_HEIGHT: u32 = 256;
const MIN_EXTRACT_SIDE: u32 = 128;
const MAX_INPUT_PIXELS: u64 = 64_000_000;
const MIN_PSNR_DB: f64 = 40.0;
const PSNR_ACCEPT_EPSILON_DB: f64 = 0.05;
const ORIGIN_MARGIN_BLOCKS: i32 = 64;
const MAX_ORIGIN_CANDIDATES: usize = 512;
const PARTITION_SAMPLE_ORDER: [usize; 16] = [0, 3, 12, 15, 5, 6, 9, 10, 1, 2, 4, 7, 8, 11, 13, 14];
const BODY_VOTE_REGION_COUNT: usize = 4;
const BCH_CODEWORD_BITS: usize = 31;
const HEADER_REPAIR_WEAK_BITS: usize = 14;
const HEADER_REPAIR_MAX_FLIPS: usize = 4;
const HEADER_SIGNATURE_MIN_SCORE: f32 = 0.78;
const HEADER_ADAPTIVE_STEP_MIN_SCORE: f32 = 0.66;
const HEADER_DAMAGE_EVIDENCE_MIN_SCORE: f32 = 0.82;
#[allow(dead_code)]
const HEADER_DAMAGE_IDENTITY_MIN_SCORE: f32 = 0.20;
#[allow(dead_code)]
const HEADER_DAMAGE_GEOMETRY_MIN_GAIN: f32 = 0.08;
const GEOMETRY_MAX_ANGLE_DEGREES: f32 = 180.0;
const GEOMETRY_PROGRESS_TOTAL: usize = 1024;
const GEOMETRY_STAGE_DECODE_CANDIDATES: usize = 3;
const GEOMETRY_MIN_DECODE_SCORE: f32 = 0.014;
const GEOMETRY_RESIZE_DECODE_MIN_SCORE: f32 = 0.004;
#[allow(dead_code)]
const GEOMETRY_DEEP_SEARCH_MIN_SCORE: f32 = 0.007;
const GEOMETRY_RELATIVE_DECODE_SCORE: f32 = 0.58;
const GEOMETRY_STRONG_DECODE_SCORE: f32 = 0.12;
const GEOMETRY_MAX_WORKERS: usize = 8;

#[derive(Debug, Clone)]
pub struct TemporalContext {
    pub clip_id: [u8; 16],
    pub frame_index: u32,
    pub frame_count: u32,
}

#[derive(Debug, Clone)]
pub struct EmbedOptions {
    pub text: String,
    pub key: WatermarkKey,
    pub strength: f32,
    pub media_kind: String,
    pub temporal: Option<TemporalContext>,
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
    pub sync_score: f32,
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
    pub tamper_regions: Vec<TamperRegion>,
    pub sync_registration: SyncRegistration,
    pub temporal: Option<TemporalPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicWatermarkHeader {
    pub algorithm: String,
    pub salt_hex: String,
    pub body_len: u32,
    pub route_step: u32,
    pub strength: f32,
    pub original_width: Option<u32>,
    pub original_height: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractOptions {
    pub allow_registration: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct GeometrySearchProgress {
    pub current: usize,
    pub total: usize,
    pub rotation_degrees: f32,
    pub scale: f32,
    pub stage: &'static str,
    pub workers: usize,
    // 评分阶段大量内部检查点只用于及时取消；验证阶段才把可见进度和子进度发给界面，避免 1025/1025 之后进度条假死。
    // Scoring checkpoints mostly exist for timely cancellation; only the verify stage emits visible progress and sub-progress so the bar cannot freeze after 1025/1025.
    pub should_emit: bool,
    pub sub_progress: f32,
}

impl GeometrySearchProgress {
    fn scoring(
        current: usize,
        rotation_degrees: f32,
        scale: f32,
        stage: &'static str,
        workers: usize,
        should_emit: bool,
    ) -> Self {
        Self {
            current: current.min(GEOMETRY_PROGRESS_TOTAL + 1),
            total: GEOMETRY_PROGRESS_TOTAL + 1,
            rotation_degrees,
            scale,
            stage,
            workers,
            should_emit,
            sub_progress: 0.0,
        }
    }

    fn verify(
        current: usize,
        total: usize,
        rotation_degrees: f32,
        scale: f32,
        should_emit: bool,
        sub_progress: f32,
    ) -> Self {
        Self {
            current,
            total: total.max(1),
            rotation_degrees,
            scale,
            stage: "verify",
            workers: 1,
            should_emit,
            sub_progress: sub_progress.clamp(0.0, 1.0),
        }
    }
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            allow_registration: true,
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedPayload {
    payload: InnerPayload,
    body: Vec<u8>,
    body_bits: Vec<bool>,
    route_step: usize,
    body_min_votes: usize,
}

#[derive(Debug, Clone)]
struct HeaderCandidate {
    header: RobustHeader,
    pixel_phase_x: u32,
    pixel_phase_y: u32,
    phase_delta: usize,
    step_factor: f32,
    score: f32,
}

#[derive(Debug, Clone, Copy)]
struct OriginCandidate {
    block_x: i32,
    block_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeometryLayout {
    Standard,
    ExpandedCanvas,
    FixedCanvasFit,
    TightBounds,
    AabbPadded,
    AabbHeightKept,
    AabbWidthKept,
    CenterZoom,
}

#[derive(Debug, Clone, Copy)]
struct GeometryCandidate {
    rotation_degrees: f32,
    scale: f32,
    layout: GeometryLayout,
    score: f32,
}

fn affine_for_geometry(
    source: &RgbaImage,
    rotation_degrees: f32,
    scale: f32,
    layout: GeometryLayout,
) -> Option<sync::AffineTransform> {
    match layout {
        GeometryLayout::Standard => {
            sync::AffineTransform::new(source.width(), source.height(), rotation_degrees, scale)
        }
        GeometryLayout::ExpandedCanvas => sync::AffineTransform::new_expanded_canvas_correction(
            source.width(),
            source.height(),
            rotation_degrees,
            scale,
        ),
        GeometryLayout::FixedCanvasFit => {
            sync::AffineTransform::new_fixed_canvas_fit_correction(
                source.width(),
                source.height(),
                rotation_degrees,
                scale,
            )
        }
        GeometryLayout::TightBounds => sync::AffineTransform::new_tight_bounds_correction(
            source.width(),
            source.height(),
            rotation_degrees,
            scale,
        ),
        GeometryLayout::AabbPadded => sync::AffineTransform::new_aabb_padded_correction(
            source.width(),
            source.height(),
            rotation_degrees,
            scale,
        ),
        GeometryLayout::AabbHeightKept => sync::AffineTransform::new_aabb_axis_cropped_correction(
            source.width(),
            source.height(),
            rotation_degrees,
            scale,
            true,
        ),
        GeometryLayout::AabbWidthKept => sync::AffineTransform::new_aabb_axis_cropped_correction(
            source.width(),
            source.height(),
            rotation_degrees,
            scale,
            false,
        ),
        GeometryLayout::CenterZoom => sync::AffineTransform::new_center_zoom_correction(
            source.width(),
            source.height(),
            scale,
        ),
    }
}

enum GeometryWorkerMessage {
    Started(f32, f32),
    Finished(GeometryCandidate),
    Done,
}

#[derive(Debug)]
struct RobustDecoded {
    payload: InnerPayload,
    corrected_codewords: usize,
    registration: SyncRegistration,
    normalized: DynamicImage,
}

#[derive(Debug)]
struct BodyVoteCandidate {
    bits: Vec<bool>,
    reliabilities: Vec<f32>,
    observed: Vec<bool>,
    score: f32,
    region_mask: u8,
}

pub fn embed_image_file(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &EmbedOptions,
) -> Result<EmbedReport> {
    validate_embed_options(options)?;
    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();
    let original = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    validate_embed_dimensions(&original)?;
    if original.width() > u16::MAX as u32 || original.height() > u16::MAX as u32 {
        return Err(CoreError::InvalidArgument("图像边长不能超过 65535 像素".into()));
    }

    let prepared = prepare_payload(&original, options)?;
    let temporal = prepared.payload.temporal.as_ref();
    let header = RobustHeader {
        salt: options.key.salt,
        body_len: prepared.body.len() as u32,
        original_width: original.width() as u16,
        original_height: original.height() as u16,
        strength_x10: (options.strength * 10.0).round() as u8,
        flags: robust_flags(temporal),
        // 新编码固定使用三级 Haar 金字塔的粗尺度方向细节配置，是因为旧一级高频系数在旋转和分辨率重采样后会快速衰减；配置同时写入公开头以保留可验证的向后兼容读取。
        // New encodes use the three-level Haar-pyramid profile because the legacy high-frequency coefficients decay quickly under rotation and resizing; the public header records the profile for verifiable backward-compatible decoding.
        qim_profile: dwt_qim::QIM_PROFILE_PYRAMID,
    };
    let header_bits = bytes_to_bits(&header.to_bytes());
    let header_min_votes = robust_header_min_votes(
        original.width() / BLOCK_SIZE,
        original.height() / BLOCK_SIZE,
    );
    if header_min_votes == 0 {
        return Err(CoreError::InsufficientCapacity {
            required_blocks: ROBUST_HEADER_BITS,
            available_blocks: (original.width() / BLOCK_SIZE * (original.height() / BLOCK_SIZE)) as usize,
        });
    }

    let original_rgba = original.to_rgba8();
    let mut watermarked = original_rgba.clone();
    embed_robust_bits(
        &mut watermarked,
        &header_bits,
        &prepared.body_bits,
        prepared.route_step,
        &options.key.salt,
        options.strength,
        header.qim_profile,
    );
    let psnr = calculate_psnr(&original_rgba, &watermarked);
    if !psnr_meets_minimum(psnr) {
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
        input = %input_path.display(),
        output = %output_path.display(),
        payload_bytes = prepared.body.len(),
        body_min_votes = prepared.body_min_votes,
        psnr,
        "DWT-QIM 鲁棒水印嵌入完成"
    );
    Ok(EmbedReport {
        output_path: output_path.display().to_string(),
        width: original.width(),
        height: original.height(),
        psnr,
        payload_bytes: prepared.body.len(),
        header_min_votes,
        body_min_votes: prepared.body_min_votes,
        sync_score: 1.0,
    })
}

pub fn embed_image_file_with_auto_strength(
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    options: &EmbedOptions,
) -> Result<EmbedReport> {
    let input_path = input_path.as_ref();
    let output_path = output_path.as_ref();
    let mut strength = options.strength;
    let minimum_strength = auto_strength_floor(&options.media_kind);
    loop {
        let mut attempt = options.clone();
        attempt.strength = strength;
        match embed_image_file(input_path, output_path, &attempt) {
            Ok(report) => return Ok(report),
            Err(error) if is_psnr_retryable(&error) && strength > minimum_strength => {
                // 仅在 PSNR 失败时自动降强度，避免把容量或密钥错误误当作可重试的视觉质量问题。
                // Strength is reduced only for PSNR failures so capacity or key errors are not misclassified as retryable visual-quality issues.
                strength = (strength - 1.0).max(minimum_strength);
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn probe_embedded_header_file(
    input_path: impl AsRef<Path>,
) -> Result<Option<PublicWatermarkHeader>> {
    let input_path = input_path.as_ref();
    let image = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    if image.width() < MIN_EXTRACT_SIDE || image.height() < MIN_EXTRACT_SIDE {
        return Ok(None);
    }
    let rgba = image.to_rgba8();
    let (header_candidates, _) = collect_robust_header_candidates(&rgba, 1, false, false);
    if let Some(candidate) = header_candidates.into_iter().next() {
        let blocks = count_robust_body_blocks(
            candidate.header.original_width as u32 / BLOCK_SIZE,
            candidate.header.original_height as u32 / BLOCK_SIZE,
        );
        return Ok(Some(PublicWatermarkHeader {
            algorithm: if candidate.header.qim_profile == dwt_qim::QIM_PROFILE_PYRAMID {
                "DWT-QIM-pyramid-BCH-balanced-spatial-voting".into()
            } else {
                "DWT-QIM-legacy-BCH-balanced-spatial-voting".into()
            },
            salt_hex: bytes_to_lower_hex(&candidate.header.salt),
            body_len: candidate.header.body_len,
            route_step: select_body_route_step(blocks, &candidate.header.salt) as u32,
            strength: candidate.header.strength_x10 as f32 / 10.0,
            original_width: Some(candidate.header.original_width as u32),
            original_height: Some(candidate.header.original_height as u32),
        }));
    }
    if let Ok(header) = collect_legacy_header(&rgba) {
        return Ok(Some(PublicWatermarkHeader {
            algorithm: "DCT-BCH-sequential-voting".into(),
            salt_hex: bytes_to_lower_hex(&header.salt),
            body_len: header.body_len,
            route_step: header.route_step as u32,
            strength: header.strength_x10 as f32 / 10.0,
            original_width: None,
            original_height: None,
        }));
    }
    Ok(None)
}

pub fn extract_image_file(
    input_path: impl AsRef<Path>,
    key_source: &KeySource,
) -> Result<ExtractReport> {
    extract_image_file_with_options(input_path, key_source, ExtractOptions::default())
}

pub fn extract_image_file_with_options(
    input_path: impl AsRef<Path>,
    key_source: &KeySource,
    options: ExtractOptions,
) -> Result<ExtractReport> {
    extract_image_file_internal(input_path.as_ref(), key_source, options, None)
}

pub fn extract_image_file_with_progress<F>(
    input_path: impl AsRef<Path>,
    key_source: &KeySource,
    options: ExtractOptions,
    mut on_progress: F,
) -> Result<ExtractReport>
where
    F: FnMut(GeometrySearchProgress) -> Result<()>,
{
    extract_image_file_internal(
        input_path.as_ref(),
        key_source,
        options,
        Some(&mut on_progress),
    )
}

fn extract_image_file_internal(
    input_path: &Path,
    key_source: &KeySource,
    options: ExtractOptions,
    mut on_progress: Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
) -> Result<ExtractReport> {
    let image = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    validate_extract_dimensions(&image)?;
    let actual_width = image.width();
    let actual_height = image.height();
    let identity = SyncRegistration::identity(1.0);
    let mut progress_current = 1_usize;
    if let Some(callback) = on_progress.as_deref_mut() {
        // 身份方向的首次检测也可能包含完整水印头扫描，因此把它纳入真实候选进度，避免进入几何搜索前界面长期停在固定百分比。
        // The identity-orientation attempt can include a full header scan, so it is included in real candidate progress to avoid a frozen percentage before geometry search begins.
        callback(GeometrySearchProgress::scoring(
            progress_current,
            0.0,
            1.0,
            "identity",
            1,
            true,
        ))?;
    }

    match extract_registered_image(&image, actual_width, actual_height, key_source, identity) {
        Ok(report) => return Ok(report),
        Err(error @ CoreError::SaltMismatch) => {
            // 公开头 salt 与已载入密钥均不匹配时，任何几何变换都不会产生正确密钥，因此可以安全地立即返回。
            // When the public-header salt matches none of the loaded keys, no geometry transform can produce the correct key, so returning immediately is safe.
            return Err(error);
        }
        Err(first_error) if !options.allow_registration => return Err(first_error),
        Err(first_error) => {
            // 即使公开头在当前尺寸下可读，缩小或重采样也可能只让正文网格失配；受限的分阶段搜索仍应继续，否则会把“头可读、正文需配准”的压缩图片误判为永久损坏。
            // Even when the public header remains readable at the current size, downscaling or resampling can misalign only the body grid; the bounded staged search must continue or compressed images with a readable header but registrable body would be misclassified as permanently damaged.
            let source_rgba = image.to_rgba8();
            let prefer_identity_damage = matches!(&first_error, CoreError::PayloadCorrupted);
            if let Some(report) = search_and_extract_geometry(
                &source_rgba,
                actual_width,
                actual_height,
                key_source,
                prefer_identity_damage,
                &mut on_progress,
                &mut progress_current,
            )? {
                return Ok(report);
            }

            if let Some(callback) = on_progress.as_deref_mut() {
                progress_current = progress_current.saturating_add(1);
                // 左右镜像是单独的反射，不是旋转梯子的一部分；先报阶段再搜，避免进度条在翻转搜索时卡住。
                // Left-right mirroring is a reflection, not part of the rotation ladder; emit the stage before searching so the bar cannot freeze during the flip pass.
                callback(GeometrySearchProgress::scoring(
                    progress_current,
                    0.0,
                    1.0,
                    "horizontal-flip",
                    1,
                    true,
                ))?;
            }
            let flipped = imageops::flip_horizontal(&source_rgba);
            let flipped_dyn = DynamicImage::ImageRgba8(flipped.clone());
            match extract_registered_image(
                &flipped_dyn,
                flipped.width(),
                flipped.height(),
                key_source,
                SyncRegistration::identity(1.0),
            ) {
                Ok(report) => return Ok(report),
                Err(CoreError::SaltMismatch) => return Err(first_error),
                Err(_) => {
                    if let Some(report) = search_and_extract_geometry(
                        &flipped,
                        flipped.width(),
                        flipped.height(),
                        key_source,
                        false,
                        &mut on_progress,
                        &mut progress_current,
                    )? {
                        return Ok(report);
                    }
                }
            }
            Err(first_error)
        }
    }
}

fn extract_registered_image(
    image: &DynamicImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    registration: SyncRegistration,
) -> Result<ExtractReport> {
    match extract_robust(image, key_source, registration) {
        Ok(decoded) => return finalize_report(decoded, actual_width, actual_height),
        Err(error @ CoreError::SaltMismatch) | Err(error @ CoreError::DecryptionFailed) => {
            return Err(error)
        }
        Err(robust_error @ CoreError::PayloadCorrupted) => {
            // 在返回鲁棒载荷损坏前仍尝试一次旧格式，是为了避免 PWW1 历史媒体在 PWW2 软导频上偶然高分时失去原有的兼容读取路径。
            // Trying the legacy format once before returning robust-payload damage preserves PWW1 compatibility when historical media happens to score highly against the PWW2 soft pilot.
            return match extract_legacy(
                image,
                actual_width,
                actual_height,
                key_source,
                registration,
            ) {
                Ok(report) => Ok(report),
                Err(CoreError::HeaderNotFound) => Err(robust_error),
                Err(error) => Err(error),
            };
        }
        Err(CoreError::HeaderNotFound) => {}
        Err(error) => return Err(error),
    }
    extract_legacy(image, actual_width, actual_height, key_source, registration)
}

fn prepare_payload(image: &DynamicImage, options: &EmbedOptions) -> Result<PreparedPayload> {
    let fingerprint = difference_hash(image);
    let all_partitions = if options.media_kind == "video_frame" {
        Vec::new()
    } else {
        partition_fingerprints(image)
    };
    let temporal = options.temporal.as_ref().map(|context| TemporalPayload {
        clip_id_hex: bytes_to_lower_hex(&context.clip_id),
        frame_index: context.frame_index,
        frame_count: context.frame_count,
    });
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let body_blocks = count_robust_body_blocks(blocks_x, blocks_y);
    let mut fallback = None;

    for sample_count in [16_usize, 12, 8, 4, 0] {
        let partitions = sample_partitions(&all_partitions, sample_count);
        let payload = InnerPayload::new(
            options.text.clone(),
            fingerprint,
            image.width(),
            image.height(),
            &options.media_kind,
            partitions,
            temporal.clone(),
        );
        let body = create_encrypted_body(&payload, &options.key)?;
        let body_bits = bch::encode_bytes(&body);
        if body_bits.len() > body_blocks {
            continue;
        }
        let candidate = PreparedPayload {
            payload,
            body,
            body_min_votes: body_blocks / body_bits.len(),
            body_bits,
            route_step: select_body_route_step(body_blocks, &options.key.salt),
        };
        if candidate.body_min_votes >= 3 {
            return Ok(candidate);
        }
        fallback = Some(candidate);
    }

    // 分区指纹按容量逐级缩减，是为了优先保住正文的多重空间观测；遮挡鲁棒性不足时，更多元数据反而会降低可恢复性。
    // Partition fingerprints are reduced with capacity so repeated spatial observations of the body take priority; when occlusion redundancy is scarce, more metadata would reduce recoverability.
    fallback.ok_or(CoreError::InsufficientCapacity {
        required_blocks: 1,
        available_blocks: body_blocks,
    })
}

fn embed_robust_bits(
    image: &mut RgbaImage,
    header_bits: &[bool],
    body_bits: &[bool],
    route_step: usize,
    salt: &[u8; 16],
    strength: f32,
    qim_profile: u8,
) {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let body_blocks = count_robust_body_blocks(blocks_x, blocks_y);
    let offset = salt_offset(salt, body_blocks);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let phase = robust_phase(bx as i32, by as i32);
            if phase % 3 == 0 {
                let index = (phase / 3) % ROBUST_HEADER_BITS;
                dwt_qim::write_bit_with_profile(
                    image,
                    bx,
                    by,
                    header_bits[index],
                    HEADER_STEP,
                    qim_profile,
                );
            } else {
                let rank = robust_body_rank(bx, by, blocks_x);
                let index = robust_body_index(rank, route_step, offset, body_blocks, body_bits.len());
                dwt_qim::write_bit_with_profile(
                    image,
                    bx,
                    by,
                    body_bits[index],
                    strength,
                    qim_profile,
                );
            }
        }
    }
}

fn extract_robust(
    image: &DynamicImage,
    key_source: &KeySource,
    mut registration: SyncRegistration,
) -> Result<RobustDecoded> {
    let rgba = image.to_rgba8();
    let (header_candidates, header_evidence_score) = collect_robust_header_candidates(
        &rgba,
        16,
        !registration.is_identity(),
        // 几何校正图只在零相位导频已经 ≥0.82 时才扫 64 相位；错误候选若无条件全相位会把 7° 铺满卡在头扫描上。
        // Geometry warps run the 64-phase sweep only when the zero-phase pilot is already ≥0.82; unconditional full-phase on false candidates stalls 7° fill-crop in header scanning.
        false,
    );
    if header_candidates.is_empty() {
        // 强导频但 CRC 未通过时先标记为“疑似局部损坏”，把微旋转与纯遮挡的判别统一交给外层并行快速层，可避免在身份解码阶段重复执行一轮串行邻近几何评分。
        // A strong pilot with a failed CRC is marked as suspected localized damage and left to the outer parallel fast layer to distinguish micro-rotation from occlusion, avoiding a duplicated sequential nearby-geometry pass during identity decoding.
        return if header_evidence_score >= HEADER_DAMAGE_EVIDENCE_MIN_SCORE {
            Err(CoreError::PayloadCorrupted)
        } else {
            Err(CoreError::HeaderNotFound)
        };
    }
    let mut last_error = CoreError::PayloadCorrupted;
    let mut credential_error = None;
    for header_candidate in header_candidates {
        let header = &header_candidate.header;
        let body_len = header.body_len as usize;
        let bit_len = bch::encoded_bit_len(body_len);
        let original_blocks_x = header.original_width as u32 / BLOCK_SIZE;
        let original_blocks_y = header.original_height as u32 / BLOCK_SIZE;
        let body_blocks = count_robust_body_blocks(original_blocks_x, original_blocks_y);
        if bit_len > body_blocks || body_len > 65_536 {
            continue;
        }
        let route_step = select_body_route_step(body_blocks, &header.salt);
        let origins = origin_candidates(
            header,
            &header_candidate,
            rgba.width(),
            rgba.height(),
        );
        for origin in origins {
            match collect_and_open_robust_body(
                &rgba,
                &header_candidate,
                origin,
                bit_len,
                route_step,
                body_blocks,
                key_source,
            ) {
                Ok((payload, corrected_codewords)) => {
                    if payload.width != header.original_width as u32
                        || payload.height != header.original_height as u32
                        || payload.temporal.is_some() != header.is_video()
                    {
                        continue;
                    }
                    let translation_x = origin.block_x * BLOCK_SIZE as i32
                        - header_candidate.pixel_phase_x as i32;
                    let translation_y = origin.block_y * BLOCK_SIZE as i32
                        - header_candidate.pixel_phase_y as i32;
                    registration.translation_x = translation_x;
                    registration.translation_y = translation_y;
                    registration.score = registration.score.max(header_candidate.score);
                    let normalized = normalize_to_original_canvas(
                        image,
                        payload.width,
                        payload.height,
                        translation_x,
                        translation_y,
                    );
                    return Ok(RobustDecoded {
                        payload,
                        corrected_codewords,
                        registration,
                        normalized,
                    });
                }
                Err(error @ CoreError::SaltMismatch) | Err(error @ CoreError::DecryptionFailed) => {
                    if credential_error.is_none() {
                        credential_error = Some(error);
                    }
                }
                Err(error) => last_error = error,
            }
        }
    }
    // 区域头修复和多相位扫描可能产生多个通过 CRC 的候选，因此凭据错误必须等全部候选验证后再返回，避免第一个错误 salt 阻断后续真正匹配的密钥。
    // Region-based header repair and multi-phase scanning can yield several CRC-valid candidates, so credential errors are returned only after all candidates are checked, preventing an early wrong salt from blocking a later key match.
    if let Some(error) = credential_error {
        Err(error)
    } else {
        Err(last_error)
    }
}

fn collect_and_open_robust_body(
    image: &RgbaImage,
    candidate: &HeaderCandidate,
    origin: OriginCandidate,
    bit_len: usize,
    route_step: usize,
    body_blocks: usize,
    key_source: &KeySource,
) -> Result<(InnerPayload, usize)> {
    let base_strength = candidate.header.strength_x10 as f32 / 10.0;
    let mut last_error = CoreError::PayloadCorrupted;
    let mut strength_factors = Vec::with_capacity(5);
    for factor in [candidate.step_factor, 1.0_f32, 0.85, 0.70, 1.15] {
        if !strength_factors
            .iter()
            .any(|existing: &f32| (*existing - factor).abs() < 0.01)
        {
            strength_factors.push(factor);
        }
    }
    // 公开头在哪个衰减步长恢复成功，正文通常也经历了相近的频域衰减；优先复用该比例可让 JPEG 压缩图片少做一到三次完整正文扫描。
    // The body usually experiences frequency attenuation close to the step that recovered the public header, so trying that ratio first avoids one to three full body scans for JPEG-compressed images.
    for factor in strength_factors {
        match collect_and_open_robust_body_at_strength(
            image,
            candidate,
            origin,
            bit_len,
            route_step,
            body_blocks,
            key_source,
            base_strength * factor,
        ) {
            Ok(decoded) => return Ok(decoded),
            Err(error @ CoreError::SaltMismatch) => return Err(error),
            Err(error) => last_error = error,
        }
    }
    // 正文量化步长与公开头分开自适应，是为了让头已恢复但正文被压缩衰减的图片仍可利用 BCH 擦除纠正，而不改变已写入文件的格式。
    // Adapting the body quantization step independently from the public header lets BCH erasure recovery handle compressed bodies after the header is found without changing the on-disk format.
    Err(last_error)
}

fn collect_and_open_robust_body_at_strength(
    image: &RgbaImage,
    candidate: &HeaderCandidate,
    origin: OriginCandidate,
    bit_len: usize,
    route_step: usize,
    body_blocks: usize,
    key_source: &KeySource,
    strength: f32,
) -> Result<(InnerPayload, usize)> {
    let header = &candidate.header;
    let blocks_x = (image.width().saturating_sub(candidate.pixel_phase_x)) / BLOCK_SIZE;
    let blocks_y = (image.height().saturating_sub(candidate.pixel_phase_y)) / BLOCK_SIZE;
    let original_blocks_x = header.original_width as u32 / BLOCK_SIZE;
    let original_blocks_y = header.original_height as u32 / BLOCK_SIZE;
    let offset = salt_offset(&header.salt, body_blocks);
    let mut region_signed = vec![0.0_f32; bit_len * BODY_VOTE_REGION_COUNT];
    let mut region_weights = vec![0.0_f32; bit_len * BODY_VOTE_REGION_COUNT];
    for local_y in 0..blocks_y {
        for local_x in 0..blocks_x {
            let absolute_x = local_x as i32 + origin.block_x;
            let absolute_y = local_y as i32 + origin.block_y;
            if absolute_x < 0
                || absolute_y < 0
                || absolute_x >= original_blocks_x as i32
                || absolute_y >= original_blocks_y as i32
                || robust_phase(absolute_x, absolute_y) % 3 == 0
            {
                continue;
            }
            let rank = robust_body_rank(absolute_x as u32, absolute_y as u32, original_blocks_x);
            let index = robust_body_index(rank, route_step, offset, body_blocks, bit_len);
            let pixel_x = candidate.pixel_phase_x + local_x * BLOCK_SIZE;
            let pixel_y = candidate.pixel_phase_y + local_y * BLOCK_SIZE;
            if block_is_solid_geometry_fill(image, pixel_x, pixel_y) {
                // 固定画布逆变换在铺满裁切后会留下大块纯黑/纯白填补；这些块没有载荷，计入投票只会把本来可纠的正文打成认证失败。
                // Fixed-canvas inversion after a fill-crop leaves solid black/white filler; those blocks carry no payload, and counting them in the vote turns an otherwise recoverable body into an authentication failure.
                continue;
            }
            let observation = read_qim_at_pixel(
                image,
                pixel_x,
                pixel_y,
                strength,
                header.qim_profile,
            );
            let confidence = observation.confidence.min(0.85);
            let region_x = ((absolute_x as usize * 2) / original_blocks_x.max(1) as usize).min(1);
            let region_y = ((absolute_y as usize * 2) / original_blocks_y.max(1) as usize).min(1);
            let region = region_y * 2 + region_x;
            let slot = index * BODY_VOTE_REGION_COUNT + region;
            let signed = if observation.bit { confidence } else { -confidence };
            region_signed[slot] += signed;
            region_weights[slot] += confidence;
        }
    }

    const REGION_MASKS: [u8; 15] = [
        0b1111, 0b0111, 0b1011, 0b1101, 0b1110, 0b0011, 0b1100, 0b0101,
        0b1010, 0b0110, 0b1001, 0b0001, 0b0010, 0b0100, 0b1000,
    ];
    let mut vote_candidates: Vec<BodyVoteCandidate> = REGION_MASKS
        .into_iter()
        .filter_map(|region_mask| {
            build_body_vote_candidate(
                &region_signed,
                &region_weights,
                bit_len,
                region_mask,
            )
        })
        .collect();
    vote_candidates.sort_by(|left, right| {
        right
            .region_mask
            .count_ones()
            .cmp(&left.region_mask.count_ones())
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(Ordering::Equal)
            })
    });
    vote_candidates.dedup_by(|left, right| {
        left.bits == right.bits && left.observed == right.observed
    });
    // 先验证全图和三象限共识、失败后才降到半图与单象限，是为了让未遮挡媒体保持一次解码的快速路径，同时把重度遮挡恢复留在按需进入的深层。
    // Full-image and three-quadrant consensus are verified before half- and single-quadrant subsets so clean media keeps a one-decode fast path while severe-occlusion recovery remains an on-demand deeper layer.

    let key = resolve_key(key_source, &header.salt)?;
    let mut last_error = CoreError::PayloadCorrupted;
    for vote_candidate in vote_candidates {
        for max_erasures in [0_usize, 2, 4, 6] {
            let erasures = build_body_erasures(
                &vote_candidate.reliabilities,
                &vote_candidate.observed,
                max_erasures,
            );
            let Some(decoded) = bch::decode_bits_with_erasures(
                &vote_candidate.bits,
                &erasures,
                header.body_len as usize,
            ) else {
                continue;
            };
            match open_encrypted_body(&decoded.bytes, &key) {
                Ok(payload) => return Ok((payload, decoded.corrected_codewords)),
                // 每个空间子集最终都必须通过 AEAD 认证，因此可以安全尝试避开被遮挡象限的票，而不会用“看似合理”的 BCH 输出制造假阳性。
                // Every spatial subset must still pass AEAD authentication, so votes from occluded quadrants can be bypassed safely without turning plausible-looking BCH output into a false positive.
                Err(error) => last_error = error,
            }
        }
    }
    Err(last_error)
}

fn build_body_vote_candidate(
    region_signed: &[f32],
    region_weights: &[f32],
    bit_len: usize,
    region_mask: u8,
) -> Option<BodyVoteCandidate> {
    if region_mask == 0 {
        return None;
    }
    let selected_regions = region_mask.count_ones() as usize;
    let mut bits = Vec::with_capacity(bit_len);
    let mut reliabilities = Vec::with_capacity(bit_len);
    let mut observed = Vec::with_capacity(bit_len);
    let mut reliability_sum = 0.0_f32;
    let mut observed_count = 0_usize;

    for index in 0..bit_len {
        let start = index * BODY_VOTE_REGION_COUNT;
        let mut margins = [0.0_f32; BODY_VOTE_REGION_COUNT];
        let mut margin_count = 0_usize;
        let mut total_weight = 0.0_f32;
        let mut total_signed = 0.0_f32;
        for region in 0..BODY_VOTE_REGION_COUNT {
            if region_mask & (1_u8 << region) == 0 {
                continue;
            }
            let slot = start + region;
            let weight = region_weights[slot];
            total_weight += weight;
            total_signed += region_signed[slot];
            if weight >= 0.1 {
                margins[margin_count] = region_signed[slot] / weight;
                margin_count += 1;
            }
        }
        let margin = if margin_count >= 3 {
            spatial_consensus_margin(&margins[..margin_count])
        } else if total_weight >= 0.1 {
            total_signed / total_weight
        } else {
            0.0
        };
        let is_observed = total_weight >= 0.1;
        let reliability = margin.abs().clamp(0.0, 1.0);
        bits.push(margin >= 0.0);
        reliabilities.push(reliability);
        observed.push(is_observed);
        if is_observed {
            reliability_sum += reliability;
            observed_count += 1;
        }
    }
    if observed_count == 0 {
        return None;
    }
    let coverage = observed_count as f32 / bit_len.max(1) as f32;
    let region_support = 0.72 + selected_regions as f32 * 0.07;
    let score = reliability_sum / observed_count as f32 * coverage.sqrt() * region_support;
    Some(BodyVoteCandidate {
        bits,
        reliabilities,
        observed,
        score,
        region_mask,
    })
}

fn spatial_consensus_margin(margins: &[f32]) -> f32 {
    let mut sorted = margins.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        len if len % 2 == 1 => sorted[len / 2],
        len => (sorted[len / 2 - 1] + sorted[len / 2]) * 0.5,
    }
}

fn build_body_erasures(
    reliabilities: &[f32],
    observed: &[bool],
    max_erasures: usize,
) -> Vec<bool> {
    let mut erasures = vec![false; reliabilities.len()];
    if max_erasures == 0 {
        return erasures;
    }
    for start in (0..reliabilities.len()).step_by(BCH_CODEWORD_BITS) {
        let end = (start + BCH_CODEWORD_BITS).min(reliabilities.len());
        let mut candidates: Vec<usize> = (start..end).collect();
        candidates.sort_by(|left, right| {
            let left_missing = !observed[*left];
            let right_missing = !observed[*right];
            right_missing
                .cmp(&left_missing)
                .then_with(|| {
                    reliabilities[*left]
                        .partial_cmp(&reliabilities[*right])
                        .unwrap_or(Ordering::Equal)
                })
        });
        for index in candidates.into_iter().take(max_erasures) {
            if !observed[index] || reliabilities[index] < 0.34 {
                erasures[index] = true;
            }
        }
    }
    // 将低一致性位按码字限制为最多六个擦除，是为了让遮挡和重采样产生的不可靠票进入 BCH 的擦除通道，同时保留其余错误纠正预算。
    // Limiting low-consensus bits to at most six erasures per codeword routes occlusion and resampling uncertainty into BCH erasure recovery while preserving the remaining error-correction budget.
    erasures
}

fn collect_robust_header_candidates(
    image: &RgbaImage,
    limit: usize,
    skip_solid_fill: bool,
    allow_full_phase: bool,
) -> (Vec<HeaderCandidate>, f32) {
    let zero_phase = [(0_u32, 0_u32)];
    let (fast_candidates, mut best_signature_score) =
        collect_robust_header_candidates_at_step_for_phases(
            image,
            limit,
            HEADER_STEP,
            &zero_phase,
            false,
            skip_solid_fill,
        );
    let cropped_against_header = fast_candidates.iter().any(|candidate| {
        candidate.header.original_width as u32 > image.width()
            || candidate.header.original_height as u32 > image.height()
    });
    if !fast_candidates.is_empty() && !cropped_against_header {
        // 未裁剪的压缩媒体通常仍保持零像素相位，因此先验证这一条路径可避免正常图片重复扫描全部六十四种块起点。
        // Compressed media that was not cropped usually keeps zero pixel phase, so validating this path first avoids rescanning all sixty-four block origins for normal images.
        return (fast_candidates, best_signature_score);
    }
    // 当前图比公开头声明的原图小，零相位 CRC 可能是裁切后的巧合命中；继续邻近相位才能落到真正的块起点。
    // When the current image is smaller than the header's original size, a zero-phase CRC can be a crop coincidence; nearby phases must still run to reach the true block origin.

    if !cropped_against_header && best_signature_score >= HEADER_ADAPTIVE_STEP_MIN_SCORE {
        for factor in [0.85_f32, 0.70, 1.15] {
            let (fallback, score) = collect_robust_header_candidates_at_step_for_phases(
                image,
                limit,
                HEADER_STEP * factor,
                &zero_phase,
                false,
                skip_solid_fill,
            );
            best_signature_score = best_signature_score.max(score);
            if !fallback.is_empty() {
                // JPEG 会改变频域量化间距却通常不会移动块网格，零相位先适配步长可显著缩短压缩图片的常见成功路径。
                // JPEG changes frequency-domain quantization spacing but usually not the block grid, so adapting the step at zero phase substantially shortens the common success path for compressed images.
                return (fallback, best_signature_score);
            }
        }
    }

    let try_nearby_phases = if skip_solid_fill && !cropped_against_header {
        best_signature_score >= HEADER_ADAPTIVE_STEP_MIN_SCORE
    } else {
        // 中心放大再裁回会先物化成比公开头更小的图；几何校正也必须扫邻近相位，不能只留给身份解码。
        // Center-zoom crop-back materializes smaller than the public header; geometry warps must still scan nearby phases instead of leaving that only to identity.
        true
    };
    if try_nearby_phases {
        const NEARBY_PHASES: [(u32, u32); 21] = [
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (0, 2),
            (2, 2),
            (7, 0),
            (0, 7),
            (7, 7),
            (4, 0),
            (0, 4),
            (3, 3),
            (5, 3),
            (5, 0),
            (0, 3),
            (6, 3),
            (5, 4),
            (1, 7),
            (7, 1),
            (2, 6),
            (6, 2),
        ];
        {
            let (nearby, score) = collect_robust_header_candidates_at_step_for_phases(
                image,
                limit,
                HEADER_STEP,
                &NEARBY_PHASES,
                true,
                skip_solid_fill,
            );
            best_signature_score = best_signature_score.max(score);
            if !nearby.is_empty() {
                let mut merged = nearby;
                merged.sort_by_key(|candidate| {
                    let phase = (candidate.pixel_phase_x, candidate.pixel_phase_y);
                    u8::from(phase != (7, 1) && phase != (1, 7))
                });
                if cropped_against_header {
                    for candidate in fast_candidates {
                        if !merged.iter().any(|existing| {
                            existing.pixel_phase_x == candidate.pixel_phase_x
                                && existing.pixel_phase_y == candidate.pixel_phase_y
                        }) {
                            merged.push(candidate);
                        }
                    }
                }
                return (merged, best_signature_score);
            }
        }
    }

    if !allow_full_phase && best_signature_score < HEADER_DAMAGE_EVIDENCE_MIN_SCORE {
        // 身份检测的 64 相位全扫描只在零相位导频已经很像真头时才做；7° 铺满裁切在错误网格上也能摸到 0.66 的巧合分，否则会在进入强制铺满倒数前卡两分钟。
        // Identity's 64-phase sweep runs only when the zero-phase pilot already looks like a real header; a 7° fill-crop can still score 0.66 on the wrong grid, which would stall for minutes before the forced fill-crop reciprocals.
        return (Vec::new(), best_signature_score);
    }

    let (candidates, score) =
        collect_robust_header_candidates_at_step(image, limit, HEADER_STEP, skip_solid_fill);
    best_signature_score = best_signature_score.max(score);
    if !candidates.is_empty() {
        return (candidates, best_signature_score);
    }
    if best_signature_score < HEADER_ADAPTIVE_STEP_MIN_SCORE {
        // 完整相位扫描仍没有接近有效导频时不再扩大步长，是为了让无水印或几何失配图片尽快进入更合适的恢复层。
        // When the full phase scan still lacks near-valid pilot evidence, step expansion stops so unmarked or geometrically misaligned images can move quickly to the appropriate recovery layer.
        return (Vec::new(), best_signature_score);
    }

    for factor in [0.85_f32, 0.70, 1.15] {
        let (fallback, score) = collect_robust_header_candidates_at_step(
            image,
            limit,
            HEADER_STEP * factor,
            skip_solid_fill,
        );
        best_signature_score = best_signature_score.max(score);
        if !fallback.is_empty() {
            return (fallback, best_signature_score);
        }
    }
    (Vec::new(), best_signature_score)
}

fn collect_robust_header_candidates_at_step(
    image: &RgbaImage,
    limit: usize,
    header_step: f32,
    skip_solid_fill: bool,
) -> (Vec<HeaderCandidate>, f32) {
    let mut phases = Vec::with_capacity((BLOCK_SIZE * BLOCK_SIZE) as usize);
    for pixel_phase_y in 0..BLOCK_SIZE.min(image.height()) {
        for pixel_phase_x in 0..BLOCK_SIZE.min(image.width()) {
            phases.push((pixel_phase_x, pixel_phase_y));
        }
    }
    collect_robust_header_candidates_at_step_for_phases(
        image,
        limit,
        header_step,
        &phases,
        true,
        skip_solid_fill,
    )
}

fn collect_robust_header_candidates_at_step_for_phases(
    image: &RgbaImage,
    limit: usize,
    header_step: f32,
    phases: &[(u32, u32)],
    allow_region_subsets: bool,
    skip_solid_fill: bool,
) -> (Vec<HeaderCandidate>, f32) {
    let modulus = ROBUST_HEADER_BITS * 3;
    let mut candidates = Vec::new();
    let mut best_signature_score = 0.0_f32;
    for &(pixel_phase_x, pixel_phase_y) in phases {
            let blocks_x = image.width().saturating_sub(pixel_phase_x) / BLOCK_SIZE;
            let blocks_y = image.height().saturating_sub(pixel_phase_y) / BLOCK_SIZE;
            if blocks_x * blocks_y < ROBUST_HEADER_BITS as u32 {
                continue;
            }
            for qim_profile in [
                dwt_qim::QIM_PROFILE_PYRAMID,
                dwt_qim::QIM_PROFILE_LEGACY,
            ] {
                let mut signed = vec![0.0_f32; modulus];
                let mut weights = vec![0.0_f32; modulus];
                let mut region_signed = vec![0.0_f32; modulus * BODY_VOTE_REGION_COUNT];
                let mut region_weights = vec![0.0_f32; modulus * BODY_VOTE_REGION_COUNT];
                for by in 0..blocks_y {
                    for bx in 0..blocks_x {
                        let pixel_x = pixel_phase_x + bx * BLOCK_SIZE;
                        let pixel_y = pixel_phase_y + by * BLOCK_SIZE;
                        if skip_solid_fill && block_is_solid_geometry_fill(image, pixel_x, pixel_y) {
                            // 仅在几何校正图上跳过纯色填补，是因为身份解码时近白文档的平坦块仍可能带着导频，一律跳过会把完好媒体读成无头。
                            // Skip solid fill only on geometry-corrected images because identity reads of near-white documents can still carry pilot in flat blocks, and skipping them always would turn intact media into a missing header.
                            continue;
                        }
                        let phase = robust_phase(bx as i32, by as i32);
                        let observation = read_qim_at_pixel(
                            image,
                            pixel_x,
                            pixel_y,
                            header_step,
                            qim_profile,
                        );
                        let observation_signed = if observation.bit {
                            observation.confidence
                        } else {
                            -observation.confidence
                        };
                        signed[phase] += observation_signed;
                        weights[phase] += observation.confidence;
                        let region_x = ((bx as usize * 2) / blocks_x.max(1) as usize).min(1);
                        let region_y = ((by as usize * 2) / blocks_y.max(1) as usize).min(1);
                        let region = region_y * 2 + region_x;
                        let region_slot = region * modulus + phase;
                        region_signed[region_slot] += observation_signed;
                        region_weights[region_slot] += observation.confidence;
                    }
                }
                let mut promising_deltas = Vec::with_capacity(modulus);
                for delta in 0..modulus {
                    let mut delta_signature_score = 0.0_f32;
                    for use_spatial_consensus in [false, true] {
                        let Some((bits, reliabilities, confidence_sum)) =
                            build_header_vote_candidate(
                                &signed,
                                &weights,
                                &region_signed,
                                &region_weights,
                                delta,
                                use_spatial_consensus,
                            )
                        else {
                            continue;
                        };
                        let signature_score = robust_header_signature_score(&bits);
                        delta_signature_score = delta_signature_score.max(signature_score);
                        best_signature_score = best_signature_score.max(signature_score);
                        if signature_score < HEADER_SIGNATURE_MIN_SCORE {
                            continue;
                        }
                        if let Some(header) = decode_or_repair_robust_header(&bits, &reliabilities) {
                            if header.qim_profile != qim_profile {
                                continue;
                            }
                            candidates.push(HeaderCandidate {
                                header,
                                pixel_phase_x,
                                pixel_phase_y,
                                phase_delta: delta,
                                step_factor: (header_step / HEADER_STEP).clamp(0.5, 1.5),
                                score: (confidence_sum / ROBUST_HEADER_BITS as f32) * 0.75
                                    + signature_score * 0.25,
                            });
                        }
                    }
                    promising_deltas.push((delta_signature_score, delta));
                }

                let has_promising_region_phase = promising_deltas
                    .iter()
                    .any(|(score, _)| *score >= HEADER_ADAPTIVE_STEP_MIN_SCORE);
                if allow_region_subsets && candidates.is_empty() && has_promising_region_phase {
                    promising_deltas.sort_by(|left, right| {
                        right.0.partial_cmp(&left.0).unwrap_or(Ordering::Equal)
                    });
                    promising_deltas.truncate(6);
                    const HEADER_REGION_MASKS: [u8; 14] = [
                        0b0001, 0b0010, 0b0100, 0b1000, 0b0011, 0b1100, 0b0101,
                        0b1010, 0b0110, 0b1001, 0b0111, 0b1011, 0b1101, 0b1110,
                    ];
                    for (_, delta) in promising_deltas {
                        for region_mask in HEADER_REGION_MASKS {
                            let Some((bits, reliabilities, confidence_sum)) =
                                build_header_region_vote_candidate(
                                    &region_signed,
                                    &region_weights,
                                    delta,
                                    region_mask,
                                )
                            else {
                                continue;
                            };
                            let signature_score = robust_header_signature_score(&bits);
                            best_signature_score = best_signature_score.max(signature_score);
                            if signature_score < HEADER_SIGNATURE_MIN_SCORE {
                                continue;
                            }
                            if let Some(header) = decode_or_repair_robust_header(&bits, &reliabilities) {
                                if header.qim_profile != qim_profile {
                                    continue;
                                }
                                // 仅对最有希望的相位尝试区域子集，是为了在大面积遮挡破坏多数象限时保留幸存载波，同时避免把所有相位乘以所有区域组合而拖慢正常路径。
                                // Region subsets are tried only for the most promising phases so surviving carriers can recover after majority-quadrant occlusion without multiplying every phase by every region combination on the normal path.
                                candidates.push(HeaderCandidate {
                                    header,
                                    pixel_phase_x,
                                    pixel_phase_y,
                                    phase_delta: delta,
                                    step_factor: (header_step / HEADER_STEP).clamp(0.5, 1.5),
                                    score: (confidence_sum / ROBUST_HEADER_BITS as f32) * 0.72
                                        + signature_score * 0.28,
                                });
                            }
                        }
                    }
                }
                if !candidates.is_empty() {
                    // 同时尝试全局加权票与四象限稳健共识，是为了让单块遮挡只污染其所在区域，而不会把整幅图中仍然一致的头位一起拖翻。
                    // Trying both global weighted voting and four-quadrant robust consensus keeps a localized occluder from overturning header bits that remain consistent across the rest of the image.
                    // 先尝试新的三级 Haar 金字塔配置并用头内配置字节交叉验证，是为了快速读取新媒体，同时防止另一套系数上的偶然 CRC 命中被当成真水印。
                    // Trying the new three-level Haar-pyramid profile first and cross-checking the profile byte in the header keeps new-media reads fast while rejecting accidental CRC hits from the other coefficient pair.
                    candidates.sort_by(|left, right| {
                        right
                            .score
                            .partial_cmp(&left.score)
                            .unwrap_or(Ordering::Equal)
                    });
                    candidates.dedup_by(|left, right| {
                        left.header.salt == right.header.salt
                            && left.header.qim_profile == right.header.qim_profile
                            && left.pixel_phase_x == right.pixel_phase_x
                            && left.pixel_phase_y == right.pixel_phase_y
                            && left.phase_delta == right.phase_delta
                    });
                    candidates.truncate(limit);
                    return (candidates, best_signature_score);
                }
            }
        }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    candidates.dedup_by(|left, right| {
        left.header.salt == right.header.salt
            && left.header.qim_profile == right.header.qim_profile
            && left.pixel_phase_x == right.pixel_phase_x
            && left.pixel_phase_y == right.pixel_phase_y
            && left.phase_delta == right.phase_delta
    });
    candidates.truncate(limit);
    (candidates, best_signature_score)
}

fn build_header_vote_candidate(
    signed: &[f32],
    weights: &[f32],
    region_signed: &[f32],
    region_weights: &[f32],
    delta: usize,
    use_spatial_consensus: bool,
) -> Option<(Vec<bool>, Vec<f32>, f32)> {
    let modulus = ROBUST_HEADER_BITS * 3;
    let mut bits = Vec::with_capacity(ROBUST_HEADER_BITS);
    let mut reliabilities = Vec::with_capacity(ROBUST_HEADER_BITS);
    let mut confidence_sum = 0.0_f32;
    for index in 0..ROBUST_HEADER_BITS {
        let local_phase = (index * 3 + modulus - delta) % modulus;
        if weights[local_phase] <= f32::EPSILON {
            // 铺满裁切逆变换会跳过纯色填补，个别头相位可能完全没有票；当成零置信擦除而不是整头作废，CRC 仍会拒绝噪声。
            // Fill-crop inversion skips solid fill, so a header phase can have no votes; treat that as a zero-confidence erasure instead of discarding the whole header, and let CRC reject noise.
            bits.push(false);
            reliabilities.push(0.0);
            continue;
        }
        let global_margin = signed[local_phase] / weights[local_phase];
        let margin = if use_spatial_consensus {
            let mut region_margins = [0.0_f32; BODY_VOTE_REGION_COUNT];
            let mut region_count = 0_usize;
            for region in 0..BODY_VOTE_REGION_COUNT {
                let slot = region * modulus + local_phase;
                if region_weights[slot] >= 0.1 {
                    region_margins[region_count] =
                        region_signed[slot] / region_weights[slot];
                    region_count += 1;
                }
            }
            if region_count >= 3 {
                spatial_consensus_margin(&region_margins[..region_count])
            } else {
                global_margin
            }
        } else {
            global_margin
        };
        let reliability = margin.abs().clamp(0.0, 1.0);
        bits.push(margin >= 0.0);
        reliabilities.push(reliability);
        confidence_sum += reliability;
    }
    if confidence_sum <= f32::EPSILON {
        return None;
    }
    Some((bits, reliabilities, confidence_sum))
}

fn build_header_region_vote_candidate(
    region_signed: &[f32],
    region_weights: &[f32],
    delta: usize,
    region_mask: u8,
) -> Option<(Vec<bool>, Vec<f32>, f32)> {
    if region_mask == 0 {
        return None;
    }
    let modulus = ROBUST_HEADER_BITS * 3;
    let mut bits = Vec::with_capacity(ROBUST_HEADER_BITS);
    let mut reliabilities = Vec::with_capacity(ROBUST_HEADER_BITS);
    let mut confidence_sum = 0.0_f32;
    for index in 0..ROBUST_HEADER_BITS {
        let local_phase = (index * 3 + modulus - delta) % modulus;
        let mut signed = 0.0_f32;
        let mut weight = 0.0_f32;
        for region in 0..BODY_VOTE_REGION_COUNT {
            if region_mask & (1_u8 << region) == 0 {
                continue;
            }
            let slot = region * modulus + local_phase;
            signed += region_signed[slot];
            weight += region_weights[slot];
        }
        if weight <= f32::EPSILON {
            bits.push(false);
            reliabilities.push(0.0);
            continue;
        }
        let margin = signed / weight;
        let reliability = margin.abs().clamp(0.0, 1.0);
        bits.push(margin >= 0.0);
        reliabilities.push(reliability);
        confidence_sum += reliability;
    }
    Some((bits, reliabilities, confidence_sum))
}

fn robust_known_header_bits() -> &'static Vec<(usize, bool)> {
    static BITS: OnceLock<Vec<(usize, bool)>> = OnceLock::new();
    BITS.get_or_init(|| {
        let mut known = Vec::new();
        for (offset, bytes) in [
            (0_usize, ROBUST_MAGIC.as_slice()),
            (31_usize, ROBUST_PILOT.as_slice()),
        ] {
            for (byte_index, byte) in bytes.iter().enumerate() {
                for shift in (0..8).rev() {
                    known.push((
                        (offset + byte_index) * 8 + (7 - shift),
                        ((*byte >> shift) & 1) == 1,
                    ));
                }
            }
        }
        let format_bits = bytes_to_bits(&[crate::payload::ROBUST_FORMAT]);
        known.extend(
            format_bits
                .into_iter()
                .enumerate()
                .map(|(index, bit)| (32 + index, bit)),
        );
        known.extend((40..47).map(|index| (index, false)));
        known
    })
}

// 将当前读取配置字节加入几何导频，是为了在无需解出完整头的阶段区分新旧载波，并减少另一配置产生的偶然相关峰。
// Adding the active profile byte to the geometric pilot distinguishes new and legacy carriers before full header recovery and reduces accidental peaks from the other profile.
fn geometry_known_header_bits(qim_profile: u8) -> &'static Vec<(usize, bool)> {
    static PROFILE_BITS: OnceLock<[Vec<(usize, bool)>; 2]> = OnceLock::new();
    let profiles = PROFILE_BITS.get_or_init(|| {
        std::array::from_fn(|profile| {
            let mut known = robust_known_header_bits().to_vec();
            known.extend(
                bytes_to_bits(&[profile as u8])
                    .into_iter()
                    .enumerate()
                    .map(|(index, bit)| (35 * 8 + index, bit)),
            );
            known
        })
    });
    &profiles[usize::from(qim_profile.min(dwt_qim::QIM_PROFILE_PYRAMID))]
}

fn robust_header_signature_score(bits: &[bool]) -> f32 {
    let known = robust_known_header_bits();
    if bits.len() < ROBUST_HEADER_BITS || known.is_empty() {
        return 0.0;
    }
    let matches = known
        .iter()
        .filter(|(index, expected)| bits[*index] == *expected)
        .count();
    matches as f32 / known.len() as f32
}

fn decode_or_repair_robust_header(
    bits: &[bool],
    reliabilities: &[f32],
) -> Option<RobustHeader> {
    if let Ok(header) = RobustHeader::from_bytes(&bits_to_bytes(bits)) {
        return Some(header);
    }
    let mut weak: Vec<usize> = (0..bits.len()).collect();
    weak.sort_by(|left, right| {
        reliabilities[*left]
            .partial_cmp(&reliabilities[*right])
            .unwrap_or(Ordering::Equal)
    });
    weak.truncate(HEADER_REPAIR_WEAK_BITS.min(weak.len()));

    for &first in &weak {
        let mut repaired = bits.to_vec();
        repaired[first] = !repaired[first];
        if let Ok(header) = RobustHeader::from_bytes(&bits_to_bytes(&repaired)) {
            return Some(header);
        }
    }
    if HEADER_REPAIR_MAX_FLIPS >= 2 {
        for left in 0..weak.len() {
            for right in left + 1..weak.len() {
                let mut repaired = bits.to_vec();
                repaired[weak[left]] = !repaired[weak[left]];
                repaired[weak[right]] = !repaired[weak[right]];
                if let Ok(header) = RobustHeader::from_bytes(&bits_to_bytes(&repaired)) {
                    return Some(header);
                }
            }
        }
    }
    if HEADER_REPAIR_MAX_FLIPS >= 3 {
        for first in 0..weak.len() {
            for second in first + 1..weak.len() {
                for third in second + 1..weak.len() {
                    let mut repaired = bits.to_vec();
                    repaired[weak[first]] = !repaired[weak[first]];
                    repaired[weak[second]] = !repaired[weak[second]];
                    repaired[weak[third]] = !repaired[weak[third]];
                    if let Ok(header) = RobustHeader::from_bytes(&bits_to_bytes(&repaired)) {
                        return Some(header);
                    }
                }
            }
        }
    }
    if HEADER_REPAIR_MAX_FLIPS >= 4 {
        for first in 0..weak.len() {
            for second in first + 1..weak.len() {
                for third in second + 1..weak.len() {
                    for fourth in third + 1..weak.len() {
                        let mut repaired = bits.to_vec();
                        repaired[weak[first]] = !repaired[weak[first]];
                        repaired[weak[second]] = !repaired[weak[second]];
                        repaired[weak[third]] = !repaired[weak[third]];
                        repaired[weak[fourth]] = !repaired[weak[fourth]];
                        if let Ok(header) = RobustHeader::from_bytes(&bits_to_bytes(&repaired)) {
                            // 四位修复仍只遍历最低置信度集合且必须通过完整 CRC，是为了让大面积遮挡边缘造成的少量头位翻转可恢复，而不会把弱相关噪声当成有效水印。
                            // Four-bit repair still searches only the least-reliable set and requires the full CRC so a few header flips near a large occlusion edge can recover without accepting weakly correlated noise as a watermark.
                            return Some(header);
                        }
                    }
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
struct ScaleLayerScores {
    priority: Vec<GeometryCandidate>,
    coarse: Vec<GeometryCandidate>,
    refined: Vec<GeometryCandidate>,
    fine: Vec<GeometryCandidate>,
}

impl ScaleLayerScores {
    #[allow(dead_code)]
    fn empty() -> Self {
        Self {
            priority: Vec::new(),
            coarse: Vec::new(),
            refined: Vec::new(),
            fine: Vec::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn search_scale_geometry_layers(
    source: &RgbaImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    attempted: &mut Vec<GeometryCandidate>,
    credential_error: &mut Option<CoreError>,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<(Option<ExtractReport>, ScaleLayerScores)> {
    let priority = score_geometry_parameters(
        source,
        period_aligned_scale_parameters(
            source.width(),
            source.height(),
            priority_scale_parameters(),
        )
        .into_iter()
        .map(|scale| (0.0, scale)),
        2,
        320,
        "resize-priority",
        on_progress,
        progress_current,
    )?;
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &priority,
        attempted,
        credential_error,
        "resize-priority",
        on_progress,
        progress_current,
    )? {
        return Ok((
            Some(report),
            ScaleLayerScores {
                priority,
                coarse: Vec::new(),
                refined: Vec::new(),
                fine: Vec::new(),
            },
        ));
    }

    // 粗搜步长用 1.04 且相位步长为 2，是因为微信类压缩分辨率很少落在整齐倍数上，4 像素相位会把已经错位的块周期再抽样得更稀。
    // Coarse search uses 1.04 steps and a 2-pixel phase so WeChat-style compressed resolutions that miss exact multiples are not undersampled by a 4-pixel phase on an already shifted grid.
    let coarse = score_geometry_parameters(
        source,
        coarse_scale_parameters().into_iter().map(|scale| (0.0, scale)),
        2,
        288,
        "resize-coarse",
        on_progress,
        progress_current,
    )?;
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &coarse,
        attempted,
        credential_error,
        "resize-coarse",
        on_progress,
        progress_current,
    )? {
        return Ok((
            Some(report),
            ScaleLayerScores {
                priority,
                coarse,
                refined: Vec::new(),
                fine: Vec::new(),
            },
        ));
    }

    let mut scale_seeds = Vec::new();
    extend_unique_geometry_candidates(&mut scale_seeds, priority.iter().copied());
    extend_unique_geometry_candidates(&mut scale_seeds, coarse.iter().copied());
    scale_seeds.sort_by(compare_geometry_candidates);
    scale_seeds.sort_by(compare_geometry_candidates);
    scale_seeds.truncate(8);
    // 截断之后再补回常见上采样校正，避免按分数排序时 2×/4× 噪声把 1.5× 挤出精化种子。
    // Forced upsample corrections are appended after truncation so 2×/4× noise cannot push 1.5× out of the refine seeds during score sorting.
    for scale in [1.25_f32, 1.5, 2.0, 3.0, 4.0] {
        if let Some(candidate) = priority
            .iter()
            .chain(coarse.iter())
            .find(|candidate| (candidate.scale - scale).abs() < 0.03)
        {
            extend_unique_geometry_candidates(&mut scale_seeds, [*candidate]);
        }
    }

    // 别扭比例的缩小图在粗网格上往往只有弱峰；若再用 0.0035 门槛挡住精化，正确块周期就永远进不了物化解码。
    // Awkward downscales often produce only weak coarse peaks; gating refinement at 0.0035 would keep the true block period from ever being materialized.
    let refined = if scale_seeds.is_empty() {
        Vec::new()
    } else {
        score_geometry_parameters(
            source,
            period_aligned_scale_parameters(
                source.width(),
                source.height(),
                scale_seeds.iter().flat_map(|candidate| {
                    (-4..=4).map(move |step| candidate.scale * (1.0 + step as f32 * 0.02))
                }),
            )
            .into_iter()
            .map(|scale| (0.0, scale)),
            2,
            320,
            "resize-refine",
            on_progress,
            progress_current,
        )?
    };
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &refined,
        attempted,
        credential_error,
        "resize-refine",
        on_progress,
        progress_current,
    )? {
        return Ok((
            Some(report),
            ScaleLayerScores {
                priority,
                coarse,
                refined,
                fine: Vec::new(),
            },
        ));
    }

    let fine = if refined.is_empty() {
        Vec::new()
    } else {
        score_geometry_parameters(
            source,
            period_aligned_scale_parameters(
                source.width(),
                source.height(),
                refined.iter().take(3).flat_map(|candidate| {
                    (-5..=5).map(move |step| candidate.scale * (1.0 + step as f32 * 0.003))
                }),
            )
            .into_iter()
            .map(|scale| (0.0, scale)),
            1,
            416,
            "resize-fine",
            on_progress,
            progress_current,
        )?
    };
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &fine,
        attempted,
        credential_error,
        "resize-fine",
        on_progress,
        progress_current,
    )? {
        return Ok((
            Some(report),
            ScaleLayerScores {
                priority,
                coarse,
                refined,
                fine,
            },
        ));
    }

    Ok((
        None,
        ScaleLayerScores {
            priority,
            coarse,
            refined,
            fine,
        },
    ))
}

fn wrap_rotation_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped > 180.0 {
        wrapped -= 360.0;
    }
    if wrapped <= -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

fn rotation_bin_centers() -> Vec<f32> {
    let mut angles: Vec<f32> = (-18..=18)
        .filter(|step| *step != 0)
        .map(|step| step as f32 * 10.0)
        .collect();
    // 2° 这类靠近 0 的真角离 ±10° 分箱太远，稀疏分会低于 30°/150° 噪声；身份之后、分箱之前补上常见小角探点。
    // True angles such as 2° are too far from the ±10° bins, so sparse scores lose to 30°/150° noise; small probes run after identity and before binning.
    for magnitude in [1.0_f32, 2.0, 3.0, 5.0] {
        angles.push(magnitude);
        angles.push(-magnitude);
    }
    angles
}

fn expanded_range_angles(center: f32, half_width: f32, step: f32) -> Vec<f32> {
    let mut angles = Vec::new();
    if step <= 0.0 {
        return angles;
    }
    let count = (half_width / step).round() as i32;
    for index in -count..=count {
        let angle = wrap_rotation_degrees(center + index as f32 * step);
        if angle.abs() >= 0.001 {
            angles.push(angle);
        }
    }
    angles.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    angles.dedup_by(|left, right| (*left - *right).abs() < step * 0.4);
    angles
}

fn select_separated_peaks(
    scored: &[GeometryCandidate],
    count: usize,
    min_gap: f32,
) -> Vec<GeometryCandidate> {
    let mut selected: Vec<GeometryCandidate> = Vec::new();
    // 2° 真峰稀疏分可能低于 30°/150° 谐波；每个布局先锁一个 |θ|≤8° 的近零峰。
    // A true 2° peak can score below 30°/150° harmonics; lock one |θ|≤8° near-zero peak per layout first.
    for candidate in scored.iter().filter(|candidate| candidate.rotation_degrees.abs() <= 8.0) {
        if selected.iter().any(|existing| existing.layout == candidate.layout) {
            continue;
        }
        selected.push(*candidate);
    }
    for candidate in scored {
        let same_layout_count = selected
            .iter()
            .filter(|existing| existing.layout == candidate.layout)
            .count();
        if same_layout_count >= count {
            continue;
        }
        // 同一角度上 AABB 与铺满分可能都很像；按布局分别留峰，避免高分错误布局把真模型挤出归一梯子。
        // AABB and fill-crop can both look plausible at the same angle; keep peaks per layout so a high-scoring wrong model cannot push the true one off the refine ladder.
        if selected.iter().any(|existing| {
            existing.layout == candidate.layout
                && wrap_rotation_degrees(candidate.rotation_degrees - existing.rotation_degrees)
                    .abs()
                    < min_gap
        }) {
            continue;
        }
        selected.push(*candidate);
    }
    selected
}

fn scale_for_layout(source: &RgbaImage, angle: f32, layout: GeometryLayout) -> f32 {
    match layout {
        GeometryLayout::FixedCanvasFit => {
            1.0 / rotation_canvas_growth(source.width(), source.height(), angle)
        }
        _ => 1.0,
    }
}

fn extend_rotation_dual_layout_parameters(
    parameters: &mut Vec<(f32, f32, GeometryLayout)>,
    source: &RgbaImage,
    angle: f32,
    has_rotation_padding: bool,
) {
    let growth = rotation_canvas_growth(source.width(), source.height(), angle);
    for extra in [0.955_f32, 1.0, 1.03] {
        let scale = 1.0 / (growth * extra);
        if (0.25..=4.0).contains(&scale) {
            parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
        }
    }
    parameters.push((angle, 1.0, GeometryLayout::AabbPadded));
    if angle.abs() <= 8.0 {
        // 小角 AABB 后裁回原图高度只出现在几度以内；大角分箱不必物化这两种轴裁布局。
        // Axis-cropped AABB after a small editor rotation only appears within a few degrees; far bins should not materialize those layouts.
        parameters.push((angle, 1.0, GeometryLayout::AabbHeightKept));
        parameters.push((angle, 1.0, GeometryLayout::AabbWidthKept));
    }
    if has_rotation_padding {
        parameters.push((angle, 1.0, GeometryLayout::ExpandedCanvas));
    }
}

fn rotation_bin_parameters(
    source: &RgbaImage,
    has_rotation_padding: bool,
) -> Vec<(f32, f32, GeometryLayout)> {
    let mut parameters = Vec::new();
    for angle in rotation_bin_centers() {
        // 补边检测会把中心放大的白角当成旋转衬边；每个分箱都同时试铺满裁切和 AABB，不能只走其中一条。
        // Padding detection treats center-zoom white corners as rotation pads; every bin must try fill-crop and AABB instead of a single layout.
        extend_rotation_dual_layout_parameters(
            &mut parameters,
            source,
            angle,
            has_rotation_padding,
        );
        if !has_rotation_padding {
            parameters.push((angle, 1.0, GeometryLayout::TightBounds));
        }
    }
    parameters
}

fn adjacent_bin_scores_are_flat(scored: &[GeometryCandidate]) -> bool {
    let Some(best) = scored.first() else {
        return false;
    };
    if best.score <= f32::EPSILON {
        return false;
    }
    scored.iter().take(6).skip(1).any(|candidate| {
        wrap_rotation_degrees(candidate.rotation_degrees - best.rotation_degrees).abs() <= 12.0
            && (best.score - candidate.score).abs() < 0.08 * best.score.max(0.01)
    })
}

#[allow(clippy::too_many_arguments)]
fn search_narrowed_rotation(
    source: &RgbaImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    has_rotation_padding: bool,
    attempted: &mut Vec<GeometryCandidate>,
    credential_error: &mut Option<CoreError>,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Option<ExtractReport>> {
    // 编辑器常把 AABB 旋转后再裁回原图高度，只剩左右白边；这些候选必须先于铺满裁切，否则 10MP 相机会在 ±7° 铺满上耗尽时间。
    // Editors often AABB-rotate then crop back to the original height, leaving only left/right pads; these candidates must run before fill-crop or a 10MP photo spends its budget on ±7° cover undos.
    let mut axis_crop_probes = Vec::new();
    for angle in [-2.0_f32, 2.0, -2.3, 2.3] {
        axis_crop_probes.push(GeometryCandidate {
            rotation_degrees: angle,
            scale: 1.0,
            layout: GeometryLayout::AabbHeightKept,
            score: 1.0,
        });
        axis_crop_probes.push(GeometryCandidate {
            rotation_degrees: angle,
            scale: 1.0,
            layout: GeometryLayout::AabbWidthKept,
            score: 1.0,
        });
    }
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &axis_crop_probes,
        attempted,
        credential_error,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    // 铺满裁切先按列表顺序物化覆盖倒数，避免稀疏评分把 ±7° 挤出前八名后再对错误小角做全图验证。
    // Fill-crop materializes cover reciprocals in list order first so sparse scores cannot drop ±7° out of the top eight and then spend full warps on false small angles.
    let fill_seven = fill_crop_cover_undo_candidates(source);
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &fill_seven,
        attempted,
        credential_error,
        "rotation-fill-forced",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let mut trim_probes = Vec::new();
    // 裁掉白边后既不是 ExpandedCanvas 也常不是 TightBounds；7°+1.03 导出裁边后要用 Standard 的 0.97 收回内容尺度。
    // After trimming white pads the canvas is often neither ExpandedCanvas nor TightBounds; a 7°+1.03 export needs Standard at 0.97 to restore content scale.
    for angle in [-7.0_f32, 7.0, -2.0, 2.0] {
        for scale in [0.97_f32, 1.0, 1.03] {
            trim_probes.push(GeometryCandidate {
                rotation_degrees: angle,
                scale,
                layout: GeometryLayout::Standard,
                score: 1.0,
            });
        }
        for scale in [1.03_f32, 1.0, 0.97] {
            trim_probes.push(GeometryCandidate {
                rotation_degrees: angle,
                scale,
                layout: GeometryLayout::TightBounds,
                score: 1.0,
            });
        }
    }
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &trim_probes,
        attempted,
        credential_error,
        "rotation-fill-forced",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let fill_small = fill_crop_small_angle_undo_candidates(source);
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &fill_small,
        attempted,
        credential_error,
        "rotation-fill-forced",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let mut pad_probes = Vec::new();
    for angle in [-2.0_f32, 2.0, -7.0, 7.0, -17.3, 17.3, -41.7, 41.7, -1.0, 1.0] {
        pad_probes.push(GeometryCandidate {
            rotation_degrees: angle,
            scale: 1.0,
            layout: GeometryLayout::AabbPadded,
            score: 1.0,
        });
    }
    for angle in [-2.0_f32, 2.0, -2.3, 2.3] {
        pad_probes.push(GeometryCandidate {
            rotation_degrees: angle,
            scale: 1.0,
            layout: GeometryLayout::AabbHeightKept,
            score: 1.0,
        });
        pad_probes.push(GeometryCandidate {
            rotation_degrees: angle,
            scale: 1.0,
            layout: GeometryLayout::AabbWidthKept,
            score: 1.0,
        });
    }
    for angle in [-7.0_f32, 7.0] {
        for scale in [1.03_f32, 1.0, 0.97] {
            pad_probes.push(GeometryCandidate {
                rotation_degrees: angle,
                scale,
                layout: GeometryLayout::ExpandedCanvas,
                score: 1.0,
            });
            pad_probes.push(GeometryCandidate {
                rotation_degrees: angle,
                scale,
                layout: GeometryLayout::TightBounds,
                score: 1.0,
            });
        }
    }
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &pad_probes,
        attempted,
        credential_error,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let bin_scores = score_geometry_layout_parameters(
        source,
        rotation_bin_parameters(source, has_rotation_padding),
        4,
        160,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )?;
    let ranges = select_separated_peaks(&bin_scores, 3, 20.0);
    if ranges.is_empty() {
        return Ok(None);
    }
    let bin_half_width = if adjacent_bin_scores_are_flat(&bin_scores) {
        25.0
    } else {
        15.0
    };

    let mut one_degree_parameters = Vec::new();
    for peak in &ranges {
        for angle in expanded_range_angles(peak.rotation_degrees, bin_half_width, 1.0) {
            extend_rotation_dual_layout_parameters(
                &mut one_degree_parameters,
                source,
                angle,
                has_rotation_padding,
            );
            if !matches!(
                peak.layout,
                GeometryLayout::FixedCanvasFit
                    | GeometryLayout::AabbPadded
                    | GeometryLayout::AabbHeightKept
                    | GeometryLayout::AabbWidthKept
                    | GeometryLayout::ExpandedCanvas
            ) {
                one_degree_parameters.push((
                    angle,
                    scale_for_layout(source, angle, peak.layout),
                    peak.layout,
                ));
            }
        }
    }

    let one_degree_scores = score_geometry_layout_parameters(
        source,
        one_degree_parameters,
        4,
        192,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )?;
    let degree_peaks = select_separated_peaks(&one_degree_scores, 3, 3.0);

    let mut tenth_parameters = Vec::new();
    for peak in &degree_peaks {
        for angle in expanded_range_angles(peak.rotation_degrees, 1.5, 0.1) {
            extend_rotation_dual_layout_parameters(
                &mut tenth_parameters,
                source,
                angle,
                has_rotation_padding,
            );
            match peak.layout {
                GeometryLayout::AabbPadded
                | GeometryLayout::AabbHeightKept
                | GeometryLayout::AabbWidthKept
                | GeometryLayout::ExpandedCanvas => {
                    for scale in [0.97_f32, 1.0, 1.03] {
                        tenth_parameters.push((angle, scale, peak.layout));
                    }
                }
                GeometryLayout::FixedCanvasFit => {
                    let growth = rotation_canvas_growth(source.width(), source.height(), angle);
                    for extra in [0.95_f32, 0.97, 1.0, 1.03, 1.05] {
                        let scale = 1.0 / (growth * extra);
                        if (0.25..=4.0).contains(&scale) {
                            tenth_parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
                        }
                    }
                    tenth_parameters.push((angle, 1.0, GeometryLayout::FixedCanvasFit));
                    if source.width() < 1600 || source.height() < 1600 {
                        tenth_parameters.push((angle, 1.0, GeometryLayout::Standard));
                        if (0.25..=4.0).contains(&(1.0 / growth)) {
                            tenth_parameters.push((angle, 1.0 / growth, GeometryLayout::Standard));
                        }
                    }
                }
                layout => {
                    tenth_parameters.push((
                        angle,
                        scale_for_layout(source, angle, layout),
                        layout,
                    ));
                }
            }
        }
    }
    let tenth_scores = score_geometry_layout_parameters(
        source,
        tenth_parameters,
        2,
        224,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )?;
    let tenth_peaks = select_separated_peaks(&tenth_scores, 3, 3.0);
    if tenth_peaks.is_empty() {
        return Ok(None);
    }

    let mut hundredth_parameters = Vec::new();
    for peak in &tenth_peaks {
        for angle in expanded_range_angles(peak.rotation_degrees, 0.15, 0.01) {
            hundredth_parameters.push((angle, peak.scale, peak.layout));
            extend_rotation_dual_layout_parameters(
                &mut hundredth_parameters,
                source,
                angle,
                has_rotation_padding,
            );
        }
    }
    let hundredth_scores = score_geometry_layout_parameters(
        source,
        hundredth_parameters,
        1,
        256,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )?;

    let mut to_materialize = Vec::new();
    for angle in [-2.0_f32, 2.0, -1.0, 1.0, -7.0, 7.0, -3.0, 3.0] {
        let growth = rotation_canvas_growth(source.width(), source.height(), angle);
        for extra in [0.955_f32, 1.0, 1.03] {
            let scale = 1.0 / (growth * extra);
            if (0.25..=4.0).contains(&scale) {
                extend_unique_geometry_candidates(
                    &mut to_materialize,
                    [GeometryCandidate {
                        rotation_degrees: angle,
                        scale,
                        layout: GeometryLayout::FixedCanvasFit,
                        score: 1.0,
                    }],
                );
            }
        }
        extend_unique_geometry_candidates(
            &mut to_materialize,
            [GeometryCandidate {
                rotation_degrees: angle,
                scale: 1.0,
                layout: GeometryLayout::AabbPadded,
                score: 1.0,
            }],
        );
        for layout in [GeometryLayout::AabbHeightKept, GeometryLayout::AabbWidthKept] {
            extend_unique_geometry_candidates(
                &mut to_materialize,
                [GeometryCandidate {
                    rotation_degrees: angle,
                    scale: 1.0,
                    layout,
                    score: 1.0,
                }],
            );
        }
        if has_rotation_padding {
            extend_unique_geometry_candidates(
                &mut to_materialize,
                [GeometryCandidate {
                    rotation_degrees: angle,
                    scale: 1.0,
                    layout: GeometryLayout::ExpandedCanvas,
                    score: 1.0,
                }],
            );
        }
    }
    for peak in &tenth_peaks {
        match peak.layout {
            GeometryLayout::AabbPadded
            | GeometryLayout::AabbHeightKept
            | GeometryLayout::AabbWidthKept
            | GeometryLayout::ExpandedCanvas => {
                for scale in [1.0_f32, 0.97, 1.03] {
                    extend_unique_geometry_candidates(
                        &mut to_materialize,
                        [GeometryCandidate {
                            rotation_degrees: peak.rotation_degrees,
                            scale,
                            layout: peak.layout,
                            score: peak.score,
                        }],
                    );
                }
                let growth =
                    rotation_canvas_growth(source.width(), source.height(), peak.rotation_degrees);
                for extra in [1.0_f32, 0.955, 1.03] {
                    let scale = 1.0 / (growth * extra);
                    if (0.25..=4.0).contains(&scale) {
                        extend_unique_geometry_candidates(
                            &mut to_materialize,
                            [GeometryCandidate {
                                rotation_degrees: peak.rotation_degrees,
                                scale,
                                layout: GeometryLayout::FixedCanvasFit,
                                score: peak.score,
                            }],
                        );
                    }
                }
            }
            GeometryLayout::FixedCanvasFit => {
                let growth =
                    rotation_canvas_growth(source.width(), source.height(), peak.rotation_degrees);
                for extra in [1.0_f32, 0.955, 1.03] {
                    let scale = 1.0 / (growth * extra);
                    if (0.25..=4.0).contains(&scale) {
                        extend_unique_geometry_candidates(
                            &mut to_materialize,
                            [GeometryCandidate {
                                rotation_degrees: peak.rotation_degrees,
                                scale,
                                layout: GeometryLayout::FixedCanvasFit,
                                score: peak.score,
                            }],
                        );
                    }
                }
                extend_unique_geometry_candidates(
                    &mut to_materialize,
                    [GeometryCandidate {
                        rotation_degrees: peak.rotation_degrees,
                        scale: 1.0,
                        layout: GeometryLayout::AabbPadded,
                        score: peak.score,
                    }],
                );
                for layout in [GeometryLayout::AabbHeightKept, GeometryLayout::AabbWidthKept] {
                    extend_unique_geometry_candidates(
                        &mut to_materialize,
                        [GeometryCandidate {
                            rotation_degrees: peak.rotation_degrees,
                            scale: 1.0,
                            layout,
                            score: peak.score,
                        }],
                    );
                }
            }
            _ => {
                extend_unique_geometry_candidates(&mut to_materialize, [*peak]);
                let mut sibling = Vec::new();
                extend_rotation_dual_layout_parameters(
                    &mut sibling,
                    source,
                    peak.rotation_degrees,
                    has_rotation_padding,
                );
                for (rotation_degrees, scale, layout) in sibling {
                    extend_unique_geometry_candidates(
                        &mut to_materialize,
                        [GeometryCandidate {
                            rotation_degrees,
                            scale,
                            layout,
                            score: peak.score,
                        }],
                    );
                }
            }
        }
    }
    extend_unique_geometry_candidates(&mut to_materialize, hundredth_scores.iter().copied().take(3));
    try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &to_materialize,
        attempted,
        credential_error,
        "rotation-range-narrow",
        on_progress,
        progress_current,
    )
}

fn luma_profile_row(source: &RgbaImage) -> Vec<f32> {
    let width = source.width() as usize;
    let height = source.height() as usize;
    let mut profile = vec![0.0_f32; width.max(1)];
    if width < 32 || height < 32 {
        return profile;
    }
    let row_start = height / 4;
    let row_end = (height * 3 / 4).max(row_start + 1);
    let row_step = ((row_end - row_start) / 16).max(1);
    let mut rows = 0_f32;
    for y in (row_start..row_end).step_by(row_step) {
        for x in 0..width {
            let pixel = source.get_pixel(x as u32, y as u32);
            profile[x] += 0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32;
        }
        rows += 1.0;
    }
    if rows > 0.0 {
        for value in &mut profile {
            *value /= rows;
        }
    }
    let mean = profile.iter().sum::<f32>() / profile.len().max(1) as f32;
    for value in &mut profile {
        *value -= mean;
    }
    profile
}

fn luma_profile_column(source: &RgbaImage) -> Vec<f32> {
    let width = source.width() as usize;
    let height = source.height() as usize;
    let mut profile = vec![0.0_f32; height.max(1)];
    if width < 32 || height < 32 {
        return profile;
    }
    let col_start = width / 4;
    let col_end = (width * 3 / 4).max(col_start + 1);
    let col_step = ((col_end - col_start) / 16).max(1);
    let mut cols = 0_f32;
    for x in (col_start..col_end).step_by(col_step) {
        for y in 0..height {
            let pixel = source.get_pixel(x as u32, y as u32);
            profile[y] += 0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32;
        }
        cols += 1.0;
    }
    if cols > 0.0 {
        for value in &mut profile {
            *value /= cols;
        }
    }
    let mean = profile.iter().sum::<f32>() / profile.len().max(1) as f32;
    for value in &mut profile {
        *value -= mean;
    }
    profile
}

fn best_autocorr_lag(profile: &[f32]) -> Option<f32> {
    if profile.len() < 48 {
        return None;
    }
    let mut best_lag = 0_usize;
    let mut best_score = f32::NEG_INFINITY;
    for lag in 5..=16 {
        if lag >= profile.len() / 4 {
            break;
        }
        let mut acc = 0.0_f32;
        let mut count = 0.0_f32;
        for index in 0..profile.len() - lag {
            acc += profile[index] * profile[index + lag];
            count += 1.0;
        }
        let score = acc / count.max(1.0);
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }
    if best_lag == 0 {
        None
    } else {
        Some(best_lag as f32)
    }
}

fn estimate_observed_block_period(source: &RgbaImage) -> Option<f32> {
    // 稀疏亮度自相关估八像素块周期；中心放大后周期变成 8×zoom，undo = 8 / observed。
    // Sparse luma autocorrelation estimates the eight-pixel block period; after center zoom the period is 8×zoom, so undo = 8 / observed.
    let row_lag = best_autocorr_lag(&luma_profile_row(source));
    let col_lag = best_autocorr_lag(&luma_profile_column(source));
    match (row_lag, col_lag) {
        (Some(row), Some(col)) => Some((row + col) * 0.5),
        (Some(lag), None) | (None, Some(lag)) => Some(lag),
        (None, None) => None,
    }
}

fn center_zoom_undo_grid() -> Vec<f32> {
    let mut scales = Vec::new();
    let mut scale = 0.50_f32;
    while scale <= 0.98001 {
        scales.push((scale * 100.0).round() / 100.0);
        scale += 0.02;
    }
    scales
}

fn push_unique_center_zoom_undo(target: &mut Vec<f32>, scale: f32) {
    if !(0.50..=0.98).contains(&scale) || !scale.is_finite() {
        return;
    }
    if target
        .iter()
        .any(|existing| (*existing - scale).abs() < 0.01)
    {
        return;
    }
    target.push(scale);
}

fn period_aligned_center_zoom_undos(source: &RgbaImage) -> Vec<f32> {
    let grid = center_zoom_undo_grid();
    let mut undos = Vec::new();
    for scale in [1.0 / 2.0, 1.0 / 1.5, 1.0 / 1.25, 1.0 / 1.1] {
        push_unique_center_zoom_undo(&mut undos, scale);
    }
    if let Some(observed) = estimate_observed_block_period(source) {
        let seed = (BLOCK_SIZE as f32 / observed).clamp(0.50, 0.98);
        push_unique_center_zoom_undo(&mut undos, seed);
        if let Some(index) = grid.iter().enumerate().min_by(|(_, left), (_, right)| {
            (**left - seed)
                .abs()
                .partial_cmp(&(**right - seed).abs())
                .unwrap_or(Ordering::Equal)
        }) {
            let start = index.0.saturating_sub(2);
            let end = (index.0 + 2).min(grid.len().saturating_sub(1));
            for neighbor in &grid[start..=end] {
                push_unique_center_zoom_undo(&mut undos, *neighbor);
            }
        }
    }
    undos
}

fn remaining_center_zoom_undos(seeds: &[f32]) -> Vec<f32> {
    let seed = seeds.first().copied().unwrap_or(0.80);
    let mut remaining = Vec::new();
    for scale in center_zoom_undo_grid() {
        if seeds.iter().any(|existing| (*existing - scale).abs() < 0.01) {
            continue;
        }
        remaining.push(scale);
    }
    remaining.sort_by(|left, right| {
        (*left - seed)
            .abs()
            .partial_cmp(&(*right - seed).abs())
            .unwrap_or(Ordering::Equal)
    });
    remaining
}

fn center_zoom_geometry_candidates(scales: &[f32]) -> Vec<GeometryCandidate> {
    let mut candidates = Vec::new();
    for &scale in scales {
        candidates.push(GeometryCandidate {
            rotation_degrees: 0.0,
            scale,
            layout: GeometryLayout::Standard,
            score: 1.0,
        });
        candidates.push(GeometryCandidate {
            rotation_degrees: 0.0,
            scale,
            layout: GeometryLayout::CenterZoom,
            score: 1.0,
        });
    }
    candidates
}

fn canvas_matches_public_header(source: &RgbaImage) -> bool {
    let (headers, _) = collect_robust_header_candidates(source, 3, false, false);
    headers.iter().any(|candidate| {
        candidate.header.original_width as u32 == source.width()
            && candidate.header.original_height as u32 == source.height()
    })
}

fn search_and_extract_geometry(
    source: &RgbaImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    prefer_identity_damage: bool,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Option<ExtractReport>> {
    let mut attempted = Vec::new();
    let mut credential_error = None;
    let has_rotation_padding = sync::has_uniform_rotation_padding(source);

    let center_zoom_seeds = period_aligned_center_zoom_undos(source);
    let center_zoom = center_zoom_geometry_candidates(&center_zoom_seeds);
    // 中心放大再裁回会留下白角，补边检测常误判；CenterZoom 必须先于公开头缩放，否则 resize-header 会把 0.80 改写成 0.5×/2×。
    // Center-zoom-then-crop leaves white corners that look like padding; CenterZoom must run before header-guided resize or resize-header rewrites 0.80 into 0.5×/2×.
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &center_zoom,
        &mut attempted,
        &mut credential_error,
        "center-zoom",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }
    let center_zoom_grid =
        center_zoom_geometry_candidates(&remaining_center_zoom_undos(&center_zoom_seeds));
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &center_zoom_grid,
        &mut attempted,
        &mut credential_error,
        "center-zoom-grid",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let header_guided_scales = header_guided_scale_candidates(source);
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &header_guided_scales,
        &mut attempted,
        &mut credential_error,
        "resize-header",
        on_progress,
        progress_current,
    )? {
        // 公开头已经携带原始宽高时直接恢复精确块周期，是因为继续枚举几十个缩放值只会重复求解一个已知答案。
        // When the public header already carries the original dimensions, restoring the exact block period directly avoids enumerating dozens of scales for an answer that is already known.
        return Ok(Some(report));
    }

    // 90°/270° 只需要三个精确候选，而且交换宽高后可直接恢复完整网格；即便没有白边也先于缩放搜索，避免无补边的直角旋转先为空跑一整轮分辨率恢复。
    // Quarter turns need only three exact candidates and restore the grid once width and height are swapped; they run before scale search even without padding so a bare 90° image does not pay a full resolution sweep first.
    let orientations = score_geometry_parameters(
        source,
        orientation_angle_parameters().into_iter().map(|angle| (angle, 1.0)),
        1,
        320,
        "orientation",
        on_progress,
        progress_current,
    )?;
    if let Some(report) = try_geometry_candidates(
        source,
        actual_width,
        actual_height,
        key_source,
        &orientations,
        &mut attempted,
        &mut credential_error,
        "orientation",
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    // 身份即便是“强导频但正文损坏”，旋转图仍可能在未校正网格上扫到假强导频；±180° 梯子必须继续跑。
    // Even when identity saw a strong pilot with a damaged body, a rotated image can still score a false pilot on the uncorrected grid; the ±180° ladder must still run.
    if let Some(report) = search_narrowed_rotation(
        source,
        actual_width,
        actual_height,
        key_source,
        has_rotation_padding,
        &mut attempted,
        &mut credential_error,
        on_progress,
        progress_current,
    )? {
        return Ok(Some(report));
    }

    let same_canvas_header = canvas_matches_public_header(source);
    if !has_rotation_padding && (!prefer_identity_damage || same_canvas_header) {
        // 同画布中心放大常把身份判成正文损坏；画布尺寸仍与公开头一致时必须继续跑缩放层。
        // Same-canvas center zoom often leaves identity as PayloadCorrupted; scale layers still run when the canvas matches the public header.
        let (report, _layers) = search_scale_geometry_layers(
            source,
            actual_width,
            actual_height,
            key_source,
            &mut attempted,
            &mut credential_error,
            on_progress,
            progress_current,
        )?;
        if let Some(report) = report {
            return Ok(Some(report));
        }
    }

    // 几何候选中的认证失败要延后到所有高分候选都验证完再返回，因为“头已对齐但正文仍略失配”和“凭据错误”都会表现为 AEAD 认证失败。
    // Authentication failures from geometry candidates are deferred until all high-scoring candidates are checked because both a header-aligned-but-body-misaligned transform and a wrong credential surface as AEAD authentication failure.
    if let Some(error) = credential_error {
        return Err(error);
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn try_geometry_candidates(
    source: &RgbaImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    candidates: &[GeometryCandidate],
    attempted: &mut Vec<GeometryCandidate>,
    credential_error: &mut Option<CoreError>,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    _progress_current: &mut usize,
) -> Result<Option<ExtractReport>> {
    let Some(best) = candidates.first() else {
        return Ok(None);
    };
    let is_resize_stage = stage.starts_with("resize");
    let is_padding_hint_stage = stage.starts_with("rotation-padding")
        && stage != "rotation-padding-forced";
    let is_forced_padding_stage = stage == "rotation-padding-forced"
        || stage == "rotation-fill-forced";
    let is_narrowed_rotation_stage = stage == "rotation-range-narrow";
    let is_center_zoom_stage = stage == "center-zoom";
    let is_center_zoom_grid_stage = stage == "center-zoom-grid";
    let is_targeted_rotation_stage = stage == "rotation-tight-priority"
        || stage == "rotation-fixed-priority"
        || stage.starts_with("rotation-refine")
        || stage.starts_with("rotation-fine")
        || stage.starts_with("rotation-resize-refine")
        || stage.starts_with("rotation-resize-fine");
    let minimum_score = if is_narrowed_rotation_stage || is_center_zoom_stage {
        // 周期对齐的邻近 undo 必须全部物化；密网格仍走缩放门槛，避免整表以 0 分强行解码。
        // Period-aligned nearby undos must all materialize; the dense grid keeps the resize floor so the whole table is not forced through at score zero.
        0.0
    } else if is_center_zoom_grid_stage {
        GEOMETRY_RESIZE_DECODE_MIN_SCORE
    } else if is_resize_stage {
        // 压缩分辨率很少落在整齐的 2×/4× 上，稀疏导频在正确比例附近往往只有弱峰；把缩放解码门槛降到 0.004，才能让精化后的前四名进入完整头扫描。
        // Compressed resolutions rarely land on exact 2×/4×, so the sparse pilot is often only a weak peak near the true scale; lowering the resize decode floor to 0.004 lets the refined top four enter a full header scan.
        GEOMETRY_RESIZE_DECODE_MIN_SCORE
    } else if is_padding_hint_stage {
        GEOMETRY_RESIZE_DECODE_MIN_SCORE
    } else if is_targeted_rotation_stage {
        GEOMETRY_MIN_DECODE_SCORE * 0.65
    } else {
        GEOMETRY_MIN_DECODE_SCORE
    };
    if best.score < minimum_score {
        return Ok(None);
    }
    let relative_ratio = if is_narrowed_rotation_stage || is_center_zoom_stage {
        0.0
    } else if is_center_zoom_grid_stage {
        0.0
    } else if is_resize_stage || is_padding_hint_stage {
        0.28
    } else if is_targeted_rotation_stage {
        0.48
    } else {
        GEOMETRY_RELATIVE_DECODE_SCORE
    };
    let relative_cutoff = (best.score * relative_ratio).max(minimum_score);
    let candidate_limit = if is_narrowed_rotation_stage {
        40
    } else if is_center_zoom_stage {
        24
    } else if is_center_zoom_grid_stage {
        32
    } else if is_forced_padding_stage {
        32
    } else if is_padding_hint_stage {
        8
    } else if is_resize_stage {
        8
    } else if is_targeted_rotation_stage {
        3
    } else if best.score >= GEOMETRY_STRONG_DECODE_SCORE {
        GEOMETRY_STAGE_DECODE_CANDIDATES
    } else if best.score >= GEOMETRY_MIN_DECODE_SCORE * 2.5 {
        2
    } else {
        1
    };
    // 缩放、白边提示和已经由粗峰收敛出的旋转层拥有明确几何先验，因此允许少量弱峰进入完整验证；普通盲搜仍保持严格阈值，避免无水印图片在错误候选上反复生成整图。
    // Resize, padding-hint, and rotation stages already converged from a coarse peak have explicit geometric priors, so a few weaker peaks may enter full verification; ordinary blind searches keep the strict threshold to avoid repeatedly materializing full images for unmarked inputs.
    // 只有达到相对峰值阈值的少量候选才生成完整校正图，是因为稀疏评分很便宜，而错误候选的全图插值和完整头扫描才是旧版“卡在 180°”的主要耗时来源。
    // Only a few candidates near the relative score peak are materialized because sparse scoring is cheap, while full-image interpolation and complete header scanning on false candidates caused the old apparent 180° stall.
    let mut selected: Vec<GeometryCandidate> = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.score >= relative_cutoff)
        .take(candidate_limit)
        .collect();
    if is_resize_stage {
        // 稀疏导频常把 0.62× 噪声排在真 1.52× 前面；解码列表以常见恢复倍数为主，并优先尝试 16 像素对齐的输出尺寸。
        // Sparse pilots often rank 0.62× noise above the true 1.52×; the decode list is built from common restore multiples and prefers 16-pixel output sizes.
        let mut restore = Vec::new();
        for scale in [0.5_f32, 0.25, 2.0, 1.5, 1.25, 3.0, 4.0] {
            if let Some(candidate) = candidates.iter().find(|candidate| {
                candidate.score >= minimum_score && (candidate.scale - scale).abs() < 0.08
            }) {
                extend_unique_geometry_candidates(&mut restore, [*candidate]);
            }
        }
        for candidate in candidates.iter().copied().take(3) {
            extend_unique_geometry_candidates(&mut restore, [candidate]);
        }
        if restore.is_empty() {
            restore = selected;
        }
        selected = expand_pixel_aligned_scale_candidates(source, &restore);
        selected.sort_by(|left, right| {
            header_period_cost(source, left)
                .cmp(&header_period_cost(source, right))
                .then_with(|| {
                    restore_ratio_cost(left.scale)
                        .partial_cmp(&restore_ratio_cost(right.scale))
                        .unwrap_or(Ordering::Equal)
                })
        });
        selected.truncate(12);
    } else if is_padding_hint_stage {
        // 白边提示下 FixedCanvasFit 噪声峰经常压过 ExpandedCanvas 的 0.97；解码必须先试扩展画布上的内容缩放，而不是只信导频排名。
        // Under padding hints, FixedCanvasFit noise often outranks ExpandedCanvas at 0.97; decode must try expanded-canvas content zooms instead of trusting pilot rank alone.
        let mut preferred = Vec::new();
        for scale in [0.97_f32, 1.0, 1.03, 0.96, 0.92, 1.04, 1.08] {
            for candidate in candidates.iter().copied().filter(|candidate| {
                candidate.layout == GeometryLayout::ExpandedCanvas
                    && candidate.score >= minimum_score
                    && (candidate.scale - scale).abs() < 0.015
            }).take(3) {
                extend_unique_geometry_candidates(&mut preferred, [candidate]);
            }
        }
        for candidate in candidates.iter().copied().filter(|candidate| {
            candidate.layout == GeometryLayout::TightBounds
                && candidate.score >= minimum_score
                && (candidate.scale - 1.0).abs() <= 0.12
        }).take(4) {
            extend_unique_geometry_candidates(&mut preferred, [candidate]);
        }
        for candidate in candidates.iter().copied().take(3) {
            extend_unique_geometry_candidates(&mut preferred, [candidate]);
        }
        if !preferred.is_empty() {
            selected = preferred;
            selected.truncate(12);
        }
    } else if stage == "rotation-fixed-priority" {
        // 铺满裁切的校正尺度是覆盖系数的倒数；稀疏导频常把错误小角度排在真 7° 前面，但若先物化 4°–8° 共十张图，1° 真峰会被困在 64 相位头扫描后面。
        // Fill-crop correction uses the reciprocal of the cover factor; sparse pilots often rank a wrong small angle above the true 7°, but materializing ten 4°–8° warps first traps a true 1° peak behind 64-phase header scans.
        let mut preferred = Vec::new();
        for candidate in candidates.iter().copied().take(3) {
            extend_unique_geometry_candidates(&mut preferred, [candidate]);
        }
        for sign in [-1.0_f32, 1.0] {
            let angle = 7.0 * sign;
            let growth = rotation_canvas_growth(source.width(), source.height(), angle);
            let undos = [1.0 / (growth * 1.03), 1.0 / growth, 1.0 / (growth * 1.05)];
            for undo in undos {
                if let Some(candidate) = candidates.iter().copied().find(|candidate| {
                    candidate.layout == GeometryLayout::FixedCanvasFit
                        && (candidate.rotation_degrees - angle).abs() < 0.15
                        && (candidate.scale - undo).abs() < 0.02
                }) {
                    extend_unique_geometry_candidates(&mut preferred, [candidate]);
                    break;
                }
            }
        }
        if !preferred.is_empty() {
            selected = preferred;
            selected.truncate(6);
        }
    }
    // 评分计数封顶后仍可能物化多张大图；验证列表单独编号，界面才不会误以为 1025/1025 已经结束。
    // After the scoring counter saturates, several large images may still be materialized; numbering the verify list separately keeps the UI from treating 1025/1025 as completion.
    let verify_jobs: Vec<GeometryCandidate> = selected
        .iter()
        .copied()
        .filter(|candidate| {
            !contains_geometry_parameter(
                attempted,
                candidate.rotation_degrees,
                candidate.scale,
                candidate.layout,
            )
        })
        .collect();
    let verify_total = verify_jobs.len();
    for (verify_index, candidate) in verify_jobs.into_iter().enumerate() {
        attempted.push(candidate);
        let transform = affine_for_geometry(
            source,
            candidate.rotation_degrees,
            candidate.scale,
            candidate.layout,
        );
        let Some(transform) = transform else {
            continue;
        };
        let verify_current = verify_index + 1;
        if let Some(callback) = on_progress.as_deref_mut() {
            callback(GeometrySearchProgress::verify(
                verify_current,
                verify_total,
                candidate.rotation_degrees,
                candidate.scale,
                true,
                0.0,
            ))?;
        }
        let output_height = transform.output_height().max(1);
        let scanned_rows = Cell::new(0_u32);
        let mut checkpoint = || -> Result<()> {
            let y = scanned_rows.get();
            scanned_rows.set(y.saturating_add(64));
            let sub_progress = (y as f32 / output_height as f32).clamp(0.0, 0.99);
            // 大约每 256 行发一次可见进度，是为了让大图双线性插值不再把界面冻在评分上限；其余 64 行检查点只负责取消。
            // Visible progress about every 256 rows keeps large bilinear warps from freezing the UI at the scoring cap; the other 64-row checkpoints exist only for cancellation.
            let should_emit = y == 0 || y % 256 == 0;
            if let Some(callback) = on_progress.as_deref_mut() {
                callback(GeometrySearchProgress::verify(
                    verify_current,
                    verify_total,
                    candidate.rotation_degrees,
                    candidate.scale,
                    should_emit,
                    sub_progress,
                ))?;
            }
            // 显式固定为核心错误类型，是因为该检查点同时经过回调和外层问号传播，留给编译器推断会产生多个合法的 From 路径。
            // Pin the checkpoint to the core error type because it flows through both the callback and outer question-mark propagation, which otherwise leaves multiple valid From paths for inference.
            Ok(())
        };
        let transformed_rgba = if candidate.layout == GeometryLayout::Standard
            && (candidate.scale - 1.0).abs() < 0.0005
            && ((candidate.rotation_degrees.abs() - 90.0).abs() < 0.001
                || (candidate.rotation_degrees.abs() - 180.0).abs() < 0.001)
        {
            sync::materialize_exact_orientation_checked(
                source,
                candidate.rotation_degrees,
                &mut checkpoint,
            )?
        } else {
            sync::materialize_transform_checked(source, &transform, &mut checkpoint)?
        };
        let Some(transformed_rgba) = transformed_rgba else {
            continue;
        };
        let trimmed_transformed = if candidate.rotation_degrees.abs() >= 0.001
            && candidate.layout != GeometryLayout::ExpandedCanvas
            && candidate.layout != GeometryLayout::FixedCanvasFit
            && candidate.layout != GeometryLayout::AabbPadded
            && candidate.layout != GeometryLayout::AabbHeightKept
            && candidate.layout != GeometryLayout::AabbWidthKept
            && candidate.layout != GeometryLayout::CenterZoom
        {
            // ExpandedCanvas 和 FixedCanvasFit 都已经把画布收回到原始宽高；再裁纯色角会少一两像素并毁掉刚恢复的八像素块周期。
            // ExpandedCanvas and FixedCanvasFit already restore the original canvas size; trimming solid corners would drop a pixel or two and destroy the eight-pixel period just recovered.
            sync::trim_uniform_border(&transformed_rgba)
        } else {
            None
        };
        let transformed = DynamicImage::ImageRgba8(transformed_rgba);
        let registration = SyncRegistration {
            rotation_degrees: candidate.rotation_degrees,
            scale: candidate.scale,
            translation_x: 0,
            translation_y: 0,
            score: candidate.score,
        };
        if let Some(callback) = on_progress.as_deref_mut() {
            // 插值完成后把头扫描标成该校正图的最后一段，避免大图 CRC 扫描期间进度条再次冻住。
            // After interpolation, the header scan is marked as the last slice of this corrected image so a large CRC scan cannot freeze the bar again.
            callback(GeometrySearchProgress::verify(
                verify_current,
                verify_total,
                candidate.rotation_degrees,
                candidate.scale,
                true,
                1.0,
            ))?;
        }
        let extraction = if let Some(trimmed) = trimmed_transformed {
            // 逆旋转后角落三角区通常会变成完整的纯色外带；先裁掉这些外带可恢复八像素块相位，而保留未裁切回退可兼容本身就具有纯色边缘的正常照片。
            // Corner triangles commonly become complete solid outer bands after inverse rotation; removing those bands first restores the eight-pixel block phase, while the untrimmed fallback preserves compatibility with ordinary photos that naturally have solid edges.
            extract_registered_image(
                &DynamicImage::ImageRgba8(trimmed),
                actual_width,
                actual_height,
                key_source,
                registration,
            )
            .or_else(|_| {
                extract_registered_image(
                    &transformed,
                    actual_width,
                    actual_height,
                    key_source,
                    registration,
                )
            })
        } else {
            extract_registered_image(
                &transformed,
                actual_width,
                actual_height,
                key_source,
                registration,
            )
        };
        match extraction {
            Ok(report) => return Ok(Some(report)),
            Err(error @ CoreError::SaltMismatch)
            | Err(error @ CoreError::DecryptionFailed) => {
                if credential_error.is_none() {
                    *credential_error = Some(error);
                }
            }
            Err(_) => {}
        }
    }
    Ok(None)
}

fn score_geometry_parameters<I>(
    source: &RgbaImage,
    parameters: I,
    pixel_phase_step: u32,
    sample_budget: usize,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Vec<GeometryCandidate>>
where
    I: IntoIterator<Item = (f32, f32)>,
{
    score_geometry_layout_parameters(
        source,
        parameters
            .into_iter()
            .map(|(rotation_degrees, scale)| {
                (rotation_degrees, scale, GeometryLayout::Standard)
            }),
        pixel_phase_step,
        sample_budget,
        stage,
        on_progress,
        progress_current,
    )
}

#[allow(dead_code)]
fn score_expanded_rotation_parameters<I>(
    source: &RgbaImage,
    angles: I,
    pixel_phase_step: u32,
    sample_budget: usize,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Vec<GeometryCandidate>>
where
    I: IntoIterator<Item = f32>,
{
    score_geometry_layout_parameters(
        source,
        angles.into_iter().map(|angle| {
            (angle, 1.0, GeometryLayout::ExpandedCanvas)
        }),
        pixel_phase_step,
        sample_budget,
        stage,
        on_progress,
        progress_current,
    )
}

#[allow(dead_code)]
fn score_fixed_canvas_rotation_parameters<I>(
    source: &RgbaImage,
    angles: I,
    pixel_phase_step: u32,
    sample_budget: usize,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Vec<GeometryCandidate>>
where
    I: IntoIterator<Item = f32>,
{
    score_geometry_layout_parameters(
        source,
        angles.into_iter().flat_map(|angle| {
            fixed_canvas_correction_scales(source.width(), source.height(), angle)
                .into_iter()
                .map(move |scale| (angle, scale, GeometryLayout::FixedCanvasFit))
        }),
        pixel_phase_step,
        sample_budget,
        stage,
        on_progress,
        progress_current,
    )
}

#[allow(dead_code)]
fn score_tight_rotation_parameters<I>(
    source: &RgbaImage,
    angles: I,
    pixel_phase_step: u32,
    sample_budget: usize,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Vec<GeometryCandidate>>
where
    I: IntoIterator<Item = f32>,
{
    score_geometry_layout_parameters(
        source,
        angles.into_iter().flat_map(|angle| {
            tight_bounds_correction_scales()
                .into_iter()
                .map(move |scale| (angle, scale, GeometryLayout::TightBounds))
        }),
        pixel_phase_step,
        sample_budget,
        stage,
        on_progress,
        progress_current,
    )
}

fn score_geometry_layout_parameters<I>(
    source: &RgbaImage,
    parameters: I,
    pixel_phase_step: u32,
    sample_budget: usize,
    stage: &'static str,
    on_progress: &mut Option<&mut dyn FnMut(GeometrySearchProgress) -> Result<()>>,
    progress_current: &mut usize,
) -> Result<Vec<GeometryCandidate>>
where
    I: IntoIterator<Item = (f32, f32, GeometryLayout)>,
{
    let mut seen = Vec::new();
    let mut work = Vec::new();
    for (rotation_degrees, scale, layout) in parameters {
        // 候选中的 scale 表示内容本身需要恢复的比例，而画布尺寸由布局模型单独决定；这样才能覆盖编辑器在自动适配画布后又附加少量缩放的情况。
        // Candidate scale represents the content scale to restore while the layout model decides canvas dimensions separately, covering editors that add a slight zoom after automatic canvas fitting.

        if rotation_degrees.abs() > 180.0
            || !(0.25..=4.0).contains(&scale)
            || (rotation_degrees.abs() < 0.0005 && (scale - 1.0).abs() < 0.0005)
            || contains_geometry_parameter(&seen, rotation_degrees, scale, layout)
        {
            continue;
        }
        let transform = affine_for_geometry(source, rotation_degrees, scale, layout);
        let Some(transform) = transform else {
            continue;
        };
        if transform.output_width() < MIN_EXTRACT_SIDE
            || transform.output_height() < MIN_EXTRACT_SIDE
            || !transform.can_materialize()
        {
            continue;
        }
        seen.push(GeometryCandidate {
            rotation_degrees,
            scale,
            layout,
            score: 0.0,
        });
        work.push((rotation_degrees, scale, layout, transform));
    }

    let workers = geometry_worker_count(work.len(), on_progress.is_some());
    let mut candidates = Vec::with_capacity(work.len());
    if workers <= 1 {
        for (rotation_degrees, scale, layout, transform) in work {
            *progress_current += 1;
            if let Some(callback) = on_progress.as_deref_mut() {
                callback(GeometrySearchProgress::scoring(
                    *progress_current,
                    rotation_degrees,
                    scale,
                    stage,
                    1,
                    true,
                ))?;
            }
            let score = geometry_signature_score(
                source,
                &transform,
                pixel_phase_step,
                sample_budget,
            );
            candidates.push(GeometryCandidate {
                rotation_degrees,
                scale,
                layout,
                score,
            });
        }
    } else {
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel();
        let mut callback_error = None;

        // 顶层图片的候选评分使用受限 CPU 工作线程，而视频帧路径保持单线程评分，是为了利用多核又避免与视频帧并行发生嵌套过度订阅。
        // Top-level image candidates use a bounded CPU worker pool while video-frame paths keep sequential scoring, using multiple cores without nested oversubscription against frame-level parallelism.
        thread::scope(|scope| {
            for _ in 0..workers {
                let sender = sender.clone();
                let next = &next;
                let stop = &stop;
                let work = &work;
                scope.spawn(move || {
                    loop {
                        if stop.load(AtomicOrdering::Relaxed) {
                            break;
                        }
                        let index = next.fetch_add(1, AtomicOrdering::Relaxed);
                        let Some((rotation_degrees, scale, layout, transform)) = work.get(index).copied()
                        else {
                            break;
                        };
                        if sender
                            .send(GeometryWorkerMessage::Started(rotation_degrees, scale))
                            .is_err()
                        {
                            break;
                        }
                        let Some(score) = geometry_signature_score_with_stop(
                            source,
                            &transform,
                            pixel_phase_step,
                            sample_budget,
                            Some(stop),
                        ) else {
                            break;
                        };
                        if sender
                            .send(GeometryWorkerMessage::Finished(GeometryCandidate {
                                rotation_degrees,
                                scale,
                                layout,
                                score,
                            }))
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = sender.send(GeometryWorkerMessage::Done);
                });
            }
            drop(sender);

            let mut completed_workers = 0_usize;
            let mut active_parameter = (0.0_f32, 1.0_f32);
            while completed_workers < workers {
                let message = match receiver.recv_timeout(Duration::from_millis(60)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if callback_error.is_none() {
                            if let Some(callback) = on_progress.as_deref_mut() {
                                if let Err(error) = callback(GeometrySearchProgress::scoring(
                                    *progress_current,
                                    active_parameter.0,
                                    active_parameter.1,
                                    stage,
                                    workers,
                                    false,
                                )) {
                                    stop.store(true, AtomicOrdering::Relaxed);
                                    callback_error = Some(error);
                                }
                            }
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                match message {
                    GeometryWorkerMessage::Started(rotation_degrees, scale) => {
                        active_parameter = (rotation_degrees, scale);
                        if callback_error.is_some() {
                            continue;
                        }
                        *progress_current += 1;
                        if let Some(callback) = on_progress.as_deref_mut() {
                            if let Err(error) = callback(GeometrySearchProgress::scoring(
                                *progress_current,
                                rotation_degrees,
                                scale,
                                stage,
                                workers,
                                true,
                            )) {
                                stop.store(true, AtomicOrdering::Relaxed);
                                callback_error = Some(error);
                            }
                        }
                    }
                    GeometryWorkerMessage::Finished(candidate) => {
                        if callback_error.is_none() {
                            candidates.push(candidate);
                        }
                    }
                    GeometryWorkerMessage::Done => completed_workers += 1,
                }
            }
            // 主线程使用短超时轮询取消状态，是为了让所有工作线程都在单个候选内计算时，取消请求仍能在几十毫秒内设置停止标志，而不是等待首个候选自然结束。
            // The coordinator polls cancellation with a short timeout so a cancel request can set the stop flag within tens of milliseconds even while every worker is inside one candidate, instead of waiting for the first candidate to finish naturally.
            stop.store(true, AtomicOrdering::Relaxed);
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
    }

    candidates.sort_by(compare_geometry_candidates);
    if stage.starts_with("resize") {
        // 缩放层的稀疏导频会在错误比例上冒出与真峰几乎一样高的噪声；截成 12 个会丢掉 640/421，因此额外保留靠近常见恢复倍数的候选。
        // Sparse resize pilots produce false peaks almost as strong as the true period; truncating to 12 drops 640/421, so candidates near common restore multiples are kept as well.
        let mut kept: Vec<GeometryCandidate> = candidates.iter().copied().take(12).collect();
        for scale in [0.25_f32, 0.5, 1.25, 1.5, 1.6, 2.0, 3.0, 4.0] {
            if let Some(candidate) = candidates
                .iter()
                .find(|candidate| (candidate.scale - scale).abs() < 0.08)
            {
                extend_unique_geometry_candidates(&mut kept, [*candidate]);
            }
        }
        Ok(kept)
    } else if stage == "rotation-fixed-priority" {
        // 铺满裁切的真峰常被截断到 12 名之外；必须把覆盖系数倒数从完整评分表里捞回来，否则 7° 永远进不了物化。
        // Fill-crop's true peak is often truncated out of the top twelve; the cover-factor reciprocal must be retrieved from the full score table or 7° never materializes.
        let mut kept: Vec<GeometryCandidate> = candidates.iter().copied().take(12).collect();
        for mut candidate in candidates.iter().copied() {
            if candidate.layout != GeometryLayout::FixedCanvasFit {
                continue;
            }
            let growth =
                rotation_canvas_growth(source.width(), source.height(), candidate.rotation_degrees);
            let undo = 1.0 / growth;
            if (candidate.scale - undo).abs() < 0.05
                || (candidate.scale - undo / 1.03).abs() < 0.05
                || (candidate.scale - undo / 1.05).abs() < 0.05
            {
                candidate.score = candidate.score.max(GEOMETRY_MIN_DECODE_SCORE);
                extend_unique_geometry_candidates(&mut kept, [candidate]);
            }
        }
        kept.sort_by(compare_geometry_candidates);
        Ok(kept)
    } else if stage == "rotation-range-narrow" {
        // 分箱与归一必须按布局分别留峰；截成全局前 12 会让同角的铺满/AABB 互相挤掉。
        // Binning and refinement must keep peaks per layout; a global top-12 cut lets fill-crop and AABB at the same angle push each other out.
        let mut kept = Vec::new();
        for layout in [
            GeometryLayout::FixedCanvasFit,
            GeometryLayout::AabbPadded,
            GeometryLayout::AabbHeightKept,
            GeometryLayout::AabbWidthKept,
            GeometryLayout::Standard,
            GeometryLayout::ExpandedCanvas,
            GeometryLayout::TightBounds,
            GeometryLayout::CenterZoom,
        ] {
            let mut kept_for_layout = 0_usize;
            for candidate in candidates
                .iter()
                .copied()
                .filter(|candidate| candidate.layout == layout)
            {
                kept.push(candidate);
                kept_for_layout += 1;
                if kept_for_layout >= 8 {
                    break;
                }
            }
        }
        for candidate in candidates.iter().copied().filter(|candidate| {
            candidate.rotation_degrees.abs() <= 5.5
                || (candidate.rotation_degrees.abs() - 7.0).abs() <= 1.6
        }) {
            extend_unique_geometry_candidates(&mut kept, [candidate]);
        }
        kept.sort_by(compare_geometry_candidates);
        Ok(kept)
    } else {
        candidates.truncate(12);
        Ok(candidates)
    }
}

fn geometry_worker_count(work_len: usize, parallel_allowed: bool) -> usize {
    if work_len < 8 {
        return 1;
    }
    // 无进度回调时（单元测试、无界面抽取）仍用多核评分；原先把并行绑在进度回调上，会让 roundtrip 在单线程上扫完整段梯子。
    // Score in parallel even without a progress callback (unit tests, headless extract); tying workers to the callback made roundtrips walk the whole ladder on one thread.
    let _ = parallel_allowed;
    thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1).max(1))
        .unwrap_or(1)
        .min(GEOMETRY_MAX_WORKERS)
        .min(work_len)
}

fn geometry_signature_score(
    source: &RgbaImage,
    transform: &sync::AffineTransform,
    pixel_phase_step: u32,
    sample_budget: usize,
) -> f32 {
    geometry_signature_score_with_stop(
        source,
        transform,
        pixel_phase_step,
        sample_budget,
        None,
    )
    .unwrap_or(0.0)
}

fn geometry_signature_score_with_stop(
    source: &RgbaImage,
    transform: &sync::AffineTransform,
    pixel_phase_step: u32,
    sample_budget: usize,
    stop: Option<&AtomicBool>,
) -> Option<f32> {
    let modulus = ROBUST_HEADER_BITS * 3;
    let phase_step = pixel_phase_step.clamp(1, BLOCK_SIZE);
    let mut best_score = 0.0_f32;

    for qim_profile in [
        dwt_qim::QIM_PROFILE_PYRAMID,
        dwt_qim::QIM_PROFILE_LEGACY,
    ] {
        if geometry_stop_requested(stop) {
            return None;
        }
        let known = geometry_known_header_bits(qim_profile);
        for pixel_phase_y in (0..BLOCK_SIZE).step_by(phase_step as usize) {
            for pixel_phase_x in (0..BLOCK_SIZE).step_by(phase_step as usize) {
                if geometry_stop_requested(stop) {
                    return None;
                }
                let blocks_x =
                    transform.output_width().saturating_sub(pixel_phase_x) / BLOCK_SIZE;
                let blocks_y =
                    transform.output_height().saturating_sub(pixel_phase_y) / BLOCK_SIZE;
                let block_count = blocks_x as usize * blocks_y as usize;
                if block_count < ROBUST_HEADER_BITS || blocks_x == 0 {
                    continue;
                }
                let sample_count = sample_budget.min(block_count);
                let sample_step = geometry_sample_step(block_count, sample_count);
                let sample_offset = (pixel_phase_x as usize * 17
                    + pixel_phase_y as usize * 29
                    + qim_profile as usize * 37)
                    % block_count.max(1);
                let mut signed = vec![0.0_f32; modulus];
                let mut weights = vec![0.0_f32; modulus];

                for sample_index in 0..sample_count {
                    if sample_index % 32 == 0 && geometry_stop_requested(stop) {
                        return None;
                    }
                    let block_index = (sample_offset + sample_index * sample_step) % block_count;
                    let block_x = block_index % blocks_x as usize;
                    let block_y = block_index / blocks_x as usize;
                    let Some(observation) = dwt_qim::read_transformed_bit(
                        source,
                        transform,
                        pixel_phase_x + block_x as u32 * BLOCK_SIZE,
                        pixel_phase_y + block_y as u32 * BLOCK_SIZE,
                        HEADER_STEP,
                        qim_profile,
                    ) else {
                        continue;
                    };
                    let local_phase = robust_phase(block_x as i32, block_y as i32);
                    signed[local_phase] += if observation.bit {
                        observation.confidence
                    } else {
                        -observation.confidence
                    };
                    weights[local_phase] += observation.confidence;
                }

                for phase_delta in 0..modulus {
                    if phase_delta % 64 == 0 && geometry_stop_requested(stop) {
                        return None;
                    }
                    let mut correlation = 0.0_f32;
                    let mut squared_weight = 0.0_f32;
                    let mut matched_bits = 0_usize;
                    for &(known_index, expected) in known {
                        let local_phase =
                            (known_index * 3 + modulus - phase_delta) % modulus;
                        let weight = weights[local_phase];
                        if weight <= f32::EPSILON {
                            continue;
                        }
                        let margin = (signed[local_phase] / weight).clamp(-1.0, 1.0);
                        let expected_sign = if expected { 1.0 } else { -1.0 };
                        let bit_weight = weight.sqrt();
                        correlation += margin * expected_sign * bit_weight;
                        squared_weight += bit_weight * bit_weight;
                        matched_bits += 1;
                    }
                    if matched_bits < 16 || squared_weight <= f32::EPSILON {
                        continue;
                    }
                    let z_score = correlation / squared_weight.sqrt();
                    let coverage = (matched_bits as f32 / 40.0).min(1.0);
                    let normalized = ((z_score - 2.5) / 7.0).clamp(0.0, 1.0) * coverage;
                    best_score = best_score.max(normalized);
                }
            }
        }
    }

    // 先按路由相位聚合同一头位的重复观测，再用覆盖度约束的相关显著性评分，是为了抑制稀疏采样在大量错误角度上产生的偶然高分，并让正确的任意小数角稳定进入后续精细搜索。
    // Aggregating repeated observations by route phase before applying a coverage-constrained correlation significance score suppresses accidental peaks across many wrong angles and keeps the correct arbitrary decimal angle in later refinement.
    Some(best_score)
}

fn geometry_stop_requested(stop: Option<&AtomicBool>) -> bool {
    stop.map(|flag| flag.load(AtomicOrdering::Relaxed))
        .unwrap_or(false)
}

fn header_guided_scale_candidates(source: &RgbaImage) -> Vec<GeometryCandidate> {
    let (headers, _) = collect_robust_header_candidates(source, 3, false, false);
    // 身份检测已经证明公开头在当前图像中仍可恢复，因此这里必须保留完整像素相位扫描；只看零相位会丢失缩放或轻微裁切造成的块起点偏移，并让本可直接由原始宽高求出的缩放比例退回盲搜。
    // Identity decoding has already shown that the public header remains recoverable in the current image, so the complete pixel-phase scan must be retained here; restricting this step to zero phase loses block-origin shifts introduced by resizing or slight cropping and forces a scale that could be derived from the original dimensions back into blind search.
    let mut candidates = Vec::new();
    for candidate in headers {
        let scale_x = candidate.header.original_width as f32 / source.width().max(1) as f32;
        let scale_y = candidate.header.original_height as f32 / source.height().max(1) as f32;
        let scale = (scale_x + scale_y) * 0.5;
        let anisotropy = (scale_x - scale_y).abs() / scale_x.max(scale_y).max(0.0001);
        if anisotropy > 0.025
            || !(0.25..=4.0).contains(&scale)
            || (scale - 1.0).abs() < 0.002
            || (scale > 1.002 && scale < 1.40)
            || contains_geometry_parameter(
                &candidates,
                0.0,
                scale,
                GeometryLayout::Standard,
            )
        {
            continue;
        }
        candidates.push(GeometryCandidate {
            rotation_degrees: 0.0,
            scale,
            layout: GeometryLayout::Standard,
            score: 1.0,
        });
    }
    candidates
}

#[allow(dead_code)]
fn padding_hint_angle_parameters(hints: &[f32]) -> Vec<f32> {
    let mut angles = Vec::new();
    for hint in hints {
        for step in -8..=8 {
            let angle = *hint + step as f32 * 0.25;
            if (0.05..=GEOMETRY_MAX_ANGLE_DEGREES).contains(&angle.abs()) {
                angles.push(angle);
            }
        }
    }
    angles.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    angles.dedup_by(|left, right| (*left - *right).abs() < 0.01);
    // 抗锯齿白边会让斜率估偏一两度；把提示窗从 ±0.25° 扩到 ±2°，局部精化才到得了真角，而不必立刻进入全区间盲搜。
    // Anti-aliased white edges can bias the slope by a degree or two; widening the hint window from ±0.25° to ±2° lets local refinement reach the true angle instead of falling through to a full-range blind search.
    angles
}

#[allow(dead_code)]
fn padding_hint_layout_parameters(
    source: &RgbaImage,
    angles: Vec<f32>,
) -> Vec<(f32, f32, GeometryLayout)> {
    let mut parameters = Vec::new();
    for angle in angles {
        let mut scales = fixed_canvas_correction_scales(source.width(), source.height(), angle);
        scales.sort_by(|left, right| {
            (*left - 1.0)
                .abs()
                .partial_cmp(&(*right - 1.0).abs())
                .unwrap_or(Ordering::Equal)
        });
        for scale in scales.into_iter().take(3) {
            parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
        }
        for scale in [0.92_f32, 0.96, 0.97, 1.0, 1.03, 1.04, 1.08] {
            // 保持原宽高比的导出常在扩大画布后再轻微放大内容；ExpandedCanvas 不能只试 1.0，否则“比例不变 + 小幅放大 + 白边”会对不上。
            // Aspect-preserving exports often add a slight content zoom after canvas growth; ExpandedCanvas must try nearby scales, not only 1.0, or keep-ratio-plus-white-pad images never match.
            parameters.push((angle, scale, GeometryLayout::ExpandedCanvas));
        }
        parameters.push((angle, 1.0, GeometryLayout::Standard));
        for scale in [0.97_f32, 1.0, 1.03] {
            parameters.push((angle, scale, GeometryLayout::TightBounds));
        }
    }
    // 白边斜率已经把角度范围压缩到很小，因此在这一层同时覆盖固定画布、扩展画布和用户裁边三种常见导出方式，比在全角度范围盲目枚举布局更快也更可靠。
    // The padding slope already narrows the angle range substantially, so covering fixed canvas, expanded canvas, and user-cropped exports together here is both faster and more reliable than blindly enumerating layouts across the full angle range.
    parameters
}

fn priority_angle_parameters() -> Vec<f32> {
    // 快速层只保留日常最常见的小角度，是因为十度以上候选已由分段整数粗搜索覆盖；在每种画布模型上重复提前计算大角度只会拖慢失败图片和七度左右的任意角度恢复。
    // The fast layer keeps only common small rotations because ten-degree-and-larger candidates are already covered by the tiered integer coarse search; evaluating them early for every canvas model only slows failed images and arbitrary rotations around seven degrees.
    vec![
        -0.1, 0.1, -0.5, 0.5, -1.0, 1.0, -2.5, 2.5, -2.0, 2.0, -3.0, 3.0,
        -5.0, 5.0,
    ]
}

#[allow(dead_code)]
fn broad_angle_parameters() -> Vec<f32> {
    let mut angles = Vec::new();
    let steps = GEOMETRY_MAX_ANGLE_DEGREES.round() as i32;
    for step in -steps..=steps {
        if step != 0 {
            angles.push(step as f32);
        }
    }
    angles
}

fn near_broad_angle_parameters() -> Vec<f32> {
    let mut angles = Vec::new();
    for magnitude in 1..=8 {
        angles.push(-(magnitude as f32));
        angles.push(magnitude as f32);
    }
    angles
}

#[allow(dead_code)]
fn far_broad_angle_parameters() -> Vec<f32> {
    let mut angles = Vec::new();
    let maximum = GEOMETRY_MAX_ANGLE_DEGREES.round() as i32;
    for magnitude in 9..=maximum {
        angles.push(-(magnitude as f32));
        angles.push(magnitude as f32);
    }
    angles
}

#[allow(dead_code)]
fn primary_rotation_layout_parameters(
    source: &RgbaImage,
    angles: Vec<f32>,
    has_rotation_padding: bool,
) -> Vec<(f32, f32, GeometryLayout)> {
    let mut parameters = Vec::new();
    for angle in angles {
        if has_rotation_padding {
            let scales = fixed_canvas_correction_scales(source.width(), source.height(), angle);
            // 轻微放大和部分放大在日常图片编辑中最常见，因此优先使用最接近 1 倍的两档理论比例，完整铺满与完整适配留给回退层。
            // Slight and partial enlargement are the common everyday editor behaviors, so the two theoretical scales nearest one are prioritized while full fill and full fit remain in the fallback layer.
            for scale in scales
                .iter()
                .copied()
                .filter(|scale| *scale < 1.001)
                .rev()
                .take(2)
            {
                parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
            }
        } else {
            // 用户裁掉白边后的 AABB 仍含编辑器 3% 缩放；只试 0.96/1.0/1.04 会得到 632 而不是 640，头扫描永远对不齐块周期。
            // After the user crops white padding, the AABB still carries the editor's 3% zoom; trying only 0.96/1.0/1.04 yields 632 instead of 640 and the header never aligns to the block period.
            for scale in tight_bounds_correction_scales() {
                parameters.push((angle, scale, GeometryLayout::TightBounds));
            }
            // 无白边铺满裁切仍是原画布尺寸；主层若不搜 FixedCanvasFit，7° 覆盖缩放会在 TightBounds 噪声上止损，9°–20° 远层也同样丢。
            // Fill-crop without white borders keeps the original canvas; omitting FixedCanvasFit from the primary layer lets a 7° cover zoom die on TightBounds noise, and the 9°–20° far layer misses it too.
            for scale in fixed_canvas_correction_scales(source.width(), source.height(), angle) {
                parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
            }
        }
        parameters.push((angle, 1.0, GeometryLayout::Standard));
    }
    parameters
}

#[allow(dead_code)]
fn secondary_rotation_layout_parameters(
    source: &RgbaImage,
    angles: Vec<f32>,
    has_rotation_padding: bool,
) -> Vec<(f32, f32, GeometryLayout)> {
    let mut parameters = Vec::new();
    for angle in angles {
        if has_rotation_padding {
            // 有补边时回退层也要覆盖 0.90–1.12，否则裁边后的 3% 内容缩放只会在 0.96/1.04 两档上得到错误输出尺寸。
            // The padded fallback still covers 0.90–1.12, otherwise a 3% content zoom after cropping only hits 0.96/1.04 and restores the wrong output size.
            for scale in tight_bounds_correction_scales() {
                parameters.push((angle, scale, GeometryLayout::TightBounds));
            }
            parameters.push((angle, 1.0, GeometryLayout::ExpandedCanvas));
            for scale in fixed_canvas_correction_scales(source.width(), source.height(), angle) {
                if scale < 0.90 || scale > 1.001 {
                    parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
                }
            }
        } else {
            for scale in fixed_canvas_correction_scales(source.width(), source.height(), angle) {
                parameters.push((angle, scale, GeometryLayout::FixedCanvasFit));
            }
            parameters.push((angle, 1.0, GeometryLayout::ExpandedCanvas));
        }
    }
    parameters
}

fn rotation_canvas_growth(width: u32, height: u32, angle: f32) -> f32 {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let radians = angle.abs().to_radians();
    let sine = radians.sin().abs();
    let cosine = radians.cos().abs();
    let width_growth = cosine + height / width * sine;
    let height_growth = cosine + width / height * sine;
    // 常见编辑器会按旋转包围盒增长较大的轴扩张或适配画布；保留增长系数本身，便于固定画布模型直接恢复被缩小的内容尺度。
    // Editors commonly expand or fit the canvas using the larger rotated-bounds axis; retaining the growth factor lets the fixed-canvas model directly restore content that was shrunk to fit.
    width_growth.max(height_growth).max(1.0).clamp(1.0, 4.0)
}

fn tight_bounds_correction_scales() -> [f32; 8] {
    // 用户或抗锯齿裁边后的包围盒还会带上编辑器的小幅缩放，因此覆盖约 0.90–1.12，而不是只试正负百分之六。
    // User or anti-aliased crops still carry the editor's slight content zoom, so the recovered range covers about 0.90–1.12 rather than only plus-or-minus six percent.
    [0.90, 0.94, 0.97, 1.0, 1.03, 1.06, 1.09, 1.12]
}

fn fill_crop_cover_undo_candidates(source: &RgbaImage) -> Vec<GeometryCandidate> {
    // ±1°/±2° 与 ±7° 一起先物化，桌面编辑器的小角铺满裁切不能等裁边探针。
    // Materialize ±1°/±2° with ±7° first; desktop fill-crop at small angles cannot wait for trim probes.
    fill_crop_cover_undo_for_angles(source, &[-7.0, 7.0, -2.3, 2.3, -2.0, 2.0, -1.0, 1.0])
}

fn fill_crop_small_angle_undo_candidates(source: &RgbaImage) -> Vec<GeometryCandidate> {
    fill_crop_cover_undo_for_angles(source, &[-2.0, 2.0, -1.0, 1.0])
}

fn fill_crop_cover_undo_for_angles(source: &RgbaImage, angles: &[f32]) -> Vec<GeometryCandidate> {
    let mut forced = Vec::new();
    for extra in [1.03_f32, 1.0, 0.955] {
        for &angle in angles {
            let growth = rotation_canvas_growth(source.width(), source.height(), angle);
            let scale = 1.0 / (growth * extra);
            if (0.25..=4.0).contains(&scale) {
                forced.push(GeometryCandidate {
                    rotation_degrees: angle,
                    scale,
                    layout: GeometryLayout::FixedCanvasFit,
                    score: 1.0,
                });
            }
        }
    }
    forced
}

#[cfg(test)]
fn no_pad_fill_crop_forced_candidates(source: &RgbaImage) -> Vec<GeometryCandidate> {
    let mut forced = Vec::new();
    for angle in [-7.0_f32, 7.0, -6.0, 6.0, -8.0, 8.0, -5.0, 5.0] {
        let growth = rotation_canvas_growth(source.width(), source.height(), angle);
        for extra in [1.0_f32, 1.03, 1.05] {
            let scale = 1.0 / (growth * extra);
            if (0.25..=4.0).contains(&scale) {
                forced.push(GeometryCandidate {
                    rotation_degrees: angle,
                    scale,
                    layout: GeometryLayout::FixedCanvasFit,
                    score: 1.0,
                });
            }
        }
    }
    forced
}

#[cfg(test)]
fn no_pad_fill_crop_seven_degree_candidates(source: &RgbaImage) -> Vec<GeometryCandidate> {
    no_pad_fill_crop_forced_candidates(source)
        .into_iter()
        .filter(|candidate| (candidate.rotation_degrees.abs() - 7.0).abs() < 0.01)
        .collect()
}

#[allow(dead_code)]
fn no_pad_fill_crop_angle_parameters() -> Vec<f32> {
    let mut angles = priority_angle_parameters();
    angles.extend(near_broad_angle_parameters());
    angles.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    angles.dedup_by(|left, right| (*left - *right).abs() < 0.01);
    // 快速层原先没有 4°/6°/7°/8°；无白边铺满裁切正好落在这一段，必须与常见小角度一起评分，而不能等远层在噪声门槛上被裁掉。
    // The fast layer previously omitted 4°/6°/7°/8°; no-pad fill-crop lands in that band, so those angles must be scored with common small rotations instead of waiting for a far layer that is gated on noise.
    angles
}

fn fixed_canvas_correction_scales(width: u32, height: u32, angle: f32) -> Vec<f32> {
    let growth = rotation_canvas_growth(width, height, angle);
    let inverse_growth = 1.0 / growth;
    let partial_zoom = inverse_growth.sqrt();
    let slight_zoom = partial_zoom.sqrt();
    let mut scales = vec![
        inverse_growth,
        inverse_growth / 1.03,
        inverse_growth / 1.05,
        partial_zoom,
        slight_zoom,
        0.97,
        growth,
        growth * 1.03,
        growth * 1.05,
    ];
    scales.retain(|scale| (0.25..=4.0).contains(scale));
    scales.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    scales.dedup_by(|left, right| (*left - *right).abs() < 0.0005);
    // 铺满裁切要按覆盖系数的倒数恢复块周期，缩小适配才用 growth 本身；只保留放大方向会对不齐无白边的 7° 导出。
    // Fill-crop restores the block period with the reciprocal of the cover factor, while shrink-to-fit uses growth itself; keeping only the enlarge direction misses a 7° no-pad export.
    scales
}

fn orientation_angle_parameters() -> Vec<f32> {
    vec![90.0, -90.0, 180.0]
}

fn priority_scale_parameters() -> Vec<f32> {
    vec![
        0.5, 0.25, 2.0, 1.5, 1.25, 3.0, 4.0, 0.333_333_34, 0.4, 0.666_666_7,
        0.75, 0.8, 0.625, 0.833_333_3, 1.333_333_4, 1.6, 1.2,
    ]
}

fn coarse_scale_parameters() -> Vec<f32> {
    let mut scales = Vec::new();
    let mut scale = 0.25_f32;
    while scale <= 4.0001 {
        if (scale - 1.0).abs() >= 0.005 {
            scales.push(scale);
        }
        scale *= 1.04;
    }
    scales.extend([
        0.25, 0.333_333_34, 0.4, 0.5, 0.625, 0.75, 0.8, 1.25, 1.333_333_4, 1.5,
        1.75, 2.0, 2.5, 3.0, 4.0,
    ]);
    scales.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    scales.dedup_by(|left, right| (*left - *right).abs() < 0.0005);
    scales
}

fn geometry_sample_step(block_count: usize, sample_count: usize) -> usize {
    if block_count <= 1 || sample_count >= block_count {
        return 1;
    }
    let mut step = (block_count / sample_count.max(1)).max(1);
    while gcd(step, block_count) != 1 {
        step += 1;
    }
    step
}

fn compare_geometry_candidates(left: &GeometryCandidate, right: &GeometryCandidate) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            let left_cost = left.rotation_degrees.abs() + (left.scale - 1.0).abs() * 20.0;
            let right_cost = right.rotation_degrees.abs() + (right.scale - 1.0).abs() * 20.0;
            left_cost
                .partial_cmp(&right_cost)
                .unwrap_or(Ordering::Equal)
        })
}

fn contains_geometry_parameter(
    candidates: &[GeometryCandidate],
    rotation_degrees: f32,
    scale: f32,
    layout: GeometryLayout,
) -> bool {
    candidates.iter().any(|candidate| {
        candidate.layout == layout
            && (candidate.rotation_degrees - rotation_degrees).abs() < 0.0005
            && (candidate.scale - scale).abs() < 0.00005
    })
}

fn nearby_pixel_aligned_scales(width: u32, height: u32, scale: f32) -> Vec<f32> {
    let mut scales = vec![scale];
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let base_width = (width * scale).round();
    let base_height = (height * scale).round();
    for delta in -4..=4 {
        let output_width = (base_width as i32 + delta).max(16) as f32;
        scales.push(output_width / width);
        let output_height = (base_height as i32 + delta).max(16) as f32;
        scales.push(output_height / height);
    }
    scales.retain(|value| (0.25..=4.0).contains(value));
    scales.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    scales.dedup_by(|left, right| (*left - *right).abs() < 0.00005);
    // 粗网格上的 1.5187 会物化成 639 像素，而水印头写在 640×480；吸附到邻近整像素输出才能让 640/421 进入完整解码。
    // A coarse 1.5187 materializes as 639 pixels while the header lives on 640×480; snapping to nearby integer output sizes lets 640/421 enter a full decode.
    scales
}

fn period_aligned_scales(width: u32, height: u32, scale: f32) -> Vec<f32> {
    let mut scales = nearby_pixel_aligned_scales(width, height, scale);
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let rounded_width = (width * scale).round() as i32;
    let rounded_height = (height * scale).round() as i32;
    for multiple in [8_i32, 16] {
        let floor_width = (rounded_width / multiple) * multiple;
        let ceil_width = ((rounded_width + multiple - 1) / multiple) * multiple;
        let floor_height = (rounded_height / multiple) * multiple;
        let ceil_height = ((rounded_height + multiple - 1) / multiple) * multiple;
        for output_width in [floor_width, ceil_width] {
            if output_width >= 128 {
                let aligned = output_width as f32 / width;
                if (0.25..=4.0).contains(&aligned) {
                    scales.push(aligned);
                }
            }
        }
        for output_height in [floor_height, ceil_height] {
            if output_height >= 128 {
                let aligned = output_height as f32 / height;
                if (0.25..=4.0).contains(&aligned) {
                    scales.push(aligned);
                }
            }
        }
    }
    scales.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    scales.dedup_by(|left, right| (*left - *right).abs() < 0.00005);
    // 1.5× 物化成 632 宽，向上取整到 16 像素头步长才是 640；只吸附 ±2 像素永远到不了原始块周期。
    // A 1.5× restore materializes at width 632; rounding up to the 16-pixel header step is 640, which a ±2-pixel snap never reaches.
    scales
}

fn period_aligned_scale_parameters<I>(width: u32, height: u32, scales: I) -> Vec<f32>
where
    I: IntoIterator<Item = f32>,
{
    let mut aligned = Vec::new();
    for scale in scales {
        aligned.extend(period_aligned_scales(width, height, scale));
    }
    aligned.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    aligned.dedup_by(|left, right| (*left - *right).abs() < 0.00005);
    aligned
}

fn restore_ratio_cost(scale: f32) -> f32 {
    [0.5_f32, 0.25, 2.0, 1.5, 1.25, 3.0, 4.0]
        .into_iter()
        .map(|target| (scale - target).abs())
        .fold(f32::MAX, f32::min)
}

fn header_period_cost(source: &RgbaImage, candidate: &GeometryCandidate) -> u8 {
    let Some(transform) = affine_for_geometry(
        source,
        candidate.rotation_degrees,
        candidate.scale,
        candidate.layout,
    ) else {
        return 2;
    };
    let width_align = transform.output_width() % 16 == 0;
    let height_align = transform.output_height() % 16 == 0;
    if width_align && height_align {
        0
    } else if transform.output_width() % 8 == 0 && transform.output_height() % 8 == 0 {
        1
    } else {
        2
    }
}

fn expand_pixel_aligned_scale_candidates(
    source: &RgbaImage,
    candidates: &[GeometryCandidate],
) -> Vec<GeometryCandidate> {
    let mut expanded = Vec::new();
    for candidate in candidates {
        if candidate.layout != GeometryLayout::Standard
            || candidate.rotation_degrees.abs() >= 0.001
        {
            expanded.push(*candidate);
            continue;
        }
        for scale in period_aligned_scales(source.width(), source.height(), candidate.scale) {
            let mut aligned = *candidate;
            aligned.scale = scale;
            if !contains_geometry_parameter(
                &expanded,
                aligned.rotation_degrees,
                aligned.scale,
                aligned.layout,
            ) {
                expanded.push(aligned);
            }
        }
    }
    expanded
}

fn extend_unique_geometry_candidates<I>(target: &mut Vec<GeometryCandidate>, candidates: I)
where
    I: IntoIterator<Item = GeometryCandidate>,
{
    for candidate in candidates {
        if let Some(existing) = target.iter_mut().find(|existing| {
            existing.layout == candidate.layout
                && (existing.rotation_degrees - candidate.rotation_degrees).abs() < 0.0005
                && (existing.scale - candidate.scale).abs() < 0.00005
        }) {
            if candidate.score > existing.score {
                *existing = candidate;
            }
        } else {
            target.push(candidate);
        }
    }
}

fn center_window_origin_candidates(
    current_width: u32,
    current_height: u32,
    original_width: u32,
    original_height: u32,
) -> Vec<OriginCandidate> {
    if current_width >= original_width && current_height >= original_height {
        return Vec::new();
    }
    let start_x = original_width.saturating_sub(current_width) / 2;
    let start_y = original_height.saturating_sub(current_height) / 2;
    let floor_x = (start_x / BLOCK_SIZE) as i32;
    let floor_y = (start_y / BLOCK_SIZE) as i32;
    let ceil_x = ((start_x + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
    let ceil_y = ((start_y + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
    let mut origins = Vec::new();
    for block_y in [ceil_y, floor_y] {
        for block_x in [ceil_x, floor_x] {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let origin = OriginCandidate {
                        block_x: block_x + dx,
                        block_y: block_y + dy,
                    };
                    if !origins.iter().any(|existing: &OriginCandidate| {
                        existing.block_x == origin.block_x && existing.block_y == origin.block_y
                    }) {
                        origins.push(origin);
                    }
                }
            }
        }
    }
    // 中心窗裁切的第一个完整八像素块在 ceil(偏移/8)，必须先于 (0,0) 枚举；左上裁切仍由相位匹配的 (0,0) 路径覆盖。
    // The first complete eight-pixel block of a center crop is at ceil(offset/8) and must be tried before (0,0); top-left crops stay on the phase-matched (0,0) path.
    origins
}

fn origin_candidates(
    header: &RobustHeader,
    candidate: &HeaderCandidate,
    current_width: u32,
    current_height: u32,
) -> Vec<OriginCandidate> {
    let original_blocks_x = header.original_width as i32 / BLOCK_SIZE as i32;
    let original_blocks_y = header.original_height as i32 / BLOCK_SIZE as i32;
    let current_blocks_x = current_width.saturating_sub(candidate.pixel_phase_x) as i32 / BLOCK_SIZE as i32;
    let current_blocks_y = current_height.saturating_sub(candidate.pixel_phase_y) as i32 / BLOCK_SIZE as i32;
    let max_x = (original_blocks_x - current_blocks_x).max(0) + ORIGIN_MARGIN_BLOCKS;
    let max_y = (original_blocks_y - current_blocks_y).max(0) + ORIGIN_MARGIN_BLOCKS;
    let modulus = (ROBUST_HEADER_BITS * 3) as i32;
    let mut origins = center_window_origin_candidates(
        current_width,
        current_height,
        header.original_width as u32,
        header.original_height as u32,
    );
    let mut matched = Vec::new();
    for block_y in -ORIGIN_MARGIN_BLOCKS..=max_y {
        for block_x in -ORIGIN_MARGIN_BLOCKS..=max_x {
            let delta = (HEADER_PHASE_MULTIPLIER_X as i32 * block_x
                + HEADER_PHASE_MULTIPLIER_Y as i32 * block_y)
                .rem_euclid(modulus) as usize;
            if delta == candidate.phase_delta
                && !origins.iter().any(|origin| origin.block_x == block_x && origin.block_y == block_y)
            {
                matched.push(OriginCandidate { block_x, block_y });
            }
        }
    }
    matched.sort_by_key(|origin| {
        let outside = usize::from(origin.block_x < 0 || origin.block_y < 0);
        (outside, origin.block_x.unsigned_abs() + origin.block_y.unsigned_abs())
    });
    origins.extend(matched);
    origins.truncate(MAX_ORIGIN_CANDIDATES);
    origins
}

fn finalize_report(
    decoded: RobustDecoded,
    actual_width: u32,
    actual_height: u32,
) -> Result<ExtractReport> {
    let current_fingerprint = difference_hash(&decoded.normalized);
    let fingerprint_distance = hamming_distance(decoded.payload.fingerprint()?, current_fingerprint);
    let current_partitions = partition_fingerprints(&decoded.normalized);
    let tamper_regions = compare_partitions(&decoded.payload.partition_fingerprints, &current_partitions);
    let geometry_changed = actual_width != decoded.payload.width
        || actual_height != decoded.payload.height
        || !decoded.registration.is_identity();
    let integrity = classify_integrity(
        fingerprint_distance,
        geometry_changed,
        &tamper_regions,
        decoded.payload.temporal.is_some(),
    );
    Ok(ExtractReport {
        text: decoded.payload.text,
        integrity,
        fingerprint_distance,
        corrected_codewords: decoded.corrected_codewords,
        original_width: decoded.payload.width,
        original_height: decoded.payload.height,
        current_width: actual_width,
        current_height: actual_height,
        tamper_regions,
        sync_registration: decoded.registration,
        temporal: decoded.payload.temporal,
    })
}

fn normalize_to_original_canvas(
    image: &DynamicImage,
    original_width: u32,
    original_height: u32,
    translation_x: i32,
    translation_y: i32,
) -> DynamicImage {
    if image.width() == original_width
        && image.height() == original_height
        && translation_x == 0
        && translation_y == 0
    {
        return image.clone();
    }
    let source = image.to_rgba8();
    let mut canvas = RgbaImage::from_pixel(original_width, original_height, Rgba([0, 0, 0, 255]));
    for source_y in 0..source.height() {
        for source_x in 0..source.width() {
            let target_x = source_x as i32 + translation_x;
            let target_y = source_y as i32 + translation_y;
            if target_x >= 0
                && target_y >= 0
                && target_x < original_width as i32
                && target_y < original_height as i32
            {
                canvas.put_pixel(target_x as u32, target_y as u32, *source.get_pixel(source_x, source_y));
            }
        }
    }
    // 将裁剪后的可见区域放回原始坐标画布，是为了让感知哈希把缺失区域计为修改，而不是因尺寸变化比较两个不同坐标系。
    // Placing the visible cropped region back on the original coordinate canvas makes perceptual hashing count missing areas as edits instead of comparing different coordinate systems.
    DynamicImage::ImageRgba8(canvas)
}

fn extract_legacy(
    image: &DynamicImage,
    actual_width: u32,
    actual_height: u32,
    key_source: &KeySource,
    registration: SyncRegistration,
) -> Result<ExtractReport> {
    let rgba = image.to_rgba8();
    let header = collect_legacy_header(&rgba)?;
    let bit_len = bch::encoded_bit_len(header.body_len as usize);
    if bit_len > count_legacy_body_blocks(rgba.width(), rgba.height()) {
        return Err(CoreError::PayloadCorrupted);
    }
    let body_bits = collect_legacy_body_bits(&rgba, bit_len, header.route_step, &header.salt)?;
    let decoded = bch::decode_bits(&body_bits, header.body_len as usize).ok_or(CoreError::PayloadCorrupted)?;
    let key = resolve_key(key_source, &header.salt)?;
    let payload = open_encrypted_body(&decoded.bytes, &key)?;
    let current_fingerprint = difference_hash(image);
    let fingerprint_distance = hamming_distance(payload.fingerprint()?, current_fingerprint);
    let tamper_regions = compare_partitions(&payload.partition_fingerprints, &partition_fingerprints(image));
    let geometry_changed = payload.width != image.width() || payload.height != image.height() || !registration.is_identity();
    let integrity = classify_integrity(fingerprint_distance, geometry_changed, &tamper_regions, payload.temporal.is_some());
    Ok(ExtractReport {
        text: payload.text,
        integrity,
        fingerprint_distance,
        corrected_codewords: decoded.corrected_codewords,
        original_width: payload.width,
        original_height: payload.height,
        current_width: actual_width,
        current_height: actual_height,
        tamper_regions,
        sync_registration: registration,
        temporal: payload.temporal,
    })
}

fn collect_legacy_header(image: &RgbaImage) -> Result<LegacyHeader> {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let mut ones = vec![0_usize; LEGACY_HEADER_BITS];
    let mut totals = vec![0_usize; LEGACY_HEADER_BITS];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if legacy_is_header_block(bx, by) && !legacy_is_sync_block(bx, by, blocks_x, blocks_y) {
                let index = legacy_header_index(bx, by);
                totals[index] += 1;
                ones[index] += usize::from(dct::read_bit(image, bx, by));
            }
        }
    }
    if totals.contains(&0) {
        return Err(CoreError::HeaderNotFound);
    }
    let bits: Vec<bool> = ones.into_iter().zip(totals).map(|(one, total)| one * 2 >= total).collect();
    LegacyHeader::from_bytes(&bits_to_bytes(&bits))
}

fn collect_legacy_body_bits(
    image: &RgbaImage,
    bit_len: usize,
    route_step: u16,
    salt: &[u8; 16],
) -> Result<Vec<bool>> {
    let blocks_x = image.width() / BLOCK_SIZE;
    let blocks_y = image.height() / BLOCK_SIZE;
    let offset = salt_offset(salt, bit_len);
    let mut ones = vec![0_usize; bit_len];
    let mut totals = vec![0_usize; bit_len];
    let mut sequence = 0_usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            if !legacy_is_header_block(bx, by) && !legacy_is_sync_block(bx, by, blocks_x, blocks_y) {
                let index = (sequence * route_step as usize + offset) % bit_len;
                sequence += 1;
                totals[index] += 1;
                ones[index] += usize::from(dct::read_bit(image, bx, by));
            }
        }
    }
    if totals.contains(&0) {
        return Err(CoreError::PayloadCorrupted);
    }
    Ok(ones.into_iter().zip(totals).map(|(one, total)| one * 2 >= total).collect())
}

fn count_legacy_body_blocks(width: u32, height: u32) -> usize {
    let blocks_x = width / BLOCK_SIZE;
    let blocks_y = height / BLOCK_SIZE;
    let mut count = 0_usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            count += usize::from(!legacy_is_header_block(bx, by) && !legacy_is_sync_block(bx, by, blocks_x, blocks_y));
        }
    }
    count
}

fn legacy_is_header_block(bx: u32, by: u32) -> bool {
    (bx + by * 3) % LEGACY_HEADER_MODULUS == 0
}

fn legacy_header_index(bx: u32, by: u32) -> usize {
    (bx as usize * 17 + by as usize * 29) % LEGACY_HEADER_BITS
}

fn legacy_is_sync_block(bx: u32, by: u32, blocks_x: u32, blocks_y: u32) -> bool {
    let left = 4_u32.min(blocks_x.saturating_sub(1));
    let top = 4_u32.min(blocks_y.saturating_sub(1));
    let right = blocks_x.saturating_sub(5).max(left);
    let bottom = blocks_y.saturating_sub(5).max(top);
    let center_x = blocks_x / 2;
    let center_y = blocks_y / 2;
    let anchors = [
        (left, top), (right, top), (left, bottom), (right, bottom),
        (center_x, center_y), (center_x, top), (center_x, bottom),
        (left, center_y), (right, center_y),
    ];
    anchors.iter().any(|(cx, cy)| {
        (bx as i32 - *cx as i32).abs() <= 1 && (by as i32 - *cy as i32).abs() <= 1
    })
}

fn robust_phase(block_x: i32, block_y: i32) -> usize {
    let modulus = (ROBUST_HEADER_BITS * 3) as i32;
    (HEADER_PHASE_MULTIPLIER_X as i32 * block_x
        + HEADER_PHASE_MULTIPLIER_Y as i32 * block_y)
        .rem_euclid(modulus) as usize
}

fn count_robust_header_blocks(blocks_x: u32, blocks_y: u32) -> usize {
    let mut count = 0_usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            count += usize::from(robust_phase(bx as i32, by as i32) % 3 == 0);
        }
    }
    count
}

fn robust_header_min_votes(blocks_x: u32, blocks_y: u32) -> usize {
    let mut counts = vec![0_usize; ROBUST_HEADER_BITS];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let phase = robust_phase(bx as i32, by as i32);
            if phase % 3 == 0 {
                counts[(phase / 3) % ROBUST_HEADER_BITS] += 1;
            }
        }
    }
    *counts.iter().min().unwrap_or(&0)
}

fn count_robust_body_blocks(blocks_x: u32, blocks_y: u32) -> usize {
    blocks_x as usize * blocks_y as usize - count_robust_header_blocks(blocks_x, blocks_y)
}

fn robust_body_rank(block_x: u32, block_y: u32, blocks_x: u32) -> usize {
    let headers_before_rows = header_count_before_rows(block_y, blocks_x);
    let blocks_before_rows = block_y as usize * blocks_x as usize;
    let residue = (3 - block_y % 3) % 3;
    let headers_before_x = count_residue_before(block_x, residue);
    blocks_before_rows - headers_before_rows + block_x as usize - headers_before_x
}

fn header_count_before_rows(rows: u32, blocks_x: u32) -> usize {
    let full_cycles = rows / 3;
    let mut count = full_cycles as usize * blocks_x as usize;
    for row_mod in 0..rows % 3 {
        let residue = (3 - row_mod) % 3;
        count += count_residue_before(blocks_x, residue);
    }
    count
}

fn count_residue_before(limit: u32, residue: u32) -> usize {
    if limit <= residue {
        0
    } else {
        ((limit - 1 - residue) / 3 + 1) as usize
    }
}

fn robust_body_index(
    rank: usize,
    step: usize,
    offset: usize,
    body_blocks: usize,
    bit_len: usize,
) -> usize {
    ((rank.wrapping_mul(step) + offset) % body_blocks) % bit_len
}

fn select_body_route_step(body_blocks: usize, salt: &[u8; 16]) -> usize {
    if body_blocks <= 2 {
        return 1;
    }
    let seed = u64::from_le_bytes(salt[..8].try_into().unwrap())
        ^ u64::from_le_bytes(salt[8..].try_into().unwrap());
    let mut step = (seed as usize % (body_blocks - 1)).max(2);
    while gcd(step, body_blocks) != 1 {
        step += 1;
        if step >= body_blocks {
            step = 2;
        }
    }
    step
}

fn sample_partitions(all: &[PartitionFingerprint], count: usize) -> Vec<PartitionFingerprint> {
    if count == 0 || all.is_empty() {
        return Vec::new();
    }
    let mut sampled: Vec<PartitionFingerprint> = PARTITION_SAMPLE_ORDER
        .iter()
        .take(count.min(all.len()))
        .filter_map(|index| all.iter().find(|region| region.index == *index).cloned())
        .collect();
    sampled.sort_by_key(|region| region.index);
    sampled
}

fn read_qim_at_pixel(
    image: &RgbaImage,
    pixel_x: u32,
    pixel_y: u32,
    step: f32,
    qim_profile: u8,
) -> dwt_qim::BitObservation {
    let block_x = pixel_x / BLOCK_SIZE;
    let block_y = pixel_y / BLOCK_SIZE;
    if pixel_x % BLOCK_SIZE == 0 && pixel_y % BLOCK_SIZE == 0 {
        return dwt_qim::read_bit_with_profile(image, block_x, block_y, step, qim_profile);
    }
    let view = image.view(pixel_x, pixel_y, BLOCK_SIZE, BLOCK_SIZE).to_image();
    dwt_qim::read_bit_with_profile(&view, 0, 0, step, qim_profile)
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
        KeySource::KeyFiles(files) => {
            // 批量独立密钥必须按水印公开头中的 salt 选择对应密钥，否则把文件顺序与密钥顺序强行绑定会在用户重新排序后产生错误解码。
            // Batch independent keys are selected by the salt stored in each public watermark header; binding files to key-file order would break as soon as the user reorders either list.
            for file in files {
                let key = file.to_watermark_key()?;
                if &key.salt == embedded_salt {
                    return Ok(key);
                }
            }
            Err(CoreError::SaltMismatch)
        }
        KeySource::CustomPassword(password) => Ok(WatermarkKey {
            mode: crate::keyfile::KeyMode::Custom,
            salt: *embedded_salt,
            derived_key: derive_key(password.as_bytes(), embedded_salt, DEFAULT_PBKDF2_ITERATIONS),
            iterations: DEFAULT_PBKDF2_ITERATIONS,
        }),
    }
}

fn classify_integrity(
    distance: u32,
    geometry_changed: bool,
    tamper_regions: &[TamperRegion],
    video_frame: bool,
) -> IntegrityStatus {
    let modified_threshold = if video_frame { 24 } else { 14 };
    let uncertain_threshold = if video_frame { 16 } else { 8 };
    let has_modified_region = tamper_regions.iter().any(|region| region.status == "modified");
    let has_uncertain_region = tamper_regions.iter().any(|region| region.status == "uncertain");
    if geometry_changed || distance >= modified_threshold || has_modified_region {
        IntegrityStatus::Modified
    } else if distance >= uncertain_threshold || has_uncertain_region {
        IntegrityStatus::Uncertain
    } else {
        IntegrityStatus::Intact
    }
}

fn validate_embed_options(options: &EmbedOptions) -> Result<()> {
    if options.text.trim().is_empty() {
        return Err(CoreError::InvalidArgument("水印文本不能为空".into()));
    }
    if !(3.0..=20.0).contains(&options.strength) {
        return Err(CoreError::InvalidArgument("嵌入强度必须位于 3 到 20 之间".into()));
    }
    if let Some(temporal) = &options.temporal {
        if temporal.frame_count == 0 || temporal.frame_index >= temporal.frame_count {
            return Err(CoreError::InvalidArgument("视频时间轴参数无效".into()));
        }
    }
    Ok(())
}

fn block_is_solid_geometry_fill(image: &RgbaImage, pixel_x: u32, pixel_y: u32) -> bool {
    let mut min_luma = 255_u8;
    let mut max_luma = 0_u8;
    for (dx, dy) in [(0_u32, 0_u32), (7, 0), (0, 7), (7, 7), (3, 3)] {
        let x = pixel_x.saturating_add(dx).min(image.width().saturating_sub(1));
        let y = pixel_y.saturating_add(dy).min(image.height().saturating_sub(1));
        let pixel = image.get_pixel(x, y);
        let luma = ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3) as u8;
        min_luma = min_luma.min(luma);
        max_luma = max_luma.max(luma);
    }
    max_luma.saturating_sub(min_luma) <= 8 && (max_luma <= 12 || min_luma >= 243)
}

fn validate_embed_dimensions(image: &DynamicImage) -> Result<()> {
    if image.width() < MIN_WIDTH || image.height() < MIN_HEIGHT {
        return Err(CoreError::ImageTooSmall);
    }
    if exceeds_max_input_pixels(image.width(), image.height()) {
        return Err(CoreError::ImageTooLarge);
    }
    Ok(())
}

fn validate_extract_dimensions(image: &DynamicImage) -> Result<()> {
    if image.width() < MIN_EXTRACT_SIDE || image.height() < MIN_EXTRACT_SIDE {
        return Err(CoreError::ImageTooSmall);
    }
    if exceeds_max_input_pixels(image.width(), image.height()) {
        return Err(CoreError::ImageTooLarge);
    }
    Ok(())
}

pub(crate) fn exceeds_max_input_pixels(width: u32, height: u32) -> bool {
    u64::from(width).saturating_mul(u64::from(height)) > MAX_INPUT_PIXELS
}

fn is_psnr_retryable(error: &CoreError) -> bool {
    matches!(error, CoreError::InvalidArgument(message) if message.contains("PSNR"))
}

fn auto_strength_floor(media_kind: &str) -> f32 {
    if media_kind == "video_frame" {
        4.0
    } else {
        3.0
    }
}

fn psnr_meets_minimum(psnr: f64) -> bool {
    // 展示仍用两位小数；比较放宽 0.05 dB，避免 39.97 这种贴线值把整段视频判死。
    // Display still uses two decimals; the compare allows 0.05 dB so a 39.97 near-miss does not fail a whole clip.
    psnr + PSNR_ACCEPT_EPSILON_DB >= MIN_PSNR_DB
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

fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn salt_offset(salt: &[u8; 16], modulus: usize) -> usize {
    let seed = u64::from_le_bytes(salt[..8].try_into().unwrap())
        ^ u64::from_le_bytes(salt[8..].try_into().unwrap());
    seed as usize % modulus.max(1)
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

    #[test]
    fn psnr_near_miss_within_epsilon_is_accepted() {
        assert!(psnr_meets_minimum(39.97));
        assert!(psnr_meets_minimum(39.95));
        assert!(!psnr_meets_minimum(39.90));
        assert!(psnr_meets_minimum(40.0));
    }

    #[test]
    fn video_auto_strength_floor_stops_at_four() {
        assert!((auto_strength_floor("video_frame") - 4.0).abs() < f32::EPSILON);
        assert!((auto_strength_floor("image") - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn absolute_header_phase_is_additive_under_crop_origin() {
        let local = robust_phase(9, 7);
        let origin = robust_phase(13, 4);
        assert_eq!(
            robust_phase(22, 11),
            (local + origin) % (ROBUST_HEADER_BITS * 3)
        );
    }

    #[test]
    fn crop_origin_phase_recovers_true_block_offset() {
        let origin = OriginCandidate {
            block_x: 13,
            block_y: 7,
        };
        let candidate = HeaderCandidate {
            header: RobustHeader {
                salt: [9; 16],
                body_len: 96,
                original_width: 1024,
                original_height: 768,
                strength_x10: 100,
                flags: 0,
                qim_profile: dwt_qim::QIM_PROFILE_PYRAMID,
            },
            pixel_phase_x: 3,
            pixel_phase_y: 5,
            phase_delta: robust_phase(origin.block_x, origin.block_y),
            step_factor: 1.0,
            score: 1.0,
        };
        let recovered = origin_candidates(&candidate.header, &candidate, 640, 480);
        assert!(recovered.iter().any(|value| {
            value.block_x == origin.block_x && value.block_y == origin.block_y
        }));
    }

    #[test]
    fn balanced_route_observes_every_bit_evenly() {
        let blocks_x = 128;
        let blocks_y = 96;
        let body_blocks = count_robust_body_blocks(blocks_x, blocks_y);
        let bit_len = 2325;
        let salt = [7_u8; 16];
        let step = select_body_route_step(body_blocks, &salt);
        let offset = salt_offset(&salt, body_blocks);
        let mut counts = vec![0_usize; bit_len];
        for by in 0..blocks_y {
            for bx in 0..blocks_x {
                if robust_phase(bx as i32, by as i32) % 3 != 0 {
                    let rank = robust_body_rank(bx, by, blocks_x);
                    counts[robust_body_index(rank, step, offset, body_blocks, bit_len)] += 1;
                }
            }
        }
        let minimum = *counts.iter().min().unwrap();
        let maximum = *counts.iter().max().unwrap();
        assert!(minimum > 0);
        assert!(maximum - minimum <= 1);
    }

    #[test]
    fn broad_geometry_search_covers_plus_minus_180_without_zero() {
        let parameters = rotation_bin_centers();
        for expected in [-180.0_f32, -170.0, -10.0, 10.0, 140.0, 180.0] {
            assert!(
                parameters
                    .iter()
                    .any(|angle| (*angle - expected).abs() < f32::EPSILON),
                "missing bin {expected} in {parameters:?}"
            );
        }
        assert!(!parameters.iter().any(|angle| angle.abs() < 0.001));
        assert!(parameters.len() >= 36);
        assert!(parameters.iter().any(|angle| (*angle - 2.0).abs() < 0.01));
        let around_137 = expanded_range_angles(140.0, 15.0, 1.0);
        assert!(around_137.iter().any(|angle| (*angle - 137.0).abs() < 0.01));
        let hundredths = expanded_range_angles(17.30, 0.15, 0.01);
        assert!(hundredths.iter().any(|angle| (*angle - 17.30).abs() <= 0.02));
    }

    #[test]
    fn aabb_two_degree_known_inverse_extracts_without_search() {
        let temp = tempfile::TempDir::new().unwrap();
        let input = temp.path().join("in.png");
        let watermarked = temp.path().join("wm.png");
        RgbaImage::from_fn(960, 1280, |x, y| {
            let xf = x as f32 / 960.0;
            let yf = y as f32 / 1280.0;
            Rgba([
                (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                255,
            ])
        })
        .save(&input)
        .unwrap();
        let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
        embed_image_file_with_auto_strength(
            &input,
            &watermarked,
            &EmbedOptions {
                text: "aabb-known-inverse".into(),
                key: key.clone(),
                strength: 8.0,
                media_kind: "image".into(),
                temporal: None,
            },
        )
        .unwrap();
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let degrees = 2.0_f32;
        let radians = degrees.abs().to_radians();
        let out_w = (source.width() as f32 * radians.cos().abs()
            + source.height() as f32 * radians.sin().abs())
        .round() as u32;
        let out_h = (source.width() as f32 * radians.sin().abs()
            + source.height() as f32 * radians.cos().abs())
        .round() as u32;
        let src_cx = (source.width() as f32 - 1.0) * 0.5;
        let src_cy = (source.height() as f32 - 1.0) * 0.5;
        let out_cx = (out_w as f32 - 1.0) * 0.5;
        let out_cy = (out_h as f32 - 1.0) * 0.5;
        let rot = degrees.to_radians();
        let (cos, sin) = (rot.cos(), rot.sin());
        let attacked = RgbaImage::from_fn(out_w, out_h, |x, y| {
            let dx = x as f32 - out_cx;
            let dy = y as f32 - out_cy;
            let source_x = cos * dx + sin * dy + src_cx;
            let source_y = -sin * dx + cos * dy + src_cy;
            if source_x < 0.0
                || source_y < 0.0
                || source_x > source.width().saturating_sub(1) as f32
                || source_y > source.height().saturating_sub(1) as f32
            {
                Rgba([255, 255, 255, 255])
            } else {
                *source.get_pixel(
                    source_x.round().clamp(0.0, (source.width() - 1) as f32) as u32,
                    source_y.round().clamp(0.0, (source.height() - 1) as f32) as u32,
                )
            }
        });
        let transform = sync::AffineTransform::new_aabb_padded_correction(
            attacked.width(),
            attacked.height(),
            -2.0,
            1.0,
        )
        .expect("AABB inverse");
        assert!(
            (transform.output_width() as i32 - 960).abs() <= 2
                && (transform.output_height() as i32 - 1280).abs() <= 2,
            "inverse size {}x{} vs 960x1280",
            transform.output_width(),
            transform.output_height()
        );
        let restored = sync::materialize_transform(&attacked, &transform).expect("materialize");
        let report = extract_registered_image(
            &DynamicImage::ImageRgba8(restored),
            source.width(),
            source.height(),
            &KeySource::KeyFile(key.to_key_file()),
            SyncRegistration::identity(1.0),
        );
        assert_eq!(report.expect("known inverse extract").text, "aabb-known-inverse");
    }

    #[test]
    fn center_zoom_photo_known_inverse_extracts() {
        let mut last_error = None;
        for _ in 0..3 {
            let temp = tempfile::TempDir::new().unwrap();
            let input = temp.path().join("in.png");
            let watermarked = temp.path().join("wm.png");
            RgbaImage::from_fn(1920, 1080, |x, y| {
                let xf = x as f32 / 1920.0;
                let yf = y as f32 / 1080.0;
                Rgba([
                    (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                    (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                    (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                    255,
                ])
            })
            .save(&input)
            .unwrap();
            let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
            embed_image_file_with_auto_strength(
                &input,
                &watermarked,
                &EmbedOptions {
                    text: "版权测试 / Copyright test".into(),
                    key: key.clone(),
                    strength: 8.0,
                    media_kind: "image".into(),
                    temporal: None,
                },
            )
            .unwrap();
            let source = image::open(&watermarked).unwrap().to_rgba8();
            let zoom = 1.5_f32;
            let src_cx = (source.width() as f32 - 1.0) * 0.5;
            let src_cy = (source.height() as f32 - 1.0) * 0.5;
            let attacked = RgbaImage::from_fn(source.width(), source.height(), |x, y| {
                let source_x = (x as f32 - src_cx) / zoom + src_cx;
                let source_y = (y as f32 - src_cy) / zoom + src_cy;
                bilinear_like(&source, source_x, source_y).unwrap_or(Rgba([255, 255, 255, 255]))
            });
            let undo = 1.0 / zoom;
            let mut attempted = Vec::new();
            let mut credential_error = None;
            let mut progress = 0_usize;
            match try_geometry_candidates(
                &attacked,
                attacked.width(),
                attacked.height(),
                &KeySource::KeyFile(key.to_key_file()),
                &center_zoom_geometry_candidates(&[undo]),
                &mut attempted,
                &mut credential_error,
                "center-zoom",
                &mut None,
                &mut progress,
            ) {
                Ok(Some(report)) => {
                    assert_eq!(report.text, "版权测试 / Copyright test");
                    return;
                }
                other => last_error = Some(format!("{other:?} credential={credential_error:?}")),
            }
        }
        panic!("known 1.5× photo-size inverse failed after retries: {last_error:?}");
    }

    #[test]
    fn fill_crop_seven_degree_known_inverse_extracts() {
        let mut last_error = None;
        for _ in 0..3 {
            let temp = tempfile::TempDir::new().unwrap();
            let input = temp.path().join("in.png");
            let watermarked = temp.path().join("wm.png");
            RgbaImage::from_fn(640, 480, |x, y| {
                let xf = x as f32 / 640.0;
                let yf = y as f32 / 480.0;
                Rgba([
                    (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                    (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                    (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                    255,
                ])
            })
            .save(&input)
            .unwrap();
            let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
            embed_image_file_with_auto_strength(
                &input,
                &watermarked,
                &EmbedOptions {
                    text: "fill-crop-known-inverse".into(),
                    key: key.clone(),
                    strength: 8.0,
                    media_kind: "image".into(),
                    temporal: None,
                },
            )
            .unwrap();
            let source = image::open(&watermarked).unwrap().to_rgba8();
            let extra = 1.03_f32;
            let degrees = 7.0_f32;
            let width = source.width() as f32;
            let height = source.height() as f32;
            let radians = degrees.abs().to_radians();
            let sine = radians.sin();
            let cosine = radians.cos();
            let growth = (cosine + height / width * sine)
                .max(cosine + width / height * sine)
                .max(1.0)
                * extra;
            let src_cx = (source.width() as f32 - 1.0) * 0.5;
            let src_cy = (source.height() as f32 - 1.0) * 0.5;
            let rot = degrees.to_radians();
            let (cos, sin) = (rot.cos(), rot.sin());
            let attacked = RgbaImage::from_fn(source.width(), source.height(), |x, y| {
                let dx = (x as f32 - src_cx) / growth;
                let dy = (y as f32 - src_cy) / growth;
                bilinear_like(
                    &source,
                    cos * dx + sin * dy + src_cx,
                    -sin * dx + cos * dy + src_cy,
                )
                .unwrap_or(Rgba([255, 255, 255, 255]))
            });
            let forced = fill_crop_cover_undo_candidates(&attacked);
            let mut attempted = Vec::new();
            let mut credential_error = None;
            let mut progress = 0_usize;
            match try_geometry_candidates(
                &attacked,
                attacked.width(),
                attacked.height(),
                &KeySource::KeyFile(key.to_key_file()),
                &forced,
                &mut attempted,
                &mut credential_error,
                "rotation-fill-forced",
                &mut None,
                &mut progress,
            ) {
                Ok(Some(report)) => {
                    assert_eq!(report.text, "fill-crop-known-inverse");
                    return;
                }
                Ok(None) => last_error = Some(format!("none credential={credential_error:?}")),
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        panic!("forced fill-crop -7°+1.03 extract failed after retries: {last_error:?}");
    }

    fn center_crop_canvas(source: &RgbaImage, keep: f32) -> RgbaImage {
        let crop_w = ((source.width() as f32) * keep).round().clamp(1.0, source.width() as f32) as u32;
        let crop_h = ((source.height() as f32) * keep).round().clamp(1.0, source.height() as f32) as u32;
        let x = (source.width() - crop_w) / 2;
        let y = (source.height() - crop_h) / 2;
        image::imageops::crop_imm(source, x, y, crop_w, crop_h).to_image()
    }

    #[test]
    fn center_crop_identity_extracts_without_geometry() {
        let mut last_error = None;
        for _ in 0..8 {
            let temp = tempfile::TempDir::new().unwrap();
            let input = temp.path().join("in.png");
            let watermarked = temp.path().join("wm.png");
            RgbaImage::from_fn(640, 480, |x, y| {
                let xf = x as f32 / 640.0;
                let yf = y as f32 / 480.0;
                Rgba([
                    (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                    (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                    (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                    255,
                ])
            })
            .save(&input)
            .unwrap();
            let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
            embed_image_file_with_auto_strength(
                &input,
                &watermarked,
                &EmbedOptions {
                    text: "版权测试 / Copyright test".into(),
                    key: key.clone(),
                    strength: 8.0,
                    media_kind: "image".into(),
                    temporal: None,
                },
            )
            .unwrap();
            let source = image::open(&watermarked).unwrap().to_rgba8();
            let cropped = center_crop_canvas(&source, 0.77);
            let attacked = temp.path().join("crop.png");
            cropped.save(&attacked).unwrap();
            let reloaded = image::open(&attacked).unwrap().to_rgba8();
            let width = reloaded.width();
            let height = reloaded.height();
            match extract_registered_image(
                &DynamicImage::ImageRgba8(reloaded),
                width,
                height,
                &KeySource::KeyFile(key.to_key_file()),
                SyncRegistration::identity(1.0),
            ) {
                Ok(report) => {
                    assert_eq!(report.text, "版权测试 / Copyright test");
                    return;
                }
                Err(error) => last_error = Some(error),
            }
        }
        panic!("identity center-crop extract failed after retries: {last_error:?}");
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
        let growth = (cosine + height / width * sine)
            .max(cosine + width / height * sine)
            .max(1.0);
        let out_w = (width * growth).round().clamp(1.0, u32::MAX as f32) as u32;
        let out_h = (height * growth).round().clamp(1.0, u32::MAX as f32) as u32;
        let src_cx = (source.width() as f32 - 1.0) * 0.5;
        let src_cy = (source.height() as f32 - 1.0) * 0.5;
        let out_cx = (out_w as f32 - 1.0) * 0.5;
        let out_cy = (out_h as f32 - 1.0) * 0.5;
        let rot = degrees.to_radians();
        let (cos, sin) = (rot.cos(), rot.sin());
        RgbaImage::from_fn(out_w, out_h, |x, y| {
            let dx = (x as f32 - out_cx) / content_zoom;
            let dy = (y as f32 - out_cy) / content_zoom;
            bilinear_like(
                source,
                cos * dx + sin * dy + src_cx,
                -sin * dx + cos * dy + src_cy,
            )
            .unwrap_or(Rgba([255, 255, 255, 255]))
        })
    }

    #[test]
    fn trimmed_white_pad_known_inverse_extracts() {
        let temp = tempfile::TempDir::new().unwrap();
        let input = temp.path().join("in.png");
        let watermarked = temp.path().join("wm.png");
        RgbaImage::from_fn(640, 480, |x, y| {
            let xf = x as f32 / 640.0;
            let yf = y as f32 / 480.0;
            Rgba([
                (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                255,
            ])
        })
        .save(&input)
        .unwrap();
        let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
        embed_image_file_with_auto_strength(
            &input,
            &watermarked,
            &EmbedOptions {
                text: "trimmed-pad-inverse".into(),
                key: key.clone(),
                strength: 8.0,
                media_kind: "image".into(),
                temporal: None,
            },
        )
        .unwrap();
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let rotated = aspect_preserving_rotated_canvas(&source, 7.0, 1.03);
        let trimmed = sync::trim_uniform_border(&rotated).expect("white pad should be trimmable");
        let layouts = [GeometryLayout::Standard];
        for layout in layouts {
            for angle in [-7.0_f32] {
                for scale in [0.97_f32] {
                    let forced = [GeometryCandidate {
                        rotation_degrees: angle,
                        scale,
                        layout,
                        score: 1.0,
                    }];
                    let mut attempted = Vec::new();
                    let mut credential_error = None;
                    let mut progress = 0_usize;
                    if let Ok(Some(report)) = try_geometry_candidates(
                        &trimmed,
                        trimmed.width(),
                        trimmed.height(),
                        &KeySource::KeyFile(key.to_key_file()),
                        &forced,
                        &mut attempted,
                        &mut credential_error,
                        "rotation-fill-forced",
                        &mut None,
                        &mut progress,
                    ) {
                        assert_eq!(report.text, "trimmed-pad-inverse");
                        eprintln!(
                            "trimmed inverse ok layout={layout:?} angle={angle} scale={scale} size={}x{}",
                            trimmed.width(),
                            trimmed.height()
                        );
                        return;
                    }
                }
            }
        }
        panic!(
            "no trimmed inverse; size={}x{} padded={}",
            trimmed.width(),
            trimmed.height(),
            sync::has_uniform_rotation_padding(&trimmed)
        );
    }

    #[test]
    fn aabb_bilinear_forced_negative_two_extracts() {
        let temp = tempfile::TempDir::new().unwrap();
        let input = temp.path().join("in.png");
        let watermarked = temp.path().join("wm.png");
        RgbaImage::from_fn(960, 1280, |x, y| {
            let xf = x as f32 / 960.0;
            let yf = y as f32 / 1280.0;
            Rgba([
                (118.0 + 48.0 * (xf * 6.2832).sin() + 18.0 * yf) as u8,
                (126.0 + 42.0 * (yf * 6.2832).cos() + 16.0 * xf) as u8,
                (132.0 + 36.0 * ((xf + yf) * 4.7124).sin()) as u8,
                255,
            ])
        })
        .save(&input)
        .unwrap();
        let key = crate::keyfile::WatermarkKey::random(crate::keyfile::KeyMode::Independent);
        embed_image_file_with_auto_strength(
            &input,
            &watermarked,
            &EmbedOptions {
                text: "aabb-forced".into(),
                key: key.clone(),
                strength: 8.0,
                media_kind: "image".into(),
                temporal: None,
            },
        )
        .unwrap();
        let source = image::open(&watermarked).unwrap().to_rgba8();
        let degrees = 2.0_f32;
        let radians = degrees.abs().to_radians();
        let out_w = (source.width() as f32 * radians.cos().abs()
            + source.height() as f32 * radians.sin().abs())
        .round() as u32;
        let out_h = (source.width() as f32 * radians.sin().abs()
            + source.height() as f32 * radians.cos().abs())
        .round() as u32;
        let src_cx = (source.width() as f32 - 1.0) * 0.5;
        let src_cy = (source.height() as f32 - 1.0) * 0.5;
        let out_cx = (out_w as f32 - 1.0) * 0.5;
        let out_cy = (out_h as f32 - 1.0) * 0.5;
        let rot = degrees.to_radians();
        let (cos, sin) = (rot.cos(), rot.sin());
        let attacked = RgbaImage::from_fn(out_w, out_h, |x, y| {
            let dx = x as f32 - out_cx;
            let dy = y as f32 - out_cy;
            bilinear_like(&source, cos * dx + sin * dy + src_cx, -sin * dx + cos * dy + src_cy)
                .unwrap_or(Rgba([255, 255, 255, 255]))
        });
        assert!(
            sync::has_uniform_rotation_padding(&attacked),
            "AABB 2° should look padded"
        );
        let mut progress = 0_usize;
        let mut attempted = Vec::new();
        let mut credential_error = None;
        let mut on_progress = None;
        let forced = [GeometryCandidate {
            rotation_degrees: -2.0,
            scale: 1.0,
            layout: GeometryLayout::AabbPadded,
            score: 1.0,
        }];
        let report = try_geometry_candidates(
            &attacked,
            attacked.width(),
            attacked.height(),
            &KeySource::KeyFile(key.to_key_file()),
            &forced,
            &mut attempted,
            &mut credential_error,
            "rotation-range-narrow",
            &mut on_progress,
            &mut progress,
        )
        .unwrap();
        assert!(
            report.is_some(),
            "forced -2° AABB should extract, credential={credential_error:?}"
        );

        let attacked_path = temp.path().join("attacked.png");
        attacked.save(&attacked_path).unwrap();
        let reloaded = image::open(&attacked_path).unwrap().to_rgba8();
        assert_eq!(
            (attacked.width(), attacked.height()),
            (reloaded.width(), reloaded.height())
        );
        let mismatch = attacked
            .pixels()
            .zip(reloaded.pixels())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(mismatch, 0, "PNG roundtrip changed {mismatch} pixels");
        assert_eq!(
            sync::has_uniform_rotation_padding(&attacked),
            sync::has_uniform_rotation_padding(&reloaded),
            "padding detection changed after PNG roundtrip"
        );
        let mut reload_attempted = Vec::new();
        let mut reload_credential = None;
        let mut reload_progress = 0_usize;
        let reload_forced = try_geometry_candidates(
            &reloaded,
            reloaded.width(),
            reloaded.height(),
            &KeySource::KeyFile(key.to_key_file()),
            &forced,
            &mut reload_attempted,
            &mut reload_credential,
            "rotation-range-narrow",
            &mut None,
            &mut reload_progress,
        )
        .unwrap();
        assert!(
            reload_forced.is_some(),
            "forced -2° AABB should extract after PNG reload, credential={reload_credential:?}"
        );
    }

    fn bilinear_like(image: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
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

    #[test]
    fn rotation_refine_ladder_reaches_137_36_and_17_30() {
        let bins = rotation_bin_centers();
        assert!(bins.iter().any(|angle| (*angle - 140.0).abs() < 0.01));
        let one_degree = expanded_range_angles(140.0, 15.0, 1.0);
        assert!(one_degree.iter().any(|angle| (*angle - 137.0).abs() < 0.01));
        let tenths = expanded_range_angles(137.4, 1.5, 0.1);
        assert!(tenths.iter().any(|angle| (*angle - 137.4).abs() < 0.01));
        let hundredths = expanded_range_angles(137.36, 0.15, 0.01);
        assert!(
            hundredths
                .iter()
                .any(|angle| (*angle - 137.36).abs() <= 0.02),
            "0.01° ladder missed 137.36 in {hundredths:?}"
        );
        let around_17 = expanded_range_angles(17.30, 0.15, 0.01);
        assert!(around_17.iter().any(|angle| (*angle - 17.30).abs() <= 0.02));
    }

    #[test]
    fn separated_peaks_keep_distinct_layouts_at_the_same_angle() {
        let scored = [
            GeometryCandidate {
                rotation_degrees: 20.0,
                scale: 1.0,
                layout: GeometryLayout::AabbPadded,
                score: 0.05,
            },
            GeometryCandidate {
                rotation_degrees: 20.0,
                scale: 0.91,
                layout: GeometryLayout::FixedCanvasFit,
                score: 0.049,
            },
            GeometryCandidate {
                rotation_degrees: 40.0,
                scale: 1.0,
                layout: GeometryLayout::AabbPadded,
                score: 0.03,
            },
        ];
        let selected = select_separated_peaks(&scored, 3, 20.0);
        assert!(selected.iter().any(|candidate| {
            candidate.layout == GeometryLayout::AabbPadded
                && (candidate.rotation_degrees - 20.0).abs() < 0.01
        }));
        assert!(selected.iter().any(|candidate| {
            candidate.layout == GeometryLayout::FixedCanvasFit
        }));
    }

    #[test]
    fn no_pad_hot_path_coverage_is_not_the_forced_five_to_eight_list() {
        let bins = rotation_bin_centers();
        assert!(bins.iter().any(|angle| angle.abs() > 8.0));
        assert!(bins.iter().any(|angle| (*angle - 140.0).abs() < 0.01));
        assert_ne!(
            no_pad_fill_crop_forced_candidates(&RgbaImage::new(640, 480))
                .iter()
                .map(|candidate| candidate.rotation_degrees)
                .collect::<Vec<_>>(),
            bins
        );
    }

    #[test]
    fn center_window_origin_is_tried_before_top_left() {
        let original_width = 640_u32;
        let original_height = 480_u32;
        let current_width = ((original_width as f32) * 0.77).round() as u32;
        let current_height = ((original_height as f32) * 0.77).round() as u32;
        let origins = center_window_origin_candidates(
            current_width,
            current_height,
            original_width,
            original_height,
        );
        assert!(!origins.is_empty());
        let start_x = (original_width - current_width) / 2;
        let start_y = (original_height - current_height) / 2;
        let expected_x = ((start_x + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
        let expected_y = ((start_y + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
        assert!(origins.iter().any(|origin| {
            origin.block_x == expected_x && origin.block_y == expected_y
        }));
        assert!(origins[0].block_x != 0 || origins[0].block_y != 0);
    }

    #[test]
    fn input_pixel_limit_rejects_over_64_megapixels() {
        assert!(!exceeds_max_input_pixels(8000, 8000));
        assert!(exceeds_max_input_pixels(8001, 8001));
        assert!(exceeds_max_input_pixels(u32::MAX, 2));
    }

    #[test]
    fn enlarged_images_are_prioritized_before_orientation_fallbacks() {
        let scales = priority_scale_parameters();
        assert_eq!(&scales[..2], &[0.5, 0.25]);
        assert_eq!(&scales[2..7], &[2.0, 1.5, 1.25, 3.0, 4.0]);
        assert_eq!(orientation_angle_parameters(), vec![90.0, -90.0, 180.0]);
    }

    #[test]
    fn coarse_scale_grid_uses_four_percent_steps() {
        let scales = coarse_scale_parameters();
        let mut expected = 0.25_f32;
        while expected <= 0.30 {
            assert!(
                scales.iter().any(|scale| (*scale - expected).abs() < 0.0005),
                "missing coarse scale {expected}"
            );
            expected *= 1.04;
        }
        assert!(
            !scales.windows(2).any(|pair| (pair[1] / pair[0] - 1.1).abs() < 0.001
                && pair[0] < 0.4),
            "coarse grid should no longer use 10% steps in the downscale range"
        );
    }

    #[test]
    fn padding_hint_layouts_include_expanded_canvas_zoom() {
        let image = RgbaImage::from_pixel(160, 120, Rgba([80, 90, 100, 255]));
        let parameters = padding_hint_layout_parameters(&image, vec![7.0]);
        assert!(parameters.iter().any(|(angle, scale, layout)| {
            *layout == GeometryLayout::ExpandedCanvas
                && (*angle - 7.0).abs() < 0.001
                && (*scale - 1.0).abs() > 0.01
        }));
        assert!(parameters.iter().any(|(_, scale, layout)| {
            *layout == GeometryLayout::ExpandedCanvas && (*scale - 0.92).abs() < 0.001
        }));
        assert!(parameters.iter().any(|(_, scale, layout)| {
            *layout == GeometryLayout::ExpandedCanvas && (*scale - 1.08).abs() < 0.001
        }));
    }

    #[test]
    fn aspect_preserving_rotation_predicts_fill_partial_and_fit_scales() {
        let growth = rotation_canvas_growth(1600, 1200, 2.5);
        let scales = fixed_canvas_correction_scales(1600, 1200, 2.5);
        assert!(growth > 1.0 && growth < 1.1);
        assert!(scales.iter().any(|scale| *scale < 1.0));
        assert!(scales.iter().any(|scale| *scale > 1.0));
        assert_eq!(rotation_canvas_growth(1600, 1200, 0.0), 1.0);
    }

    #[test]
    fn robust_header_repairs_two_low_confidence_compression_errors() {
        let header = RobustHeader {
            salt: [11; 16],
            body_len: 96,
            original_width: 1024,
            original_height: 768,
            strength_x10: 100,
            flags: 0,
            qim_profile: dwt_qim::QIM_PROFILE_PYRAMID,
        };
        let mut bits = bytes_to_bits(&header.to_bytes());
        bits[18] = !bits[18];
        bits[237] = !bits[237];
        let mut reliabilities = vec![0.9_f32; bits.len()];
        reliabilities[18] = 0.05;
        reliabilities[237] = 0.08;
        let repaired = decode_or_repair_robust_header(&bits, &reliabilities).unwrap();
        assert_eq!(repaired.salt, header.salt);
        assert_eq!(repaired.body_len, header.body_len);
    }

    #[test]
    fn body_erasures_are_limited_per_codeword() {
        let reliabilities = vec![0.0_f32; BCH_CODEWORD_BITS * 2];
        let observed = vec![false; reliabilities.len()];
        let erasures = build_body_erasures(&reliabilities, &observed, 6);
        for codeword in erasures.chunks(BCH_CODEWORD_BITS) {
            assert_eq!(codeword.iter().filter(|erased| **erased).count(), 6);
        }
    }

    #[test]
    fn robust_header_layout_matches_declared_size() {
        assert_eq!(RobustHeader {
            salt: [0; 16],
            body_len: 32,
            original_width: 640,
            original_height: 480,
            strength_x10: 120,
            flags: 0,
            qim_profile: dwt_qim::QIM_PROFILE_PYRAMID,
        }.to_bytes().len(), crate::payload::ROBUST_HEADER_LEN);
    }

    #[test]
    fn pixel_aligned_scale_restores_awkward_original_width() {
        let scales = nearby_pixel_aligned_scales(421, 316, 1.518_702_9);
        assert!(
            scales
                .iter()
                .any(|scale| (421.0 * scale).round() as u32 == 640),
            "snapped scales={scales:?}"
        );
        let period = period_aligned_scale_parameters(421, 316, [1.5_f32]);
        assert!(
            period
                .iter()
                .any(|scale| (421.0 * scale).round() as u32 == 640),
            "1.5× period-aligned scales should include 640/421, got {period:?}"
        );
        let coarse = period_aligned_scale_parameters(421, 316, coarse_scale_parameters());
        assert!(
            coarse
                .iter()
                .any(|scale| (421.0 * scale).round() as u32 == 640),
            "coarse aligned scales should include 640/421"
        );
    }

    #[test]
    fn no_pad_fill_crop_search_includes_seven_degree_cover_undo() {
        let angles = no_pad_fill_crop_angle_parameters();
        assert!(angles.iter().any(|angle| (*angle - 7.0).abs() < 0.01));
        assert!(angles.iter().any(|angle| (*angle + 7.0).abs() < 0.01));
        let scales = fixed_canvas_correction_scales(640, 480, 7.0);
        let growth = rotation_canvas_growth(640, 480, 7.0);
        let undo = 1.0 / (growth * 1.03);
        assert!(
            scales.iter().any(|scale| (*scale - undo).abs() < 0.01),
            "expected undo {undo} in {scales:?}"
        );
        assert_eq!(
            no_pad_fill_crop_seven_degree_candidates(&RgbaImage::new(640, 480)).len(),
            6
        );
    }

    #[test]
    fn center_zoom_undo_grid_covers_editor_factors() {
        let grid = center_zoom_undo_grid();
        assert!(grid.first().is_some_and(|scale| (*scale - 0.50).abs() < 0.001));
        assert!(grid.last().is_some_and(|scale| (*scale - 0.98).abs() < 0.001));
        for factor in [1.10_f32, 1.25, 1.50, 2.00] {
            let undo = 1.0 / factor;
            assert!(
                grid.iter().any(|scale| (*scale - undo).abs() < 0.02),
                "grid missing undo {undo} for zoom {factor}"
            );
        }
        let blank = RgbaImage::from_pixel(64, 64, Rgba([128, 128, 128, 255]));
        let seeds = period_aligned_center_zoom_undos(&blank);
        assert!(seeds.iter().any(|scale| (*scale - 1.0 / 1.1).abs() < 0.01));
        assert!(seeds.iter().any(|scale| (*scale - 1.0 / 1.5).abs() < 0.01));
        assert!(seeds.iter().any(|scale| (*scale - 0.5).abs() < 0.01));
    }

    #[test]
    fn fill_crop_cover_undo_includes_one_and_two_degrees() {
        let forced = fill_crop_cover_undo_candidates(&RgbaImage::new(640, 480));
        for angle in [-7.0_f32, 7.0, -2.3, 2.3, -2.0, 2.0, -1.0, 1.0] {
            assert!(
                forced
                    .iter()
                    .any(|candidate| (candidate.rotation_degrees - angle).abs() < 0.01
                        && candidate.layout == GeometryLayout::FixedCanvasFit),
                "forced fill-crop missing {angle}°"
            );
        }
    }

    #[test]
    fn rotation_bin_parameters_always_try_fill_crop_and_aabb() {
        let source = RgbaImage::new(640, 480);
        for has_padding in [true, false] {
            let parameters = rotation_bin_parameters(&source, has_padding);
            assert!(parameters.iter().any(|(_, _, layout)| {
                *layout == GeometryLayout::FixedCanvasFit
            }));
            assert!(parameters.iter().any(|(_, _, layout)| {
                *layout == GeometryLayout::AabbPadded
            }));
            assert!(parameters.iter().any(|(_, _, layout)| {
                *layout == GeometryLayout::AabbHeightKept
            }));
            assert!(parameters.iter().any(|(_, _, layout)| {
                *layout == GeometryLayout::AabbWidthKept
            }));
            assert_eq!(
                has_padding,
                parameters
                    .iter()
                    .any(|(_, _, layout)| *layout == GeometryLayout::ExpandedCanvas)
            );
        }
    }
}
