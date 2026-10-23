use std::{env, path::PathBuf, time::Instant};
use watermark_core::{extract_image_file, KeyFile, KeySource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 4 {
        return Err("usage: inspect_corpus IMAGE_DIRECTORY KEY REFERENCE_WATERMARKED_IMAGE".into());
    }
    let key = KeySource::KeyFile(KeyFile::read(&args[2])?);
    // 参考水印图仅用于在内存中核对最终正文；每个待测图片仍独立解码，不能把参考图传给几何恢复。
    // The reference watermark is used only for an in-memory text comparison; each input is decoded independently without passing reference pixels to registration.
    let reference = extract_image_file(&args[3], &key)?;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&args[1])?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    paths.retain(|path| path.extension().is_some_and(|extension| extension == "png"));
    paths.sort();
    let mut passed = 0;
    for path in &paths {
        let started = Instant::now();
        match extract_image_file(path, &key) {
            Ok(report) => {
                let matches = report.text == reference.text;
                passed += usize::from(matches);
                // 只输出相等与否，不输出正文或其哈希，避免短文本被日志或字典猜测泄漏。
                // Log only equality, never text or its hash, preventing logs or dictionary guesses from revealing short payloads.
                println!(
                    "file={} authenticated=true same_text={} elapsed={:.3}s",
                    path.file_name().unwrap().to_string_lossy(),
                    matches,
                    started.elapsed().as_secs_f64()
                );
            }
            Err(error) => println!(
                "file={} authenticated=false error={error}",
                path.file_name().unwrap().to_string_lossy()
            ),
        }
    }
    println!("matching={passed} total={}", paths.len());
    if passed != paths.len() || paths.is_empty() {
        return Err("corpus authentication or text equality failed".into());
    }
    Ok(())
}
