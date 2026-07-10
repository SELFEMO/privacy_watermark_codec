# Privacy Watermark Codec

[English](README.md) | [简体中文](README.zh-CN.md)

Privacy Watermark Codec is a local-first desktop app for invisible privacy watermarking. It embeds encrypted watermark text into images or videos, extracts the watermark with the matching key file or custom password, and reports likely tampering with perceptual fingerprints.

> The app is built with Tauri, Vue, and Rust. Media files, passwords, and key files stay on the local machine.

## AI development notice

This project was implemented with AI-assisted programming. Before public release or production use, review the code, test the target platforms, verify the watermark behavior with your own sample set, and confirm all third-party binary licenses.

## What it does

- Encodes a single image, a batch of images, or a video.
- Decodes watermarked images and videos.
- Supports independent keys, one shared batch key, and custom-password mode.
- Uses PBKDF2-HMAC-SHA256 for key derivation and ChaCha20-Poly1305 for encrypted payloads.
- Uses DCT-domain embedding, BCH error correction, synchronization-assisted registration, and repeated voting to improve recovery robustness.
- Reports global tamper status and suspicious regions from perceptual fingerprints.
- Scans unknown images for this project watermark header and common privacy or AI metadata traces.
- Provides Chinese and English UI text.
- Adds grouped Windows image context-menu actions when installed through the Windows installer.

## Repository layout

```text
privacy-watermark-codec/
├─ src/                         Vue frontend
├─ src/components/              UI components and brand mark
├─ src-tauri/                   Tauri shell, commands, packaging config, video orchestration
├─ src-tauri/icons/             App icons
├─ src-tauri/linux/             Linux desktop entries, package hooks, and AppImage helpers
├─ src-tauri/vendor/ffmpeg/     Bundled FFmpeg files and generated manifest
├─ src-tauri/windows/           NSIS installer hooks
├─ crates/watermark-core/       Rust watermark, crypto, key, scan, and tamper core
├─ scripts/                     Development, release, FFmpeg, and AppImage helper scripts
└─ .github/workflows/           Release workflow
```

## Prerequisites

Install Node.js, Rust, and the Linux/macOS/Windows native dependencies required by Tauri on your platform.

On Linux hosts, AppImage packaging also needs these commands:

```Shell
sudo apt install patchelf binutils file
```

## Clone and restore large FFmpeg files

Recommended clone flow:

```Shell
git clone https://github.com/SELFEMO/privacy_watermark_codec.git
cd privacy_watermark_codec
```

If FFmpeg binaries are stored through Git LFS, restore them after cloning:

```Shell
git lfs install
git lfs pull
```

If `git lfs` is unavailable on Ubuntu or Debian:

```Shell
sudo apt update
sudo apt install -y git-lfs
git lfs install
git lfs pull
```

A GitHub source ZIP may not include Git LFS binary objects. Use a full clone or a release source package when you need a build-ready tree.

## FFmpeg assets

Video features require `ffmpeg` and `ffprobe`. The project reads bundled files from:

```text
src-tauri/vendor/ffmpeg/
```

Minimum expected layout:

```text
src-tauri/vendor/ffmpeg/windows_x64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_x64/ffprobe.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffmpeg.exe
src-tauri/vendor/ffmpeg/windows_arm64/ffprobe.exe

src-tauri/vendor/ffmpeg/macos_x64/ffmpeg
src-tauri/vendor/ffmpeg/macos_x64/ffprobe
src-tauri/vendor/ffmpeg/macos_amd64/ffmpeg
src-tauri/vendor/ffmpeg/macos_amd64/ffprobe
src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg
src-tauri/vendor/ffmpeg/macos_arm64/ffprobe

src-tauri/vendor/ffmpeg/linux_x64/ffmpeg
src-tauri/vendor/ffmpeg/linux_x64/ffprobe
src-tauri/vendor/ffmpeg/linux_amd64/ffmpeg
src-tauri/vendor/ffmpeg/linux_amd64/ffprobe
src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg
src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
```

> `ffplay` can be present but is not required for watermark encoding or decoding.

After adding or replacing FFmpeg files, refresh the manifest:

```Shell
npm run ffmpeg:manifest
```

On macOS and Linux, ensure runtime files are executable:

```Shell
chmod +x src-tauri/vendor/ffmpeg/linux_x64/ffmpeg src-tauri/vendor/ffmpeg/linux_x64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg src-tauri/vendor/ffmpeg/linux_arm64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/macos_x64/ffmpeg src-tauri/vendor/ffmpeg/macos_x64/ffprobe
chmod +x src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg src-tauri/vendor/ffmpeg/macos_arm64/ffprobe
```

