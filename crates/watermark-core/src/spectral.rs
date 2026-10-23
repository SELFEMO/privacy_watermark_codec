use crate::{recovery, Result};
use image::RgbaImage;
use rustfft::{num_complex::Complex32, FftPlanner};

pub(crate) fn fft_square(values: &mut [Complex32], side: usize) -> Result<()> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(side);
    for row in values.chunks_exact_mut(side) {
        fft.process(row);
    }
    let mut column = vec![Complex32::default(); side];
    for x in 0..side {
        if x % 16 == 0 {
            recovery::checkpoint()?;
        }
        for y in 0..side {
            column[y] = values[y * side + x];
        }
        fft.process(&mut column);
        for y in 0..side {
            values[y * side + x] = column[y];
        }
    }
    Ok(())
}

pub(crate) fn luma(image: &RgbaImage, x: u32, y: u32) -> f32 {
    let p = image.get_pixel(x, y);
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SpectralHint {
    pub angle: f32,
    pub scale: f32,
    pub score: f32,
    qx: f32,
    qy: f32,
    pub offset: Option<(f32, f32)>,
}

impl SpectralHint {
    pub(crate) fn new(angle: f32, scale: f32, score: f32) -> Self {
        Self {
            angle,
            scale,
            score,
            qx: 0.0,
            qy: 0.0,
            offset: None,
        }
    }
}

pub(crate) fn grid_hints(image: &RgbaImage) -> Result<Vec<SpectralHint>> {
    let min_side = image.width().min(image.height()) as usize;
    if min_side < 128 {
        return Ok(Vec::new());
    }
    let side = if min_side >= 1024 {
        1024
    } else if min_side >= 256 {
        256
    } else {
        128
    };
    let mut spectrum = vec![0.0_f32; side * side];
    let window: Vec<_> = (0..side)
        .map(|i| (std::f32::consts::PI * 2.0 * i as f32 / (side - 1) as f32).cos() * -0.5 + 0.5)
        .collect();
    // 在多个相隔较远的窗口中平均块边缘能量谱，保留随旋转缩放移动的块周期，同时削弱单个物体边缘。
    // Average block-edge energy spectra in separated windows to preserve the moving block lattice while reducing isolated object edges.
    for (fx, fy) in [(0.2, 0.2), (0.8, 0.2), (0.2, 0.8), (0.8, 0.8)] {
        recovery::checkpoint()?;
        let ox = ((image.width() as usize - side) as f32 * fx) as u32;
        let oy = ((image.height() as usize - side) as f32 * fy) as u32;
        let mut energies = vec![0.0_f32; side * side];
        for y in 0..side {
            for x in 0..side {
                let px = ox + x as u32;
                let py = oy + y as u32;
                let value = 2.0 * luma(image, px, py)
                    - luma(image, px.saturating_sub(1), py)
                    - luma(image, px, py.saturating_sub(1));
                energies[y * side + x] = value * value;
            }
        }
        let mut ordered = energies.clone();
        let index = ordered.len() * 85 / 100;
        ordered.select_nth_unstable_by(index, |a, b| a.total_cmp(b));
        let cap = ordered[index].max(1.0);
        let mean = energies.iter().map(|value| value.min(cap)).sum::<f32>() / energies.len() as f32;
        let mut data: Vec<_> = energies
            .iter()
            .enumerate()
            .map(|(i, value)| {
                Complex32::new(
                    (value.min(cap) - mean) * window[i % side] * window[i / side],
                    0.0,
                )
            })
            .collect();
        fft_square(&mut data, side)?;
        let mut magnitudes: Vec<_> = data.iter().map(|value| value.norm()).collect();
        let middle = magnitudes.len() / 2;
        magnitudes.select_nth_unstable_by(middle, |a, b| a.total_cmp(b));
        let background = magnitudes[middle].max(0.01);
        for (sum, value) in spectrum.iter_mut().zip(data.iter()) {
            *sum += value.norm() / background;
        }
    }
    let frequency = |index: usize| {
        let signed = if index > side / 2 {
            index as i32 - side as i32
        } else {
            index as i32
        };
        signed as f32 / side as f32
    };
    let mut peaks = Vec::new();
    let index = |x: isize, y: isize| {
        y.rem_euclid(side as isize) as usize * side + x.rem_euclid(side as isize) as usize
    };
    for y in 0..side / 2 {
        for x in 0..side {
            let u = frequency(x);
            let v = frequency(y);
            let radius = u.hypot(v);
            if !(0.03..=0.48).contains(&radius) {
                continue;
            }
            let score = spectrum[y * side + x];
            if [-1_isize, 0, 1].iter().all(|dy| {
                [-1_isize, 0, 1].iter().all(|dx| {
                    (*dx == 0 && *dy == 0)
                        || score >= spectrum[index(x as isize + dx, y as isize + dy)]
                })
            }) {
                // 三点抛物线把 FFT 分箱精化到亚像素频率，几何精度随后由公开头检验。
                // Three-point interpolation refines FFT bins to subpixel frequencies; the public header subsequently validates geometric accuracy.
                let refine = |a: f32, b: f32, c: f32| {
                    let den = a - 2.0 * b + c;
                    if den.abs() > 1e-6 {
                        (0.5 * (a - c) / den).clamp(-0.5, 0.5)
                    } else {
                        0.0
                    }
                };
                let dx = refine(
                    spectrum[index(x as isize - 1, y as isize)],
                    score,
                    spectrum[index(x as isize + 1, y as isize)],
                );
                let dy = refine(
                    spectrum[index(x as isize, y as isize - 1)],
                    score,
                    spectrum[index(x as isize, y as isize + 1)],
                );
                peaks.push((u + dx / side as f32, v + dy / side as f32, score));
            }
        }
    }
    peaks.sort_by(|a, b| b.2.total_cmp(&a.2));
    peaks.truncate(100);
    let mut hints = Vec::new();
    for &(u, v, power) in &peaks {
        let radius = u.hypot(v);
        let mut angle = v.atan2(u).to_degrees();
        angle = (angle + 45.0).rem_euclid(90.0) - 45.0;
        let partner = peaks
            .iter()
            .filter(|&&(x, y, _)| {
                ((x.hypot(y) / radius) - 1.0).abs() < 0.025
                    && ((x * u + y * v) / (radius * radius)).abs() < 0.025
            })
            .map(|p| p.2)
            .fold(0.0_f32, f32::max);
        if partner > 5.0 || power > 20.0 {
            let scale = radius * 8.0;
            hints.push(SpectralHint {
                angle: -angle,
                scale,
                score: power * (1.0 + partner.min(power) / power),
                qx: u,
                qy: v,
                offset: None,
            });
        }
    }
    hints.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut unique: Vec<SpectralHint> = Vec::new();
    for hint in hints {
        if !unique
            .iter()
            .any(|old| (old.angle - hint.angle).abs() < 0.3)
        {
            unique.push(hint);
        }
        if unique.len() == 8 {
            break;
        }
    }
    let mut samples = Vec::new();
    let mut seed = 17_u64;
    for _ in 0..49152 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = 1 + (seed >> 32) as u32 % (image.width() - 2);
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let y = 1 + (seed >> 32) as u32 % (image.height() - 2);
        let value = 2.0 * luma(image, x, y) - luma(image, x - 1, y) - luma(image, x, y - 1);
        samples.push((x as f32, y as f32, value * value));
    }
    let mut energies: Vec<_> = samples.iter().map(|p| p.2).collect();
    let cap_index = energies.len() * 85 / 100;
    energies.select_nth_unstable_by(cap_index, |a, b| a.total_cmp(b));
    let cap = energies[cap_index].max(1.0);
    let mean = samples.iter().map(|p| p.2.min(cap)).sum::<f32>() / samples.len() as f32;
    for p in &mut samples {
        p.2 = p.2.min(cap) - mean;
    }
    for hint in &mut unique {
        let mut qx = hint.qx;
        let mut qy = hint.qy;
        let power = |u: f32, v: f32| {
            let mut real = 0.0;
            let mut imaginary = 0.0;
            for &(x, y, value) in &samples {
                let (sin, cos) = (std::f32::consts::TAU * (u * x + v * y)).sin_cos();
                real += value * cos;
                imaginary += value * sin;
            }
            real * real + imaginary * imaginary
        };
        let mut step = 0.5 / side as f32;
        for _ in 0..8 {
            recovery::checkpoint()?;
            let mut best = (power(qx, qy), qx, qy);
            for dx in [-1.0, 0.0, 1.0] {
                for dy in [-1.0, 0.0, 1.0] {
                    let x = qx + dx * step;
                    let y = qy + dy * step;
                    let value = power(x, y);
                    if value > best.0 {
                        best = (value, x, y);
                    }
                }
            }
            qx = best.1;
            qy = best.2;
            step *= 0.35;
        }
        hint.angle = -((qy.atan2(qx).to_degrees() + 45.0).rem_euclid(90.0) - 45.0);
        hint.scale = qx.hypot(qy) * 8.0;
    }
    if std::env::var_os("PWC_GEOMETRY_DIAGNOSTICS").is_some() {
        eprintln!("spectral hints: {unique:?}");
    }
    Ok(unique)
}
