# Privacy Watermark Codec

[English](README.md) | [简体中文](README.zh-CN.md)

Privacy Watermark Codec is a local-first desktop app built with Tauri, Vue, and Rust. It embeds encrypted invisible privacy watermarks into images and videos, extracts watermarks with a key/password, performs perceptual tamper detection, and scans unknown images for likely privacy/AI watermark traces.

## AI development notice

This project was implemented with AI-assisted programming. Before public release or production use, review the code, test the target platforms, verify the watermark behavior with your own sample set, and confirm all third-party binary licenses.

## Feature summary

- Single-image and batch-image watermark encoding.
- Video frame-by-frame encoding and decoding through bundled FFmpeg.
- Independent key, shared batch key, and custom-password key modes.
- PBKDF2-HMAC-SHA256 key derivation and ChaCha20-Poly1305 authenticated encryption.
- DCT mid-frequency watermark embedding, Hamming error correction, and repeated spatial voting.
- Perceptual fingerprint reporting for likely tamper status.
- Unknown-image scan for this project watermark headers and common privacy/AI metadata traces.
- Local-only processing. Media, passwords, and key files are not uploaded.
- Chinese and English UI.
- Windows image right-click menu integration with a single grouped submenu.

## Current validation status

| Platform | Packaging status | Notes |
| --- | --- | --- |
| Windows x64 | Default target | Main development and packaging path. NSIS installer is the default. |
| Windows ARM64 | Prepared but not tested | Directory and manifest structure are present. Requires ARM64 FFmpeg binaries and platform testing. |
| macOS ARM64 | Prepared but not tested | Requires macOS machine, executable permissions on FFmpeg files, signing/notarization decisions, and platform testing. |
| macOS x64 | Prepared but not tested | Requires Intel macOS build environment or suitable runner. Not verified. |
| Linux x64 | Prepared but not tested | Requires Linux Tauri system dependencies and Linux FFmpeg binaries. Not verified. |
| Linux ARM64 | Prepared but not tested | Requires ARM64 Linux build environment and FFmpeg binaries. Not verified. |

Only Windows x64 should be treated as the current verified packaging target. Other platforms have directory conventions and operation notes, but they are not release-certified.

## FFmpeg vendor layout

This repository is designed to keep bundled FFmpeg files under:

```text
src-tauri/vendor/ffmpeg/
├─ LICENSE.txt
├─ README.md
├─ VERSION.txt
├─ manifest.json
├─ windows_x64/
├─ windows_arm64/
├─ macos_arm64/
├─ macos_x64/
├─ linux_x64/
├─ linux_amd64/
└─ linux_arm64/
```

Expected runtime files:

```text
windows_x64/ffmpeg.exe
windows_x64/ffprobe.exe

windows_arm64/ffmpeg.exe
windows_arm64/ffprobe.exe

macos_arm64/ffmpeg
macos_arm64/ffprobe

macos_x64/ffmpeg
macos_x64/ffprobe

linux_x64/ffmpeg
linux_x64/ffprobe

linux_arm64/ffmpeg
linux_arm64/ffprobe
```

`ffplay` is optional. If present, it is listed on the FFmpeg information page but is not required for encoding or decoding.

After copying or replacing binaries, refresh the manifest:

```bash
npm run ffmpeg:manifest
```

For macOS and Linux, make runtime files executable before packaging:

```bash
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
```

Adjust the directory names to the platform you are building.

## Windows x64 build

Install dependencies:

```bash
npm install
```

Refresh FFmpeg manifest:

```bash
npm run ffmpeg:manifest
```

Run development app:

```bash
npm run tauri:dev
```

Build the Windows installer:

```bash
npm run tauri:build
```

The installer is generated under:

```text
target/release/bundle/nsis/
```

The build script renames the final setup file to remove the app version segment from the installer filename.

## Non-Windows build notes

The project includes directory conventions for macOS and Linux, but those packages have not been tested.

General steps:

```bash
npm install
npm run ffmpeg:manifest
cargo test -p watermark-core --release
npm run tauri:build
```

Additional notes:

- macOS builds should be performed on macOS. Signing and notarization are not configured in this project.
- Linux builds require the Tauri WebKitGTK/AppIndicator/librsvg/patchelf dependencies for the target distribution.
- Confirm that `src-tauri/tauri.conf.json` packages the FFmpeg directory for the platform you are building.
- Re-run the full encode/decode/scan workflow on each target OS before publishing installers.

## Git and Git LFS

The FFmpeg vendor tree can exceed normal GitHub file size limits, so this project includes:

```text
.gitattributes
```

with this rule:

```text
src-tauri/vendor/ffmpeg/** filter=lfs diff=lfs merge=lfs -text
```

This tracks the entire FFmpeg vendor directory through Git LFS, including files with no extension.

Use Git LFS before the first commit that adds the FFmpeg binaries:

```bash
git lfs install
git lfs track "src-tauri/vendor/ffmpeg/**"
git add .gitattributes
```

A full upload command sequence is provided in:

```text
GITHUB_UPLOAD_COMMANDS.md
```

## Runtime storage policy

The app avoids using `%APPDATA%` as its own runtime data directory. Runtime data is stored beside the executable:

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

The NSIS installer tries to choose a non-system drive when possible. Uninstall removes `PrivacyWatermarkCodecData` from the install directory.

## Windows right-click menu

The Windows installer registers a grouped image context menu:

```text
Privacy Watermark Codec
├─ 编码隐私水印 / Encode privacy watermark
├─ 检查隐私水印 / Decode and inspect
└─ 无密钥扫描 / Keyless scan
```

The submenu uses the app icon and supports multi-select by launching through Windows Explorer and merging files in the app's single-instance handler.

## How to use

### Encode

1. Select images or videos.
2. Select the output directory.
3. Enter watermark text.
4. Choose a key mode.
5. Start encoding.

### Decode

1. Select encoded media.
2. Select a `.key` file or enter the original custom password.
3. Start decode and inspection.

### Scan unknown images

1. Select one or more unknown images, or use the right-click menu.
2. Start scan. Right-click keyless scan starts automatically after import.
3. Review the trace summary.

A positive project-header detection means an encrypted project watermark is likely present. It does not reveal private text without the key/password. A negative scan result is not proof that the image is watermark-free.

## Key file warning

`.key` files contain derived decryption material. Treat them like passwords. If a key file is exposed, the watermark text protected by that key can be recovered.

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

## Review notes

The current package has been statically reviewed for obvious broken paths, stale documentation references, context-menu `%*` usage, startup FFmpeg probing, and visible command-window FFmpeg calls. Frontend type checking and production web build pass with:

```bash
npm run build
```

Rust compilation and Windows installer generation must still be verified on the target Windows machine after FFmpeg binaries are copied.
