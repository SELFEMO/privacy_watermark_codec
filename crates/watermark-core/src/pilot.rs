use crate::{
    recovery,
    spectral::{fft_square, luma, SpectralHint},
    Result,
};
use image::RgbaImage;
use rustfft::num_complex::Complex32;

const PERIOD: f32 = 128.0;
const WAVES: [(f32, f32, f32); 4] = [
    (5.0, 1.0, 0.44879895),
    (2.0, 7.0, 0.62831855),
    (7.0, 4.0, 1.0471976),
    (4.0, 7.0, 0.28559932),
];
const AMPLITUDE: f32 = 0.85;

pub(crate) fn embed(image: &mut RgbaImage) {
    // 块内采用相同整数偏移，只改变直流均值，避免导频直接污染正文使用的 Haar 方向差。
    // A constant integer offset per block changes only its DC mean, keeping the pilot from directly polluting the body's Haar directional difference.
    for by in 0..image.height() / 8 {
        for bx in 0..image.width() / 8 {
            let x = bx as f32 * 8.0 + 3.5;
            let y = by as f32 * 8.0 + 3.5;
            let delta = WAVES
                .iter()
                .map(|&(u, v, phase)| {
                    (std::f32::consts::TAU * (u * x + v * y) / PERIOD + phase).sin() * AMPLITUDE
                })
                .sum::<f32>()
                .round() as i16;
            let mut low = -255_i16;
            let mut high = 255_i16;
            let mut transparent = false;
            for y in by * 8..by * 8 + 8 {
                for x in bx * 8..bx * 8 + 8 {
                    let p = image.get_pixel(x, y);
                    transparent |= p[3] < 255;
                    for c in 0..3 {
                        low = low.max(-(p[c] as i16));
                        high = high.min(255 - p[c] as i16);
                    }
                }
            }
            if transparent {
                continue;
            }
            let delta = delta.clamp(low, high);
            for y in by * 8..by * 8 + 8 {
                for x in bx * 8..bx * 8 + 8 {
                    let p = image.get_pixel_mut(x, y);
                    for c in 0..3 {
                        p[c] = (p[c] as i16 + delta).clamp(0, 255) as u8;
                    }
                }
            }
        }
    }
}

