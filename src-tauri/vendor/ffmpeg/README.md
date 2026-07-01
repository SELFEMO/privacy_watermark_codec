# FFmpeg vendor directory

Place bundled FFmpeg runtime files in this directory before using video watermark features.

The app requires `ffmpeg` and `ffprobe`. `ffplay` is optional.

Official FFmpeg website:

```text
https://ffmpeg.org/
```

Official FFmpeg download page:

```text
https://ffmpeg.org/download.html
```

The official download page mainly provides source code and links to compiled package providers. You may use files already stored in this repository, download compiled binaries from the providers listed by FFmpeg, or compile FFmpeg yourself. The only requirement for this project is that the final filenames and directories match the layout below.

Required layout:

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

After copying or replacing binaries, run this command from the project root:

```text
npm run ffmpeg:manifest
```

If files are managed by Git LFS but do not appear after clone, try:

```text
git lfs install --force
git lfs fetch origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
git lfs checkout
git lfs pull origin main --include="src-tauri/vendor/ffmpeg/**" --exclude=""
```

`git lfs fetch --all` may download objects into the local LFS cache without replacing pointer files in the working tree. `git lfs checkout` is the command that writes cached LFS objects back to the working tree.
