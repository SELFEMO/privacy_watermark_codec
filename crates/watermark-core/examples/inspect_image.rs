use std::{
    env,
    path::Path,
    time::{Duration, Instant},
};
use watermark_core::{
    extract_image_file_with_recovery, ExtractOptions, KeyFile, KeySource, RecoveryMode,
    RecoveryOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        return Err("usage: inspect_image IMAGE KEY [direct|probe|SECONDS]".into());
    }
    if args.get(3).is_some_and(|value| value == "probe") {
        println!(
            "{:?}",
            watermark_core::watermark::probe_embedded_header_file(&args[1])?.map(|h| (
                h.algorithm,
                h.original_width,
                h.original_height,
                h.strength
            ))
        );
        return Ok(());
    }
    let key = KeySource::KeyFile(KeyFile::read(&args[2])?);
    let direct = args.get(3).is_some_and(|value| value == "direct");
    let budget = args
        .get(3)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10);
    let start = Instant::now();
    let mut last_stage = "";
    let mut events = 0;
    let result = extract_image_file_with_recovery(
        Path::new(&args[1]),
        &key,
        ExtractOptions {
            allow_registration: !direct,
        },
        RecoveryOptions {
            mode: if budget > 10 {
                RecoveryMode::Deep
            } else {
                RecoveryMode::Standard
            },
            max_duration: Some(Duration::from_secs(budget)),
            cancellation: None,
        },
        |progress| {
            events += 1;
            // 诊断只记录阶段和耗时，避免将水印正文或密钥写入终端日志。
            // Diagnostics log only stages and timings to keep watermark text and keys out of terminal logs.
            if progress.stage != last_stage {
                eprintln!(
                    "{:.3}s stage={} angle={:.4} scale={:.6}",
                    start.elapsed().as_secs_f64(),
                    progress.stage,
                    progress.rotation_degrees,
                    progress.scale
                );
                last_stage = progress.stage;
            }
            Ok(())
        },
    );
    match result {
        Ok(report) => println!(
            "ok elapsed={:.3}s chars={} integrity={:?} angle={:.5} scale={:.7} events={events}",
            start.elapsed().as_secs_f64(),
            report.text.chars().count(),
            report.integrity,
            report.sync_registration.rotation_degrees,
            report.sync_registration.scale
        ),
        Err(error) => {
            println!(
                "failed elapsed={:.3}s error={error} events={events}",
                start.elapsed().as_secs_f64()
            );
            std::process::exit(1);
        }
    }
    Ok(())
}
