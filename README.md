# Privacy Watermark Codec

[English](README.md) | [简体中文](README.zh-CN.md)

Privacy Watermark Codec is a local desktop app built with Tauri, Vue, and Rust. It embeds encrypted invisible privacy watermarks into images and videos, extracts watermark text with a `.key` file or the original custom password, and reports likely media tampering.

The goal is not to add visible marks to images. The app embeds encrypted information into image frequency data while preserving visual quality, so it can be used for ownership claims, content tracing, and tamper inspection.

## Features

- Single-image, batch-image, and video watermark encoding.
- Image and video watermark decoding.
- Independent key, shared batch key, and custom-password key modes.
- PBKDF2-HMAC-SHA256 key derivation and ChaCha20-Poly1305 encrypted payloads.
- DCT mid-frequency embedding, BCH error correction, synchronization-template assisted registration, and repeated spatial voting.
- Global and partitioned perceptual fingerprint reporting for likely tamper status and suspicious-region localization.
- Unknown-image scan for project watermark headers and common privacy/AI metadata traces.
- Local-only processing. Media, passwords, and key files are not uploaded.
- Chinese and English UI.
- Grouped Windows image right-click menu entries.

## Clone or download

Recommended clone flow:

```text
git clone https://github.com/SELFEMO/privacy_watermark_codec.git
cd privacy_watermark_codec
```

If you only need to inspect the source code, GitHub **Code > Download ZIP** can also be used. However, GitHub source ZIP archives may not include large Git LFS objects, so they should not be treated as complete build-ready packages.

If FFmpeg binaries in the repository are managed by Git LFS, run these commands after cloning:

```text
git lfs install
git lfs pull
```

These Git commands are the same on Windows, macOS, and Linux. Only the local project path differs, such as `D:\MyWorkstation\Learning\Rust\privacy_watermark_codec` on Windows or `~/Learning/Rust/privacy_watermark_codec` on macOS/Linux.

## Prepare FFmpeg files

Video encoding and decoding require `ffmpeg` and `ffprobe`. The project reads bundled FFmpeg files from:

```text
src-tauri/vendor/ffmpeg/
```

You can use the FFmpeg files already uploaded with the repository, or you can download or build FFmpeg yourself and place the files into the expected directory. The official FFmpeg website is `https://ffmpeg.org/`, and the official download page is `https://ffmpeg.org/download.html`. The official download page mainly provides source code and links to compiled package providers. Windows users can choose a compiled build from the providers listed on that page.

Minimum required files:

```text
src-tauri/vendor/ffmpeg/windows_x64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_x64/ffprobe.exe

src-tauri/vendor/ffmpeg/windows_arm64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffprobe.exe

src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg
src-tauri/vendor/ffmpeg/macos_arm64/ffprobe

src-tauri/vendor/ffmpeg/macos_x64/ffmpeg
src-tauri/vendor/ffmpeg/macos_x64/ffprobe

src-tauri/vendor/ffmpeg/linux_x64/ffmpeg
src-tauri/vendor/ffmpeg/linux_x64/ffprobe

src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg
src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
```

`ffplay` is optional and is not required for watermark encoding or decoding.

After copying or replacing FFmpeg files, refresh the FFmpeg manifest from the project root:

```text
npm run ffmpeg:manifest
```

On macOS and Linux, make runtime files executable before packaging. Replace the platform directory with the one you actually use:

```text
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
```

## When Git LFS does not restore all FFmpeg files

If you are sure the FFmpeg files exist in GitHub LFS but `git lfs pull` or `git lfs fetch --all` still leaves local files incomplete, handle it in this order.

First, check whether local LFS include/exclude filters are active. Those filters can download only part of the paths:

```text
git config --show-origin --get-regexp "lfs\.(fetchinclude|fetchexclude)"
```

If the command prints `lfs.fetchinclude` or `lfs.fetchexclude`, remove repository and global filters:

```text
git config --local --unset-all lfs.fetchinclude
git config --local --unset-all lfs.fetchexclude
git config --global --unset-all lfs.fetchinclude
git config --global --unset-all lfs.fetchexclude
```

If one line says the key does not exist, ignore it and continue.

Second, fetch only the FFmpeg directory and explicitly check cached LFS objects out into the working tree:

