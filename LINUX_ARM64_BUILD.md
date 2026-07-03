# Linux arm64 packaging from an Ubuntu x64 host

This project can cross-build Linux arm64 deb/rpm packages from Ubuntu x64, but arm64 AppImage packaging must run inside an actual arm64 userspace, such as an arm64 machine, an arm64 CI runner, or an arm64 container/emulator. The split is necessary because the AppImage helper chain executes target-architecture AppImage tools during bundling.

## Cross-build arm64 deb/rpm on Ubuntu x64

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt update
sudo apt install gcc-aarch64-linux-gnu
sudo dpkg --add-architecture arm64
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev:arm64 libssl-dev:arm64
export PKG_CONFIG_SYSROOT_DIR=/usr/aarch64-linux-gnu/
export PKG_CONFIG_ALLOW_CROSS=1

npm install
npm run ffmpeg:manifest
npm run tauri:build:linux:installers:arm64
```

Before packaging, verify that `src-tauri/vendor/ffmpeg/linux_arm64/ffmpeg`, `ffprobe`, and `ffplay` are AArch64 ELF files. The package architecture and the bundled FFmpeg architecture must match.

## Build arm64 deb/rpm/AppImage from an Ubuntu x64 machine

Use an arm64 userspace through an arm64 machine, arm64 CI runner, or QEMU-backed arm64 container, then run the build commands inside that arm64 environment:

```bash
npm install
npm run ffmpeg:manifest
npm run tauri:build:linux:appimage:diagnose:arm64
npm run tauri:build:linux:appimage:prefetch:arm64
npm run tauri:build:linux:all:arm64
```

The expected release package names are:

```text
release/privacy-watermark-codec-linux-arm64.deb
release/privacy-watermark-codec-linux-arm64.rpm
release/privacy-watermark-codec-linux-arm64.AppImage
```
