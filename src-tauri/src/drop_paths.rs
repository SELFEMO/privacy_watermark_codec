use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const MAX_RESOLVED_FILES: usize = 10_000;
const MAX_DIRECTORY_DEPTH: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDropPathsRequest {
    pub paths: Vec<String>,
    pub allowed_extensions: Vec<String>,
    pub expand_directories: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropPathResolution {
    pub files: Vec<String>,
    pub directories: Vec<String>,
    pub ignored_count: usize,
    pub truncated: bool,
}

#[tauri::command]
pub fn resolve_drop_paths(request: ResolveDropPathsRequest) -> DropPathResolution {
    let allowed_extensions = request
        .allowed_extensions
        .into_iter()
        .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
        .filter(|extension| !extension.is_empty())
        .collect::<HashSet<_>>();
    let mut resolution = DropPathResolution {
        files: Vec::new(),
        directories: Vec::new(),
        ignored_count: 0,
        truncated: false,
    };
    let mut seen_files = HashSet::new();
    let mut seen_directories = HashSet::new();

    for raw_path in request.paths {
        let path = PathBuf::from(raw_path);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            resolution.ignored_count += 1;
            continue;
        };

        if metadata.file_type().is_symlink() {
            // 拖入目录递归时不跟随符号链接，避免循环链接把扫描带回父目录或遍历到用户未主动选择的位置。
            // Symlinks are not followed during recursive drops so link cycles cannot return to a parent directory or traverse locations the user did not explicitly choose.
            resolution.ignored_count += 1;
            continue;
        }

        if metadata.is_dir() {
            if request.expand_directories {
                collect_supported_files(
                    &path,
                    &allowed_extensions,
                    &mut resolution,
                    &mut seen_files,
                );
            } else {
                push_unique_path(&path, &mut resolution.directories, &mut seen_directories);
            }
        } else if metadata.is_file() {
            if extension_is_allowed(&path, &allowed_extensions) {
                push_unique_path(&path, &mut resolution.files, &mut seen_files);
            } else {
                resolution.ignored_count += 1;
            }
        } else {
            resolution.ignored_count += 1;
        }

        if resolution.truncated {
            break;
        }
    }

    resolution
}

fn collect_supported_files(
    root: &Path,
    allowed_extensions: &HashSet<String>,
    resolution: &mut DropPathResolution,
    seen_files: &mut HashSet<String>,
) {
    let mut pending = vec![(root.to_path_buf(), 0usize)];

    while let Some((directory, depth)) = pending.pop() {
        if resolution.files.len() >= MAX_RESOLVED_FILES {
            resolution.truncated = true;
            return;
        }
        if depth > MAX_DIRECTORY_DEPTH {
            resolution.ignored_count += 1;
            continue;
        }

        let Ok(read_dir) = fs::read_dir(&directory) else {
            resolution.ignored_count += 1;
            continue;
        };
        let mut entries = read_dir.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            if resolution.files.len() >= MAX_RESOLVED_FILES {
                resolution.truncated = true;
                return;
            }

            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                resolution.ignored_count += 1;
                continue;
            };
            if file_type.is_symlink() {
                resolution.ignored_count += 1;
            } else if file_type.is_dir() {
                pending.push((path, depth + 1));
            } else if file_type.is_file() && extension_is_allowed(&path, allowed_extensions) {
                push_unique_path(&path, &mut resolution.files, seen_files);
            } else {
                resolution.ignored_count += 1;
            }
        }
    }
}

fn extension_is_allowed(path: &Path, allowed_extensions: &HashSet<String>) -> bool {
    if allowed_extensions.is_empty() {
        return false;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| allowed_extensions.contains(&extension.to_ascii_lowercase()))
        .unwrap_or(false)
}

fn push_unique_path(path: &Path, destination: &mut Vec<String>, seen: &mut HashSet<String>) {
    let display_path = path.to_string_lossy().into_owned();
    // Windows 路径大小写不敏感，拖入同一文件的不同大小写形式时按同一项处理；其它平台保留原生大小写语义。
    // Windows paths are case-insensitive, so differently cased forms of the same dropped file are deduplicated; other platforms retain native case semantics.
    let identity = if cfg!(target_os = "windows") {
        display_path.to_lowercase()
    } else {
        display_path.clone()
    };
    if seen.insert(identity) {
        destination.push(display_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_supported_files_from_a_dropped_directory() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pwc-drop-test-{unique}"));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create test directory");
        fs::write(root.join("first.png"), b"png").expect("write image");
        fs::write(nested.join("second.JPG"), b"jpg").expect("write image");
        fs::write(nested.join("ignored.txt"), b"text").expect("write text");

        let result = resolve_drop_paths(ResolveDropPathsRequest {
            paths: vec![root.to_string_lossy().into_owned()],
            allowed_extensions: vec!["png".into(), "jpg".into()],
            expand_directories: true,
        });

        assert_eq!(result.files.len(), 2);
        assert_eq!(result.ignored_count, 1);
        assert!(!result.truncated);
        fs::remove_dir_all(root).expect("remove test directory");
    }
}