## Development

Install frontend dependencies:

```Shell
npm install
```

Start the desktop app in development mode:

```Shell
npm run tauri:dev
```

Linux development mode temporarily writes user-level desktop entries and hicolor icons before the Tauri window starts. This helps GNOME/Ubuntu Dock match the debug window to the real app icon. The wrapper removes those temporary files on normal exit.

If the terminal is killed or stale development icons remain, run either cleanup command from the project root:

```Shell
npm run tauri:dev:cleanup
npm run linux:desktop:cleanup
```

Both cleanup commands call the same script. They only remove project-managed temporary development entries and refresh desktop/icon caches; they do not uninstall `.deb` or `.rpm` packages.

## Packaging commands

Build the default bundle for the current platform:

```Shell
npm run tauri:build
```

### Windows

```Shell
npm run tauri:build:windows
npm run tauri:build:windows:nsis:x64
npm run tauri:build:windows:nsis:arm64
npm run tauri:build:windows:msi:x64
npm run tauri:build:windows:msi:arm64
npm run tauri:build:windows:all:x64
npm run tauri:build:windows:all:arm64
```

The short `windows` command builds the NSIS installer for the current Windows host architecture. The `all` commands build both NSIS and MSI.

### macOS

```Shell
npm run tauri:build:macos
npm run tauri:build:macos:x64
npm run tauri:build:macos:arm64
npm run tauri:build:macos:dmg
npm run tauri:build:macos:x64:dmg
npm run tauri:build:macos:arm64:dmg
```

`macos` builds a runnable app bundle. The `dmg` commands also try to create a disk image.

### Linux stable installers

For Ubuntu, Debian, Fedora, openSUSE, and other package-manager flows, prefer DEB/RPM installers:

```Shell
npm run tauri:build:linux:installers:x64
npm run tauri:build:linux:deb:x64
npm run tauri:build:linux:rpm:x64
```

ARM64 aliases are also available:

```Shell
npm run tauri:build:linux:installers:arm64
npm run tauri:build:linux:deb:arm64
npm run tauri:build:linux:rpm:arm64
```

Install a generated DEB from the project root:

```Shell
sudo apt install ./release/privacy-watermark-codec-linux-x64.deb
```

If your shell is already inside `release/`, use:

```Shell
sudo apt install ./privacy-watermark-codec-linux-x64.deb
```

### Linux AppImage, optional

AppImage is kept as an optional single-file Linux distribution format. It is useful when users want to download one executable file and run it without installing a package, but it has higher maintenance cost because it depends on linuxdeploy, AppImage helper downloads, WebKitGTK/GStreamer dependency collection, and local ELF patching tools.

Normal AppImage flow:

```Shell
npm run tauri:build:linux:appimage:diagnose
npm run tauri:build:linux:appimage:prefetch
npm run tauri:build:linux:appimage:x64
```

Run the generated AppImage:

```Shell
chmod u+x ./release/privacy-watermark-codec-linux-x64.AppImage
./release/privacy-watermark-codec-linux-x64.AppImage
```

Verbose AppImage retry for diagnosis only:

```Shell
PWC_APPIMAGE_VERBOSE=1 npm run tauri:build:linux:appimage:x64
```

If GitHub release-asset downloads are slow or blocked, the build script first caches required helper binaries under `target/.tauri/pwc-appimage-tools`, caches raw linuxdeploy helper scripts under `target/.tauri`, and serves release helpers to Tauri through a temporary local mirror. You can also provide a mirror template:

```text
PWC_APPIMAGE_TOOLS_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset> npm run tauri:build:linux:appimage:prefetch
TAURI_BUNDLER_TOOLS_GITHUB_MIRROR_TEMPLATE=https://mirror.example.com/<owner>/<repo>/releases/download/<version>/<asset> npm run tauri:build:linux:appimage:x64
```

During AppImage packaging, Linux FFmpeg binaries are temporarily encoded as `.pwcbin` non-ELF resources so linuxdeploy does not run `patchelf` on the large prebuilt FFmpeg executables. The build script restores the source tree after packaging. At runtime, the Rust backend restores `.pwcbin` resources into the user data cache and verifies them against the manifest hashes before use. AppImage builds set `LDAI_NO_APPSTREAM=1` because appimagetool can treat optional AppStream warnings as fatal; DEB/RPM packages still install the system metainfo file.

### Linux all-in-one command

```Shell
npm run tauri:build:linux:x64:all
```

