use image::{imageops, DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

const MAX_MATERIALIZED_PIXELS: u64 = 96_000_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRegistration {
    pub rotation_degrees: f32,
    pub scale: f32,
    #[serde(default)]
    pub translation_x: i32,
    #[serde(default)]
    pub translation_y: i32,
    pub score: f32,
}

impl SyncRegistration {
    pub fn identity(score: f32) -> Self {
        Self {
            rotation_degrees: 0.0,
            scale: 1.0,
            translation_x: 0,
            translation_y: 0,
            score,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.rotation_degrees.abs() < 0.001
            && (self.scale - 1.0).abs() < 0.001
            && self.translation_x == 0
            && self.translation_y == 0
    }
}

#[derive(Debug, Clone)]
pub struct RegistrationCandidate {
    pub image: DynamicImage,
    pub registration: SyncRegistration,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AffineTransform {
    output_width: u32,
    output_height: u32,
    source_center_x: f32,
    source_center_y: f32,
    output_center_x: f32,
    output_center_y: f32,
    cosine: f32,
    sine: f32,
    inverse_scale: f32,
    nearest: bool,
}

impl AffineTransform {
    pub(crate) fn new(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        scale: f32,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !scale.is_finite()
            || !(0.25..=4.0).contains(&scale)
        {
            return None;
        }
        let radians = rotation_degrees.to_radians();
        let quarter_turn = (rotation_degrees.abs() - 90.0).abs() < 0.001;
        let (base_width, base_height) = if quarter_turn {
            // 精确四分之一圈校正必须交换输出宽高，否则横图旋转成竖图后仍被塞回原宽高画布，会在完整解码前永久裁掉一部分水印网格。
            // Exact quarter-turn correction must swap the output dimensions; keeping the original canvas shape would permanently crop part of the watermark grid before full decoding.
            (source_height, source_width)
        } else {
            (source_width, source_height)
        };
        let output_width = ((base_width as f64 * scale as f64).round() as u64)
            .clamp(1, u32::MAX as u64) as u32;
        let output_height = ((base_height as f64 * scale as f64).round() as u64)
            .clamp(1, u32::MAX as u64) as u32;
        Some(Self::from_parts(
            source_width,
            source_height,
            output_width,
            output_height,
            radians,
            scale,
            false,
        ))
    }

    pub(crate) fn new_expanded_canvas_correction(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
            || rotation_degrees.abs() < 0.001
            || rotation_degrees.abs() >= 44.0
        {
            return None;
        }
        let canvas_growth = aspect_canvas_growth(source_width, source_height, rotation_degrees)?;
        let output_width = ((source_width as f64 / canvas_growth as f64).round() as u64)
            .clamp(1, u32::MAX as u64) as u32;
        let output_height = ((source_height as f64 / canvas_growth as f64).round() as u64)
            .clamp(1, u32::MAX as u64) as u32;
        // 等比例扩大的旋转画布只改变了画布尺寸，内容本身仍以原像素尺度旋转；因此输出尺寸与采样尺度必须分离，否则会把内容再次缩放并破坏八像素水印网格。
        // An aspect-expanded rotation canvas changes only the canvas dimensions while the content remains rotated at its original pixel scale; output sizing and sampling scale must therefore be separated or the content would be resized again and the eight-pixel watermark grid would be destroyed.
        Some(Self::from_parts(
            source_width,
            source_height,
            output_width,
            output_height,
            rotation_degrees.to_radians(),
            content_scale,
            false,
        ))
    }

    pub(crate) fn new_fixed_canvas_fit_correction(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
            || rotation_degrees.abs() < 0.001
            || rotation_degrees.abs() > 180.0
        {
            return None;
        }
        // 无白边铺满不走 AABB 行列式，47° 或 137.36° 仍是合法的同画布旋转；旧的 44° 闸会把这些角直接丢掉。
        // No-pad fill-crop does not invert an AABB, so 47° or 137.36° remain legal same-canvas rotations; the old 44° gate dropped those angles outright.
        // 固定宽高画布的内容缩放与输出尺寸必须分开建模，是因为编辑器可能在“适应画布”之外再附加少量缩放；把两者混成一个普通 resize 会丢失正确块周期。
        // Content scaling and output dimensions are modeled separately for fixed canvases because editors may add a small zoom beyond fit-to-canvas; collapsing both into an ordinary resize loses the correct block period.
        // 铺满裁切已经做过一次双线性插值；逆变换再用双线性会把 QIM 决策边界抹掉，因此这里按最近邻采样，只恢复块周期。
        // Fill-crop already applied one bilinear interpolation; a second bilinear inverse wipes QIM decision boundaries, so nearest-neighbor sampling is used here to restore only the block period.
        Some(Self::from_parts(
            source_width,
            source_height,
            source_width,
            source_height,
            rotation_degrees.to_radians(),
            content_scale,
            true,
        ))
    }

    pub(crate) fn new_center_zoom_correction(
        source_width: u32,
        source_height: u32,
        content_scale: f32,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
        {
            return None;
        }
        // 中心放大再裁回原画布是 0° 的固定画布缩放；沿用铺满裁切的最近邻，避免二次双线性抹掉 QIM。
        // Center-zoom then crop-back is a 0° fixed-canvas scale; nearest-neighbor matches fill-crop so a second bilinear pass does not wipe QIM.
        Some(Self::from_parts(
            source_width,
            source_height,
            source_width,
            source_height,
            0.0,
            content_scale,
            true,
        ))
    }

    pub(crate) fn new_aabb_padded_correction(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
    ) -> Option<Self> {
        Self::new_aabb_padded_correction_with_header(
            source_width,
            source_height,
            rotation_degrees,
            content_scale,
            None,
        )
    }

    pub(crate) fn new_aabb_padded_correction_with_header(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
        original_size: Option<(u32, u32)>,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
            || rotation_degrees.abs() < 0.001
            || rotation_degrees.abs() > 180.0
        {
            return None;
        }
        let radians = rotation_degrees.to_radians();
        let sine = radians.sin().abs();
        let cosine = radians.cos().abs();
        let determinant = cosine * cosine - sine * sine;
        let (output_width, output_height) = if determinant.abs() < 0.08 {
            // 45° 族上 AABB 反解奇异，不能从画布宽高唯一还原原图；有公开头尺寸才用头，否则让邻域角接手。
            // AABB inversion is singular near the 45° family and cannot uniquely recover the original size; use the public header when present, otherwise skip this angle for a neighbor.
            let (orig_w, orig_h) = original_size?;
            (
                snap_to_header_period(orig_w as f32 * content_scale),
                snap_to_header_period(orig_h as f32 * content_scale),
            )
        } else {
            let source_width_f = source_width as f32;
            let source_height_f = source_height as f32;
            let recovered_width =
                (cosine * source_width_f - sine * source_height_f) / determinant;
            let recovered_height =
                (cosine * source_height_f - sine * source_width_f) / determinant;
            if !recovered_width.is_finite()
                || !recovered_height.is_finite()
                || recovered_width < 1.0
                || recovered_height < 1.0
            {
                return None;
            }
            (
                snap_to_header_period(recovered_width * content_scale),
                snap_to_header_period(recovered_height * content_scale),
            )
        };
        // 真 AABB 的宽高增长不同；再用各向同性 max-growth 反解会把 3637 收成约 3568，八像素周期直接错开。
        // A true AABB grows each axis differently; an isotropic max-growth inverse collapses 3637 to ~3568 and misses the eight-pixel period.
        Some(Self::from_parts(
            source_width,
            source_height,
            output_width,
            output_height,
            radians,
            content_scale,
            true,
        ))
    }

    pub(crate) fn new_aabb_axis_cropped_correction(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
        keep_height: bool,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
            || rotation_degrees.abs() < 0.001
            || rotation_degrees.abs() > 180.0
        {
            return None;
        }
        let radians = rotation_degrees.to_radians();
        let sine = radians.sin().abs();
        let cosine = radians.cos().abs();
        // 一轴被裁回原图尺寸后，AABB 行列式会把那一轴解短；这里改为假定未裁的边仍是包围盒，反解另一边。
        // After one axis is cropped back to the original size, AABB inversion shortens that axis; assume the uncropped side is still the bounding box and solve the other.
        if cosine < 0.08 {
            return None;
        }
        let (output_width, output_height) = if keep_height {
            let recovered_width = (source_width as f32 - source_height as f32 * sine) / cosine;
            if !recovered_width.is_finite()
                || recovered_width < 1.0
                || recovered_width >= source_width as f32 + 1.0
            {
                return None;
            }
            (
                snap_to_nearest_block_period(recovered_width),
                source_height,
            )
        } else {
            let recovered_height = (source_height as f32 - source_width as f32 * sine) / cosine;
            if !recovered_height.is_finite()
                || recovered_height < 1.0
                || recovered_height >= source_height as f32 + 1.0
            {
                return None;
            }
            (
                source_width,
                snap_to_nearest_block_period(recovered_height),
            )
        };
        Some(Self::from_parts(
            source_width,
            source_height,
            output_width,
            output_height,
            radians,
            content_scale,
            false,
        ))
    }

    pub(crate) fn new_tight_bounds_correction(
        source_width: u32,
        source_height: u32,
        rotation_degrees: f32,
        content_scale: f32,
    ) -> Option<Self> {
        if source_width == 0
            || source_height == 0
            || !rotation_degrees.is_finite()
            || !content_scale.is_finite()
            || !(0.25..=4.0).contains(&content_scale)
            || rotation_degrees.abs() < 0.001
            || rotation_degrees.abs() >= 44.0
        {
            return None;
        }
        let radians = rotation_degrees.to_radians();
        let sine = radians.sin().abs();
        let cosine = radians.cos().abs();
        let determinant = cosine * cosine - sine * sine;
        if determinant.abs() < 0.05 {
            return None;
        }
        let source_width_f = source_width as f32;
        let source_height_f = source_height as f32;
        let recovered_width = (cosine * source_width_f - sine * source_height_f) / determinant;
        let recovered_height = (cosine * source_height_f - sine * source_width_f) / determinant;
        if !recovered_width.is_finite()
            || !recovered_height.is_finite()
            || recovered_width < 1.0
            || recovered_height < 1.0
        {
            return None;
        }
        let output_width = round_up_near_block_boundary(recovered_width * content_scale);
        let output_height = round_up_near_block_boundary(recovered_height * content_scale);
        // 紧贴旋转内容裁掉白边后，反解出的宽高仍包含编辑器附加的内容缩放；将校正比例同时应用到输出尺寸，才能让采样坐标和八像素水印块周期一起回到原始尺度。
        // After white borders are tightly cropped, the dimensions solved from the rotated bounds still include the editor's content zoom; applying the correction scale to the output size keeps both sampling coordinates and the eight-pixel watermark period at the original scale.
        Some(Self::from_parts(
            source_width,
            source_height,
            output_width,
            output_height,
            radians,
            content_scale,
            false,
        ))
    }

    fn from_parts(
        source_width: u32,
        source_height: u32,
        output_width: u32,
        output_height: u32,
        radians: f32,
        content_scale: f32,
        nearest: bool,
    ) -> Self {
        Self {
            output_width,
            output_height,
            source_center_x: (source_width as f32 - 1.0) * 0.5,
            source_center_y: (source_height as f32 - 1.0) * 0.5,
            output_center_x: (output_width as f32 - 1.0) * 0.5,
            output_center_y: (output_height as f32 - 1.0) * 0.5,
            cosine: radians.cos(),
            sine: radians.sin(),
            inverse_scale: 1.0 / content_scale,
            nearest,
        }
    }

    pub(crate) fn output_width(&self) -> u32 {
        self.output_width
    }

    pub(crate) fn output_height(&self) -> u32 {
        self.output_height
    }

    pub(crate) fn can_materialize(&self) -> bool {
        self.output_width as u64 * self.output_height as u64 <= MAX_MATERIALIZED_PIXELS
    }

    pub(crate) fn sample_rgba(&self, image: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
        let dx = (x - self.output_center_x) * self.inverse_scale;
        let dy = (y - self.output_center_y) * self.inverse_scale;
        let source_x = self.cosine * dx + self.sine * dy + self.source_center_x;
        let source_y = -self.sine * dx + self.cosine * dy + self.source_center_y;
        if self.nearest {
            nearest_sample(image, source_x, source_y)
        } else {
            bilinear_sample(image, source_x, source_y)
        }
    }

    pub(crate) fn sample_luma(&self, image: &RgbaImage, x: f32, y: f32) -> Option<f32> {
        self.sample_rgba(image, x, y).map(|pixel| {
            0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32
        })
    }
}

fn snap_to_nearest_block_period(value: f32) -> u32 {
    if !value.is_finite() || value < 1.0 {
        return 1;
    }
    let rounded = value.round().clamp(1.0, u32::MAX as f32) as u32;
    let down = (rounded / 8) * 8;
    let up = down.saturating_add(8);
    if down == 0 {
        return up.max(1);
    }
    if (value - down as f32).abs() <= (up as f32 - value).abs() {
        down
    } else {
        up.max(1)
    }
}

fn snap_to_header_period(value: f32) -> u32 {
    if !value.is_finite() || value < 1.0 {
        return 1;
    }
    let rounded = value.round().clamp(1.0, u32::MAX as f32) as i32;
    let mut best = rounded.max(1) as u32;
    let mut best_dist = (best as f32 - value).abs();
    for delta in -8..=8 {
        let candidate = (rounded + delta).max(1) as u32;
        let remainder = candidate % 8;
        let period_bonus = if remainder == 0 || remainder == 1 {
            0.15
        } else {
            0.0
        };
        let dist = (candidate as f32 - value).abs() - period_bonus;
        if dist < best_dist {
            best_dist = dist;
            best = candidate;
        }
    }
    best
}

fn round_up_near_block_boundary(value: f32) -> u32 {
    let rounded = (value.round() as u64).clamp(1, u32::MAX as u64) as u32;
    let remainder = rounded % 8;
    if remainder >= 6 {
        // 逆旋转和裁边常因抗锯齿阈值少一两个像素；仅在非常接近下一块边界时向上补齐，可恢复最后一列/行载波，而额外块会在头中原始尺寸校验后被忽略。
        // Inverse rotation and anti-aliased cropping often lose one or two pixels; rounding up only when very near the next block boundary restores the final carrier row or column, while any extra block is ignored after original dimensions are validated from the header.
        rounded.saturating_add(8 - remainder)
    } else {
        rounded
    }
}

fn aspect_canvas_growth(width: u32, height: u32, rotation_degrees: f32) -> Option<f32> {
    if width == 0 || height == 0 || !rotation_degrees.is_finite() {
        return None;
    }
    let width = width as f32;
    let height = height as f32;
    let radians = rotation_degrees.abs().to_radians();
    let sine = radians.sin().abs();
    let cosine = radians.cos().abs();
    let width_growth = cosine + height / width * sine;
    let height_growth = cosine + width / height * sine;
    let growth = width_growth.max(height_growth).max(1.0);
    growth.is_finite().then_some(growth)
}


pub fn trim_uniform_border(source: &RgbaImage) -> Option<RgbaImage> {
    if source.width() < 64 || source.height() < 64 {
        return None;
    }
    let max_x_trim = source.width() / 3;
    let max_y_trim = source.height() / 3;
    let mut left = 0;
    let mut right = source.width();
    let mut top = 0;
    let mut bottom = source.height();

    while left < max_x_trim && border_column(source, left, top, bottom) {
        left += 1;
    }
    while right > left + 1
        && source.width() - right < max_x_trim
        && border_column(source, right - 1, top, bottom)
    {
        right -= 1;
    }
    while top < max_y_trim && border_row(source, top, left, right) {
        top += 1;
    }
    while bottom > top + 1
        && source.height() - bottom < max_y_trim
        && border_row(source, bottom - 1, left, right)
    {
        bottom -= 1;
    }

    let removed = left + (source.width() - right) + top + (source.height() - bottom);
    if removed < 4 || right - left < 48 || bottom - top < 48 {
        return None;
    }
    // 近白阈值放到约 232、行列纯度放到约 0.96，是为了让旋转抗锯齿留下的灰白边也能被裁掉，同时仍只处理与外边界连通的色带，避免吃掉照片内部。
    // Near-white around 232 and a 0.96 row/column fill let anti-aliased gray fringes be removed after rotation, while still limiting the trim to outer-connected bands so interior photo regions are not eaten.
    Some(imageops::crop_imm(source, left, top, right - left, bottom - top).to_image())
}

fn border_column(source: &RgbaImage, x: u32, top: u32, bottom: u32) -> bool {
    let length = bottom.saturating_sub(top).max(1);
    let step = (length / 192).max(1);
    let mut samples = 0_u32;
    let mut border = 0_u32;
    let mut y = top;
    while y < bottom {
        samples += 1;
        if is_uniform_border_pixel(source.get_pixel(x, y)) {
            border += 1;
        }
        y = y.saturating_add(step);
    }
    samples > 0 && border as f32 / samples as f32 >= 0.96
}

fn border_row(source: &RgbaImage, y: u32, left: u32, right: u32) -> bool {
    let length = right.saturating_sub(left).max(1);
    let step = (length / 192).max(1);
    let mut samples = 0_u32;
    let mut border = 0_u32;
    let mut x = left;
    while x < right {
        samples += 1;
        if is_uniform_border_pixel(source.get_pixel(x, y)) {
            border += 1;
        }
        x = x.saturating_add(step);
    }
    samples > 0 && border as f32 / samples as f32 >= 0.96
}

fn is_uniform_border_pixel(pixel: &Rgba<u8>) -> bool {
    // 232 而不是 248，是为了把旋转抗锯齿形成的灰白边算进补边，否则裁切会在灰边上停下并留下错误的块相位。
    // 232 rather than 248 counts anti-aliased gray-white as padding, otherwise the trim stops on the fringe and leaves the wrong block phase.
    pixel[3] <= 8
        || (pixel[0] >= 232 && pixel[1] >= 232 && pixel[2] >= 232)
        || (pixel[0] <= 7 && pixel[1] <= 7 && pixel[2] <= 7)
}

pub(crate) fn has_uniform_rotation_padding(image: &RgbaImage) -> bool {
    if image.width() < 64 || image.height() < 64 {
        return false;
    }
    let probe_side = (image.width().min(image.height()) / 48).clamp(4, 24);
    let corners = [
        (0, 0),
        (image.width() - probe_side, 0),
        (0, image.height() - probe_side),
        (image.width() - probe_side, image.height() - probe_side),
    ];
    let mut counts = [0_u8; 4];
    for (start_x, start_y) in corners {
        let kind = uniform_padding_patch_kind(image, start_x, start_y, probe_side);
        if kind > 0 {
            counts[kind as usize] += 1;
        }
    }
    // 只有至少三个角落呈现同一种纯色或透明补边时才调整搜索顺序，是为了优先处理旋转画布，同时避免普通照片中单个亮角或暗角被误判为编辑器边框。
    // Search ordering changes only when at least three corners share the same solid or transparent padding, prioritizing rotated canvases without mistaking one naturally bright or dark photo corner for an editor border.
    counts.into_iter().skip(1).max().unwrap_or(0) >= 3
}


#[allow(dead_code)]
pub(crate) fn rotation_padding_angle_hints(image: &RgbaImage) -> Vec<f32> {
    if !has_uniform_rotation_padding(image) {
        return Vec::new();
    }
    let maximum_depth = (image.width().min(image.height()) / 3).clamp(24, 384);
    let stride = (maximum_depth / 96).max(1);
    let mut estimates = Vec::new();
    for corner_right in [false, true] {
        for corner_bottom in [false, true] {
            if let Some(angle) = padding_boundary_angle(
                image,
                corner_right,
                corner_bottom,
                true,
                maximum_depth,
                stride,
            ) {
                estimates.push(angle);
            }
            if let Some(angle) = padding_boundary_angle(
                image,
                corner_right,
                corner_bottom,
                false,
                maximum_depth,
                stride,
            ) {
                estimates.push(angle);
            }
        }
    }
    estimates.retain(|value| (0.05..=20.0).contains(value));
    if estimates.is_empty() {
        return Vec::new();
    }
    estimates.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let median = estimates[estimates.len() / 2];
    let nearby: Vec<f32> = estimates
        .into_iter()
        .filter(|value| (*value - median).abs() <= 2.0)
        .collect();
    let magnitude = if nearby.is_empty() {
        median
    } else {
        nearby.iter().sum::<f32>() / nearby.len() as f32
    };
    if !(0.05..=20.0).contains(&magnitude) {
        return Vec::new();
    }
    // 白边斜率提示只服务常见小角快捷路径；±180° 任意角由 10° 分箱归一梯子覆盖，避免把 34° 噪声当成 7° 提示。
    // Padding-slope hints only serve the common small-angle shortcut; arbitrary ±180° angles are covered by the 10° bin-and-refine ladder so 34° noise cannot replace a 7° hint.
    vec![-magnitude, magnitude]
}

#[allow(dead_code)]
fn padding_boundary_angle(
    image: &RgbaImage,
    corner_right: bool,
    corner_bottom: bool,
    scan_rows: bool,
    maximum_depth: u32,
    stride: u32,
) -> Option<f32> {
    let corner_x = if corner_right { image.width() - 1 } else { 0 };
    let corner_y = if corner_bottom { image.height() - 1 } else { 0 };
    let expected_kind = padding_pixel_kind(image.get_pixel(corner_x, corner_y));
    if expected_kind == 0 {
        return None;
    }
    let inward_limit = if scan_rows {
        image.width() / 2
    } else {
        image.height() / 2
    };
    let mut points = Vec::new();
    let mut depth = 0_u32;
    while depth < maximum_depth {
        let mut boundary = None;
        let mut inward = 0_u32;
        while inward < inward_limit {
            let (x, y) = if scan_rows {
                (
                    if corner_right { image.width() - 1 - inward } else { inward },
                    if corner_bottom { image.height() - 1 - depth } else { depth },
                )
            } else {
                (
                    if corner_right { image.width() - 1 - depth } else { depth },
                    if corner_bottom { image.height() - 1 - inward } else { inward },
                )
            };
            if padding_pixel_kind(image.get_pixel(x, y)) != expected_kind {
                boundary = Some(inward as f32);
                break;
            }
            inward += 1;
        }
        if let Some(boundary) = boundary {
            if boundary >= 1.0 {
                points.push((depth as f32, boundary));
            }
        }
        depth = depth.saturating_add(stride);
    }
    if points.len() < 8 {
        return None;
    }
    let minimum = points
        .iter()
        .map(|(_, boundary)| *boundary)
        .fold(f32::INFINITY, f32::min);
    let maximum = points
        .iter()
        .map(|(_, boundary)| *boundary)
        .fold(f32::NEG_INFINITY, f32::max);
    if maximum - minimum < 4.0 {
        return None;
    }
    let count = points.len() as f32;
    let mean_depth = points.iter().map(|(value, _)| *value).sum::<f32>() / count;
    let mean_boundary = points.iter().map(|(_, value)| *value).sum::<f32>() / count;
    let numerator = points
        .iter()
        .map(|(depth, boundary)| (depth - mean_depth) * (boundary - mean_boundary))
        .sum::<f32>();
    let denominator = points
        .iter()
        .map(|(depth, _)| (depth - mean_depth).powi(2))
        .sum::<f32>();
    if denominator <= f32::EPSILON {
        return None;
    }
    let slope = (numerator / denominator).abs();
    if !slope.is_finite() || slope <= 0.001 {
        return None;
    }
    let reduced_slope = slope.min(1.0 / slope);
    let angle = reduced_slope.atan().to_degrees();
    angle.is_finite().then_some(angle)
}

fn uniform_padding_patch_kind(
    image: &RgbaImage,
    start_x: u32,
    start_y: u32,
    side: u32,
) -> u8 {
    let sample_offsets = [0, side / 2, side.saturating_sub(1)];
    let mut expected = 0_u8;
    for offset_y in sample_offsets {
        for offset_x in sample_offsets {
            let kind = padding_pixel_kind(image.get_pixel(start_x + offset_x, start_y + offset_y));
            if kind == 0 {
                return 0;
            }
            if expected == 0 {
                expected = kind;
            } else if expected != kind {
                return 0;
            }
        }
    }
    expected
}

fn padding_pixel_kind(pixel: &Rgba<u8>) -> u8 {
    if pixel[3] <= 8 {
        3
    } else if pixel[0] >= 242 && pixel[1] >= 242 && pixel[2] >= 242 {
        1
    } else if pixel[0] <= 13 && pixel[1] <= 13 && pixel[2] <= 13 {
        2
    } else {
        0
    }
}

pub fn transform_image(image: &DynamicImage, rotation_degrees: f32, scale: f32) -> DynamicImage {
    let source = image.to_rgba8();
    transform_rgba_image(&source, rotation_degrees, scale)
}

pub(crate) fn transform_rgba_image(
    source: &RgbaImage,
    rotation_degrees: f32,
    scale: f32,
) -> DynamicImage {
    let Some(transform) = AffineTransform::new(
        source.width(),
        source.height(),
        rotation_degrees,
        scale,
    ) else {
        return DynamicImage::ImageRgba8(source.clone());
    };
    let Some(output) = materialize_transform(source, &transform) else {
        return DynamicImage::ImageRgba8(source.clone());
    };
    DynamicImage::ImageRgba8(output)
}

pub(crate) fn materialize_transform(
    source: &RgbaImage,
    transform: &AffineTransform,
) -> Option<RgbaImage> {
    materialize_transform_checked(source, transform, || Ok::<(), ()>(()))
        .ok()
        .flatten()
}

pub(crate) fn materialize_exact_orientation_checked<E, F>(
    source: &RgbaImage,
    rotation_degrees: f32,
    mut checkpoint: F,
) -> std::result::Result<Option<RgbaImage>, E>
where
    F: FnMut() -> std::result::Result<(), E>,
{
    if source.width() == 0 || source.height() == 0 || !rotation_degrees.is_finite() {
        return Ok(None);
    }
    let normalized = rotation_degrees.rem_euclid(360.0);
    let orientation = if (normalized - 90.0).abs() < 0.001 {
        90
    } else if (normalized - 180.0).abs() < 0.001 {
        180
    } else if (normalized - 270.0).abs() < 0.001 {
        270
    } else {
        return Ok(None);
    };
    let (output_width, output_height) = if orientation == 180 {
        (source.width(), source.height())
    } else {
        (source.height(), source.width())
    };
    if output_width as u64 * output_height as u64 > MAX_MATERIALIZED_PIXELS {
        return Ok(None);
    }
    let mut output = RgbaImage::new(output_width, output_height);
    for y in 0..output_height {
        if y % 64 == 0 {
            checkpoint()?;
        }
        for x in 0..output_width {
            let (source_x, source_y) = match orientation {
                90 => (y, source.height() - 1 - x),
                180 => (source.width() - 1 - x, source.height() - 1 - y),
                270 => (source.width() - 1 - y, x),
                _ => unreachable!(),
            };
            output.put_pixel(x, y, *source.get_pixel(source_x, source_y));
        }
    }
    checkpoint()?;
    // 直角旋转必须使用整数像素置换而不是通用双线性插值，因为一次不必要的半像素混合就可能削弱频域奇偶间隔，而直角方向本身无需任何重采样。
    // Right-angle rotations use integer pixel permutation rather than generic bilinear interpolation because an unnecessary half-pixel blend can weaken frequency-domain parity spacing while exact orientations require no resampling at all.
    Ok(Some(output))
}

pub(crate) fn materialize_transform_checked<E, F>(
    source: &RgbaImage,
    transform: &AffineTransform,
    mut checkpoint: F,
) -> std::result::Result<Option<RgbaImage>, E>
where
    F: FnMut() -> std::result::Result<(), E>,
{
    if !transform.can_materialize() {
        return Ok(None);
    }
    let mut output = RgbaImage::from_pixel(
        transform.output_width(),
        transform.output_height(),
        Rgba([0, 0, 0, 255]),
    );

    // 大图仿射校正按固定行块检查取消，是为了避免数千万像素的单次循环独占后台线程，让“取消任务”无需等整张临时图生成完才生效。
    // Large affine corrections check cancellation at fixed row blocks so a tens-of-millions-of-pixels loop cannot monopolize the worker and force cancellation to wait for the entire temporary image.
    for y in 0..transform.output_height() {
        if y % 64 == 0 {
            checkpoint()?;
        }
        for x in 0..transform.output_width() {
            if let Some(pixel) = transform.sample_rgba(source, x as f32, y as f32) {
                output.put_pixel(x, y, pixel);
            }
        }
    }
    checkpoint()?;
    Ok(Some(output))
}

pub fn registration_candidates(image: &DynamicImage) -> Vec<RegistrationCandidate> {
    const ANGLES: [f32; 20] = [
        -5.0, -3.0, -2.5, -2.0, -1.0, -0.5, -0.25, -0.1, 0.0, 0.1, 0.25, 0.5,
        1.0, 2.0, 2.5, 3.0, 5.0, 90.0, -90.0, 180.0,
    ];
    const SCALES: [f32; 13] = [
        0.5, 0.625, 0.75, 0.8, 0.875, 0.95, 1.0, 1.05, 1.125, 1.25, 1.333_333_4,
        1.5, 1.75,
    ];
    let mut candidates = Vec::new();
    let source = image.to_rgba8();
    for angle in ANGLES {
        for scale in SCALES {
            if angle == 0.0 && (scale - 1.0).abs() < f32::EPSILON {
                continue;
            }
            let Some(transform) = AffineTransform::new(source.width(), source.height(), angle, scale)
            else {
                continue;
            };
            let Some(output) = materialize_transform(&source, &transform) else {
                continue;
            };
            candidates.push(RegistrationCandidate {
                image: DynamicImage::ImageRgba8(output),
                registration: SyncRegistration {
                    rotation_degrees: angle,
                    scale,
                    translation_x: 0,
                    translation_y: 0,
                    score: 0.0,
                },
            });
        }
    }
    candidates
}

fn nearest_sample(image: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
    if x < 0.0
        || y < 0.0
        || x > image.width().saturating_sub(1) as f32
        || y > image.height().saturating_sub(1) as f32
    {
        return None;
    }
    Some(*image.get_pixel(
        x.round().clamp(0.0, image.width().saturating_sub(1) as f32) as u32,
        y.round().clamp(0.0, image.height().saturating_sub(1) as f32) as u32,
    ))
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
        result[channel] = (top * (1.0 - fy) + bottom * fy)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    Some(Rgba(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_inverse_of_two_degree_portrait_matches_original() {
        let (w, h, deg) = (2728_u32, 3637_u32, 2.0_f32);
        let rad = deg.to_radians();
        let expanded_w = (w as f32 * rad.cos() + h as f32 * rad.sin()).round() as u32;
        let expanded_h = (w as f32 * rad.sin() + h as f32 * rad.cos()).round() as u32;
        assert!((expanded_w as i32 - 2854).abs() <= 1);
        assert!((expanded_h as i32 - 3730).abs() <= 1);
        let t =
            AffineTransform::new_aabb_padded_correction(expanded_w, expanded_h, 2.0, 1.0).unwrap();
        assert!((t.output_width() as i32 - 2728).abs() <= 2);
        assert!((t.output_height() as i32 - 3637).abs() <= 2);
    }

    #[test]
    fn aabb_height_cropped_two_degree_portrait_recovers_original() {
        let (w, h, deg) = (2728_u32, 3637_u32, 2.0_f32);
        let rad = deg.to_radians();
        let expanded_w = (w as f32 * rad.cos() + h as f32 * rad.sin()).round() as u32;
        let t = AffineTransform::new_aabb_axis_cropped_correction(
            expanded_w,
            h,
            2.0,
            1.0,
            true,
        )
        .unwrap();
        assert_eq!(t.output_width(), 2728, "width {}", t.output_width());
        assert_eq!(t.output_height(), 3637);
    }

    #[test]
    fn expanded_canvas_isotropic_misses_true_aabb_height() {
        let t = AffineTransform::new_expanded_canvas_correction(2854, 3729, 2.0, 1.0).unwrap();
        assert!(
            (t.output_height() as i32 - 3637).abs() > 20,
            "document the bug: isotropic inverse height={}",
            t.output_height()
        );
    }

    #[test]
    fn aabb_forty_five_degrees_without_header_is_none() {
        assert!(AffineTransform::new_aabb_padded_correction(1000, 1000, 45.0, 1.0).is_none());
        let t = AffineTransform::new_aabb_padded_correction_with_header(
            1000,
            1000,
            45.0,
            1.0,
            Some((640, 480)),
        )
        .unwrap();
        assert!((t.output_width() as i32 - 640).abs() <= 2);
        assert!((t.output_height() as i32 - 480).abs() <= 2);
    }

    #[test]
    fn aabb_inverse_of_non_integer_angles_matches_original() {
        for (w, h, deg) in [(960_u32, 1280_u32, 17.3_f32), (960, 1280, 41.7)] {
            let rad = deg.to_radians();
            let expanded_w = (w as f32 * rad.cos().abs() + h as f32 * rad.sin().abs()).round() as u32;
            let expanded_h = (w as f32 * rad.sin().abs() + h as f32 * rad.cos().abs()).round() as u32;
            let t = AffineTransform::new_aabb_padded_correction(expanded_w, expanded_h, deg, 1.0)
                .expect("non-singular AABB inverse");
            assert!(
                (t.output_width() as i32 - w as i32).abs() <= 2,
                "width {} vs {w} at {deg}",
                t.output_width()
            );
            assert!(
                (t.output_height() as i32 - h as i32).abs() <= 2,
                "height {} vs {h} at {deg}",
                t.output_height()
            );
        }
    }

    #[test]
    fn fixed_canvas_fit_accepts_angles_beyond_forty_four() {
        assert!(AffineTransform::new_fixed_canvas_fit_correction(640, 480, 47.0, 0.85).is_some());
        assert!(AffineTransform::new_fixed_canvas_fit_correction(640, 480, 137.36, 0.7).is_some());
        assert!(AffineTransform::new_fixed_canvas_fit_correction(640, 480, 0.0, 1.0).is_none());
    }

    #[test]
    fn correction_scale_changes_the_output_canvas() {
        let transform = AffineTransform::new(640, 480, 0.0, 1.5).unwrap();
        assert_eq!(transform.output_width(), 960);
        assert_eq!(transform.output_height(), 720);
    }

    #[test]
    fn inverse_resize_restores_original_dimensions() {
        let transform = AffineTransform::new(1280, 960, 0.0, 0.5).unwrap();
        assert_eq!(transform.output_width(), 640);
        assert_eq!(transform.output_height(), 480);
    }

    #[test]
    fn exact_quarter_turn_round_trip_preserves_every_pixel() {
        let source = RgbaImage::from_fn(7, 4, |x, y| {
            Rgba([(x * 17) as u8, (y * 31) as u8, (x + y) as u8, 255])
        });
        let clockwise = materialize_exact_orientation_checked(&source, 90.0, || Ok::<(), ()>(()))
            .unwrap()
            .unwrap();
        let restored = materialize_exact_orientation_checked(&clockwise, -90.0, || Ok::<(), ()>(()))
            .unwrap()
            .unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn quarter_turn_restores_the_swapped_canvas_shape() {
        let transform = AffineTransform::new(768, 1024, 90.0, 1.0).unwrap();
        assert_eq!(transform.output_width(), 1024);
        assert_eq!(transform.output_height(), 768);
    }

    #[test]
    fn expanded_rotation_canvas_restores_dimensions_without_rescaling_content() {
        let transform = AffineTransform::new_expanded_canvas_correction(1692, 1269, -2.5, 1.0).unwrap();
        assert!((transform.output_width() as i32 - 1600).abs() <= 2);
        assert!((transform.output_height() as i32 - 1200).abs() <= 2);
        assert!((transform.inverse_scale - 1.0).abs() < 0.0001);
    }

    #[test]
    fn fixed_rotation_canvas_restores_content_scale_inside_the_same_dimensions() {
        let transform = AffineTransform::new_fixed_canvas_fit_correction(1600, 1200, -2.5, aspect_canvas_growth(1600, 1200, -2.5).unwrap()).unwrap();
        assert_eq!(transform.output_width(), 1600);
        assert_eq!(transform.output_height(), 1200);
        assert!(transform.inverse_scale < 1.0);
    }

    #[test]
    fn tight_rotated_bounds_recover_the_original_canvas_shape() {
        let transform = AffineTransform::new_tight_bounds_correction(1651, 1269, -2.5, 1.0).unwrap();
        assert!((transform.output_width() as i32 - 1600).abs() <= 2);
        assert!((transform.output_height() as i32 - 1200).abs() <= 2);
    }

    #[test]
    fn tight_rotated_bounds_remove_embedded_content_zoom_from_the_canvas() {
        let transform = AffineTransform::new_tight_bounds_correction(1700, 1307, -2.5, 0.97).unwrap();
        assert!((transform.output_width() as i32 - 1600).abs() <= 4);
        assert!((transform.output_height() as i32 - 1200).abs() <= 4);
    }

    #[test]
    fn inverse_rotation_outer_bands_are_trimmed_without_touching_inner_pixels() {
        let image = RgbaImage::from_fn(120, 96, |x, y| {
            if x < 12 || x >= 108 || y < 8 || y >= 88 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([
                    40 + (x % 80) as u8,
                    60 + (y % 70) as u8,
                    90 + ((x + y) % 60) as u8,
                    255,
                ])
            }
        });
        let trimmed = trim_uniform_border(&image).unwrap();
        // 此测试锁定“只裁外部连通纯色带”的约束，是因为旋转恢复依赖它修正块相位，而内部亮色区域必须保持原样。
        // This test locks in the outer-connected-solid-band constraint because rotation recovery relies on it to repair block phase while bright regions inside the image must remain untouched.
        assert_eq!(trimmed.dimensions(), (96, 80));
        assert_eq!(trimmed.get_pixel(0, 0), image.get_pixel(12, 8));
    }

    #[test]
    fn anti_aliased_near_white_fringe_is_trimmed() {
        let image = RgbaImage::from_fn(120, 96, |x, y| {
            if x < 10 || x >= 110 || y < 8 || y >= 88 {
                Rgba([236, 236, 236, 255])
            } else {
                Rgba([
                    40 + (x % 80) as u8,
                    60 + (y % 70) as u8,
                    90 + ((x + y) % 60) as u8,
                    255,
                ])
            }
        });
        let trimmed = trim_uniform_border(&image).expect("gray-white fringe should count as pad");
        assert_eq!(trimmed.dimensions(), (100, 80));
        assert_eq!(trimmed.get_pixel(0, 0), image.get_pixel(10, 8));
    }

    #[test]
    fn repeated_solid_corners_are_detected_as_rotation_padding() {
        let mut image = RgbaImage::from_pixel(320, 240, Rgba([255, 255, 255, 255]));
        for y in 24..216 {
            for x in 24..296 {
                image.put_pixel(x, y, Rgba([80, 120, 160, 255]));
            }
        }
        assert!(has_uniform_rotation_padding(&image));
    }

    #[test]
    fn solid_rotation_padding_estimates_the_small_angle_before_watermark_search() {
        let width = 320_u32;
        let height = 240_u32;
        let radians = 7.0_f32.to_radians();
        let cosine = radians.cos();
        let sine = radians.sin();
        let center_x = (width as f32 - 1.0) * 0.5;
        let center_y = (height as f32 - 1.0) * 0.5;
        let image = RgbaImage::from_fn(width, height, |x, y| {
            let dx = x as f32 - center_x;
            let dy = y as f32 - center_y;
            let local_x = cosine * dx + sine * dy;
            let local_y = -sine * dx + cosine * dy;
            if local_x.abs() <= 140.0 && local_y.abs() <= 100.0 {
                Rgba([70, 110, 160, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let hints = rotation_padding_angle_hints(&image);
        assert!(hints
            .iter()
            .any(|angle| (angle.abs() - 7.0).abs() <= 0.5));
    }

    #[test]
    fn aspect_preserving_export_keeps_padding_and_angle_hint() {
        let source = RgbaImage::from_fn(640, 480, |x, y| {
            Rgba([
                (40 + (x % 180) as u8),
                (50 + (y % 160) as u8),
                (60 + ((x + y) % 140) as u8),
                255,
            ])
        });
        let width = source.width() as f32;
        let height = source.height() as f32;
        let degrees = 7.0_f32;
        let content_zoom = 1.03_f32;
        let radians = degrees.abs().to_radians();
        let growth = (radians.cos() + height / width * radians.sin())
            .max(radians.cos() + width / height * radians.sin())
            .max(1.0);
        let out_w = (width * growth).round() as u32;
        let out_h = (height * growth).round() as u32;
        let src_cx = (source.width() as f32 - 1.0) * 0.5;
        let src_cy = (source.height() as f32 - 1.0) * 0.5;
        let out_cx = (out_w as f32 - 1.0) * 0.5;
        let out_cy = (out_h as f32 - 1.0) * 0.5;
        let rot = degrees.to_radians();
        let (cos, sin) = (rot.cos(), rot.sin());
        let rotated = RgbaImage::from_fn(out_w, out_h, |x, y| {
            let dx = (x as f32 - out_cx) / content_zoom;
            let dy = (y as f32 - out_cy) / content_zoom;
            let source_x = cos * dx + sin * dy + src_cx;
            let source_y = -sin * dx + cos * dy + src_cy;
            bilinear_sample(&source, source_x, source_y).unwrap_or(Rgba([255, 255, 255, 255]))
        });
        assert!(has_uniform_rotation_padding(&rotated));
        let hints = rotation_padding_angle_hints(&rotated);
        assert!(
            hints.iter().any(|angle| (angle.abs() - 7.0).abs() <= 1.5),
            "hints={hints:?}"
        );
        let trimmed = trim_uniform_border(&rotated).expect("pad should trim");
        assert!(trimmed.width() < rotated.width() || trimmed.height() < rotated.height());
    }

    #[test]
    fn textured_corners_are_not_classified_as_editor_padding() {
        let image = RgbaImage::from_fn(320, 240, |x, y| {
            Rgba([
                ((x * 3 + y) % 220 + 15) as u8,
                ((x + y * 5) % 210 + 20) as u8,
                ((x * 7 + y * 2) % 200 + 25) as u8,
                255,
            ])
        });
        assert!(!has_uniform_rotation_padding(&image));
    }
}