pub(crate) fn hints(image: &RgbaImage) -> Result<Vec<SpectralHint>> {
    let min = image.width().min(image.height()) as usize;
    if min < 512 {
        return Ok(Vec::new());
    }
    let side = if min >= 1024 { 1024 } else { 512 };
    let mut magnitude = vec![0.0_f32; side * side];
    let window: Vec<_> = (0..side)
        .map(|x| 0.5 - 0.5 * (std::f32::consts::TAU * x as f32 / (side - 1) as f32).cos())
        .collect();
    for (fx, fy) in [(0.2, 0.2), (0.8, 0.2), (0.2, 0.8), (0.8, 0.8)] {
        recovery::checkpoint()?;
        let ox = ((image.width() as usize - side) as f32 * fx) as u32;
        let oy = ((image.height() as usize - side) as f32 * fy) as u32;
        let mut data = vec![Complex32::default(); side * side];
        for y in 0..side {
            for x in 0..side {
                let px = ox + x as u32;
                let py = oy + y as u32;
                let high = 4.0 * luma(image, px, py)
                    - luma(image, px.saturating_sub(1), py)
                    - luma(image, (px + 1).min(image.width() - 1), py)
                    - luma(image, px, py.saturating_sub(1))
                    - luma(image, px, (py + 1).min(image.height() - 1));
                data[y * side + x] = Complex32::new(high * window[x] * window[y], 0.0);
            }
        }
        fft_square(&mut data, side)?;
        for (sum, value) in magnitude.iter_mut().zip(data.iter()) {
            *sum += value.norm();
        }
    }
    let wrapped = |index: i32| index.rem_euclid(side as i32) as usize;
    let contrast = |x: i32, y: i32| {
        let value = magnitude[wrapped(y) * side + wrapped(x)];
        let mut background = 0.0;
        for (dx, dy) in [
            (-5, 0),
            (5, 0),
            (0, -5),
            (0, 5),
            (-4, -4),
            (4, 4),
            (-4, 4),
            (4, -4),
        ] {
            background += magnitude[wrapped(y + dy) * side + wrapped(x + dx)];
        }
        value / (background / 8.0).max(0.01)
    };
    let at = |x: f32, y: f32| {
        contrast(
            (x * side as f32).round() as i32,
            (y * side as f32).round() as i32,
        )
    };
    let mut peaks = Vec::new();
    for y in 1..side / 4 {
        for x in 1..side - 1 {
            let u = if x > side / 2 {
                x as f32 / side as f32 - 1.0
            } else {
                x as f32 / side as f32
            };
            let v = y as f32 / side as f32;
            if !(0.008..=0.12).contains(&u.hypot(v)) {
                continue;
            }
            let value = contrast(x as i32, y as i32);
            if value > 2.0 {
                peaks.push((u, v, value));
            }
        }
    }
    peaks.sort_by(|a, b| b.2.total_cmp(&a.2));
    peaks.truncate(300);
    let mut candidates = Vec::new();
    for (u, v, _) in peaks {
        for &(x, y, _) in &WAVES {
            let angle = v.atan2(u) - y.atan2(x);
            let scale = (x.hypot(y) / PERIOD) / u.hypot(v);
            if !(0.5..=2.0).contains(&scale) {
                continue;
            }
            let (sin, cos) = angle.sin_cos();
            let mut values = Vec::new();
            for &(a, b, _) in &WAVES {
                values.push(at(
                    (cos * a - sin * b) / PERIOD / scale,
                    (sin * a + cos * b) / PERIOD / scale,
                ));
            }
            values.sort_by(|a, b| a.total_cmp(b));
            // 至少三组非共线导频共同吻合才能作为几何先验；最终是否有水印仍由头 CRC 和正文认证决定。
            // At least three non-collinear pilots must agree before proposing geometry; header CRC and payload authentication still decide watermark validity.
            if values[1] > 2.5 && values[0] > 1.3 {
                candidates.push(SpectralHint::new(
                    -angle.to_degrees(),
                    1.0 / scale,
                    values.iter().map(|x| x.min(20.0)).sum::<f32>(),
                ));
            }
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut unique: Vec<SpectralHint> = Vec::new();
    for candidate in candidates {
        if !unique.iter().any(|old| {
            (old.angle - candidate.angle).abs() < 1.0 && (old.scale - candidate.scale).abs() < 0.02
        }) {
            unique.push(candidate);
        }
        if unique.len() == 2 {
            break;
        }
    }
    refine(image, &mut unique, side)?;
    if std::env::var_os("PWC_GEOMETRY_DIAGNOSTICS").is_some() {
        eprintln!("pilot hints: {unique:?}");
    }
    Ok(unique)
}

fn refine(image: &RgbaImage, hints: &mut [SpectralHint], side: usize) -> Result<()> {
    let mut samples = Vec::new();
    let mut seed = 31_u64;
    for _ in 0..65536 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = 1 + (seed >> 32) as u32 % (image.width() - 2);
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let y = 1 + (seed >> 32) as u32 % (image.height() - 2);
        let value = 4.0 * luma(image, x, y)
            - luma(image, x - 1, y)
            - luma(image, x + 1, y)
            - luma(image, x, y - 1)
            - luma(image, x, y + 1);
        samples.push((x as f32, y as f32, value.clamp(-8.0, 8.0)));
    }
    let project = |u: f32, v: f32| {
        let mut value = Complex32::default();
        for &(x, y, weight) in &samples {
            let (sin, cos) = (std::f32::consts::TAU * (u * x + v * y)).sin_cos();
            value.re += weight * cos;
            value.im -= weight * sin;
        }
        value
    };
    for hint in hints {
        let theta = (-hint.angle).to_radians();
        let (sin, cos) = theta.sin_cos();
        let mut measured = Vec::new();
        for &(x, y, phase) in &WAVES {
            let mut u = (cos * x - sin * y) * hint.scale / PERIOD;
            let mut v = (sin * x + cos * y) * hint.scale / PERIOD;
            let mut step = 0.5 / side as f32;
            for _ in 0..8 {
                recovery::checkpoint()?;
                let mut best = (project(u, v).norm_sqr(), u, v);
                for dx in [-1.0, 0.0, 1.0] {
                    for dy in [-1.0, 0.0, 1.0] {
                        let a = u + dx * step;
                        let b = v + dy * step;
                        let power = project(a, b).norm_sqr();
                        if power > best.0 {
                            best = (power, a, b);
                        }
                    }
                }
                u = best.1;
                v = best.2;
                step *= 0.35;
            }
            measured.push((x / PERIOD, y / PERIOD, u, v, phase));
        }
        // 多个导频共同拟合相似变换，避免单个天然纹理峰决定角度和缩放。
        // Jointly fit the similarity transform from several pilots so one natural texture peak cannot determine rotation and scale.
        let denominator = measured.iter().map(|p| p.0 * p.0 + p.1 * p.1).sum::<f32>();
        let a = measured.iter().map(|p| p.2 * p.0 + p.3 * p.1).sum::<f32>() / denominator;
        let b = measured.iter().map(|p| p.3 * p.0 - p.2 * p.1).sum::<f32>() / denominator;
        hint.angle = -b.atan2(a).to_degrees();
        hint.scale = a.hypot(b);
        let (sin, cos) = hint.angle.to_radians().sin_cos();
        let cx = (image.width() as f32 - 1.0) / 2.0;
        let cy = (image.height() as f32 - 1.0) / 2.0;
        let ox = ((image.width() as f32 * hint.scale).round() - 1.0) / 2.0;
        let oy = ((image.height() as f32 * hint.scale).round() - 1.0) / 2.0;
        let offset_x = ox - hint.scale * (cos * cx - sin * cy);
        let offset_y = oy - hint.scale * (sin * cx + cos * cy);
        let mut phases = Vec::new();
        for &(x, y, _, _, phase) in &measured {
            let u = a * x - b * y;
            let v = b * x + a * y;
            let measured_phase =
                project(u, v).arg() - std::f32::consts::TAU * (x * offset_x + y * offset_y);
            phases.push((x, y, measured_phase - phase + std::f32::consts::FRAC_PI_2));
        }
        let score = |x: f32, y: f32| {
            phases
                .iter()
                .map(|&(u, v, phase)| (std::f32::consts::TAU * (u * x + v * y) - phase).cos())
                .sum::<f32>()
        };
        let mut best = (-f32::INFINITY, 0.0, 0.0);
        for y in 0..128 {
            for x in 0..128 {
                let value = score(x as f32, y as f32);
                if value > best.0 {
                    best = (value, x as f32, y as f32);
                }
            }
        }
        let mut step = 0.5;
        for _ in 0..5 {
            let initial = best;
            for dx in [-1.0, 0.0, 1.0] {
                for dy in [-1.0, 0.0, 1.0] {
                    let x = initial.1 + dx * step;
                    let y = initial.2 + dy * step;
                    let value = score(x, y);
                    if value > best.0 {
                        best = (value, x, y);
                    }
                }
            }
            step *= 0.5;
        }
        hint.offset = Some(((-best.1).rem_euclid(8.0), (-best.2).rem_euclid(8.0)));
    }
    Ok(())
}