This builds DEB and RPM first, then attempts AppImage. If AppImage fails after DEB/RPM are already produced, the script keeps the successful installer packages in `release/` and reports AppImage as optional.

## Release output

The release script copies distributable artifacts to:

```text
release/
```

Canonical names:

```text
privacy-watermark-codec-windows-x64.exe
privacy-watermark-codec-windows-x64.msi
privacy-watermark-codec-macos-x64.app
privacy-watermark-codec-macos-x64.dmg
privacy-watermark-codec-linux-x64.deb
privacy-watermark-codec-linux-x64.rpm
privacy-watermark-codec-linux-x64.AppImage
```

Tauri's original bundle output remains under `target/**/release/bundle/`.

## Runtime storage policy

Portable builds and development mode use a writable directory beside the executable when possible:

```text
PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

Linux DEB/RPM packages are normally installed under system locations that ordinary users cannot write to. In that case the app falls back to user data storage, for example:

```text
~/.local/share/privacy-watermark-codec/PrivacyWatermarkCodecData/
├─ webview-data/
└─ work/
```

This fallback prevents installed Linux packages from exiting before the WebView window is created.

## App usage

### Encode

1. Open encode mode.
2. Select image or video files.
3. Choose an output directory.
4. Enter watermark text.
5. Choose independent key, shared batch key, or custom password.
6. Start encoding.
7. Keep the output media, generated `.key` file, and evidence manifest.

### Decode

1. Open decode mode.
2. Select encoded images or videos.
3. Select the `.key` file or enter the original custom password.
4. Start decoding and inspection.
5. Review extracted text, tamper status, and suspicious regions.

### Scan

1. Select unknown images, or import them from the Windows right-click menu.
2. Start scan.
3. Review whether a project watermark header, privacy metadata, or AI watermark traces are found.

A positive project-header scan means an encrypted project watermark may be present. It does not reveal watermark text without the key or password. A negative scan result is not proof that the image is watermark-free.

## Current validation status

| Platform                   | Current status                                 | Notes                                                                                                                                                                       |
| -------------------------- | ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows x64                | Verified                                       | Encoding, decoding, packaging, and NSIS installer flow have passed in earlier validation.                                                                                   |
| Windows ARM64              | Supported, pending device review               | Scripts and FFmpeg directory names are present; test on real hardware before release.                                                                                       |
| macOS ARM64                | Verified                                       | Apple Silicon build flow and local runtime were validated earlier.                                                                                                          |
| macOS x64                  | Supported                                      | `macos_x64` is the preferred Intel Mac folder; `macos_amd64` remains as a compatibility alias.                                                                          |
| Linux x64 / amd64 DEB      | Verified                                       | Package installs and launches; runtime data falls back to the user data directory when system install paths are not writable.                                               |
| Linux x64 / amd64 RPM      | Verified through local conversion/install flow | Package launches after install; distribution-native RPM testing is still recommended.                                                                                       |
| Linux x64 / amd64 AppImage | Optional                                       | Previous failures reached appimagetool and failed on optional AppStream validation. The AppImage build now sets`LDAI_NO_APPSTREAM=1`; DEB/RPM still keep system metadata. |
| Linux ARM64                | Supported, pending device review               | Scripts and FFmpeg directory names are present; test with ARM64 FFmpeg binaries on target hardware.                                                                         |

## Security notes

- `.key` files contain derived decryption material. Treat them like password files.
- Custom-password mode depends on the strength and secrecy of the password.
- FFmpeg binaries are checked against the generated manifest before video processing, so replacing bundled FFmpeg files requires regenerating the manifest before release.

## Troubleshooting

### AppImage reports missing tools

```Shell
npm run tauri:build:linux:appimage:diagnose
sudo apt install patchelf binutils file
```

### AppImage fails during AppStream validation

If `pwc-appimage-build.log` contains `Failed to validate AppStream information with appstreamcli`, rebuild with this updated tree. The AppImage build now sets `LDAI_NO_APPSTREAM=1`, the environment variable supported by `linuxdeploy-plugin-appimage`, because AppImage metadata warnings can stop `appimagetool`; DEB/RPM packages still include system metainfo.

### AppImage output is too long

Do not use `PWC_APPIMAGE_VERBOSE=1` for normal builds. Use it only when AppImage fails and you need a detailed `target/**/release/pwc-appimage-build.log`.

### Linux Dock still shows an old icon

Unpin the old icon, run the cleanup command for development entries, and restart the app:

```Shell
npm run linux:desktop:cleanup
```

If a system package was installed, refresh desktop caches or log out and back in.