```text
git lfs install --force
git lfs fetch origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
git lfs checkout
git lfs pull origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
```

The important command is `git lfs checkout`. `git lfs fetch --all` downloads objects into the local LFS cache, but it does not always replace pointer text files in the working tree. `git lfs checkout` writes the real binaries from the local LFS cache back into the working tree.

Third, check whether the working tree still contains pointer files. Real `ffmpeg.exe` or `ffmpeg` files are normally not tiny text files:

```text
git lfs ls-files
```

If FFmpeg entries are listed but local files are still small pointer files, run:

```text
git lfs checkout src-tauri/vendor/ffmpeg
```

If this still fails, the problem is usually not ordinary clone behavior. Common causes are: the current branch does not reference those LFS objects, the objects were not pushed to the same remote, repository LFS permission/quota problems, or local proxy/network blocking. The repository maintainer should push LFS objects again using `GITHUB_UPLOAD_COMMANDS.md`.

## Install, start, and package

Install frontend dependencies:

```text
npm install
```

Start development mode:

```text
npm run tauri:dev
```

Build installer/package:

```text
npm run tauri:build
```

The Windows NSIS installer is generated under:

```text
target/release/bundle/nsis/
```

## Usage

### Encode watermark

1. Open the app and enter encode mode.
2. Select image or video files.
3. Select the output directory.
4. Enter the watermark text.
5. Choose independent key, shared batch key, or custom password.
6. Start encoding.
7. Keep the output media, `.key` file, and evidence manifest.

### Decode watermark

1. Open the app and enter decode mode.
2. Select encoded images or videos.
3. Select the `.key` file or enter the original custom password.
4. Start decoding and inspection.
5. Review extracted watermark text, tamper status, and suspicious regions.

### Scan unknown images

1. Select one or more unknown images, or import them from the Windows right-click menu.
2. Start scan.
3. Review whether project watermark headers, privacy metadata, or AI watermark traces are found.

A positive project-header scan means an encrypted project watermark may be present. It does not reveal watermark text without the key/password. A negative scan result is not proof that the image is watermark-free.

## Current validation status

| Platform | Current status | Notes |
| --- | --- | --- |
| Windows x64 | Main verified path | Main development, debugging, and packaging target. NSIS is the default installer format. |
| Windows ARM64 | Not tested | Directory placeholder exists. Requires matching FFmpeg files and platform validation. |
| macOS ARM64 | Not tested | Requires macOS environment, FFmpeg executable permissions, signing/notarization decisions, and validation. |
| macOS x64 | Not tested | Requires matching build environment and platform validation. |
| Linux x64 | Not tested | Requires Linux Tauri system dependencies, matching FFmpeg files, and platform validation. |
| Linux ARM64 | Not tested | Requires matching build environment, matching FFmpeg files, and platform validation. |

## Runtime storage policy

The app does not use `%APPDATA%` as its own runtime data directory. Runtime data is stored beside the executable:

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

The Windows installer tries to choose a non-system drive when possible. Uninstall removes `PrivacyWatermarkCodecData` from the install directory.

## Windows right-click menu

The Windows installer registers a grouped image context menu:

```text
Privacy Watermark Codec
├─ 编码隐私水印 / Encode privacy watermark
├─ 检查隐私水印 / Decode and inspect
└─ 无密钥扫描 / Keyless scan
```

The submenu uses the app icon and supports multi-select. Selected files are merged into the app through the single-instance handler.

## Key file warning

`.key` files contain derived decryption material. Treat them like password files. If a key file is exposed, the corresponding watermark text may be recovered.

## Project structure

```text
privacy-watermark-codec/
├─ src/                         Vue frontend
├─ src/components/              UI components and brand mark
├─ src-tauri/                   Tauri shell, commands, installer config, video orchestration
├─ src-tauri/icons/             App icons
├─ src-tauri/vendor/ffmpeg/     Bundled FFmpeg binaries and manifest
├─ src-tauri/windows/           NSIS installer hooks
├─ crates/watermark-core/       Rust watermark, crypto, key, scan, and tamper core
├─ scripts/                     Build, release, and manifest scripts
└─ .github/workflows/           Windows release workflow
```

## AI development notice

This project was implemented with AI-assisted programming. Before public release or production use, review the code, verify watermark behavior with your own sample set, and confirm all third-party binary licenses.
