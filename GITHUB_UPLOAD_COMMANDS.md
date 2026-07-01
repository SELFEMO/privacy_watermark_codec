# GitHub upload commands

Repository URL supplied by the user:

```text
https://github.com/SELFEMO/privacy_watermark_codec.git
```

Run the commands from the project root.

## First upload

```powershell
cd D:\MyWorkstation\Learning\Rust\privacy_watermark_codec

git init
git lfs install
git lfs track "src-tauri/vendor/ffmpeg/**"

git add .gitattributes
git add .gitignore
git add README.md README.zh-CN.md GITHUB_UPLOAD_COMMANDS.md
git add package.json package-lock.json .npmrc tsconfig.json vite.config.ts index.html Cargo.toml
git add src src-tauri crates scripts .github

git status
git commit -m "Initial import of privacy watermark codec"

git branch -M main
git remote add origin https://github.com/SELFEMO/privacy_watermark_codec.git
git push -u origin main
```

## If the remote already exists locally

```powershell
git remote set-url origin https://github.com/SELFEMO/privacy_watermark_codec.git
git push -u origin main
```

## Verify that FFmpeg files are tracked by LFS

```powershell
git lfs ls-files
git check-attr -a -- src-tauri/vendor/ffmpeg/windows_x64/ffmpeg.exe
git check-attr -a -- src-tauri/vendor/ffmpeg/macos_arm64/ffmpeg
```

## Push all LFS objects again if GitHub reports missing LFS objects

```powershell
git lfs push --all origin main
git push
```

## If large FFmpeg files were committed before enabling LFS

Only use this when the first push failed because large binaries were already committed as regular Git objects.

```powershell
git lfs migrate import --include="src-tauri/vendor/ffmpeg/**"
git push --force-with-lease origin main
```

## Suggested commit messages after future changes

```powershell
git add .
git commit -m "Fix Windows context menu import"
git push

git add src-tauri/vendor/ffmpeg
git add src-tauri/vendor/ffmpeg/manifest.json
git commit -m "Update bundled FFmpeg binaries"
git push
```

## Files intentionally not uploaded

`docs/`, `node_modules/`, `dist/`, `target/`, logs, temporary files, and editor settings are ignored or removed from this project package.

# Routine code update

```powershell
git add .
git commit -m "Update project"
git push
```
