---
name: pwc-dev-workflow
description: >-
  Privacy Watermark Codec desktop run/build workflow. Use after code changes in
  this repo, when launching the Tauri desktop app, when the user says run,
  desktop:dev, desktop:build, or build, and when committing (only if the user
  asked). Windows PowerShell; npmmirror registry; port 1420.
---

# PWC desktop workflow

## Run

```
npm run desktop:dev:clean
```

## Build

```
npm run desktop:build:clean
```

Both entry points support Windows CMD, Windows PowerShell, and PowerShell without command separators. If PowerShell's execution policy blocks `npm.ps1`, use `npm.cmd run desktop:build:clean` or `npm.cmd run desktop:dev:clean`; changing the system execution policy is unnecessary. They run full `clean`, install dependencies with `--registry=https://registry.npmmirror.com`, and then call the existing desktop entry point in order. Any failure stops the remaining steps and preserves the failed step's exit code.

Full cleanup removes frontend `dist`/Vite caches, Cargo `target` directories, Tauri temporary directories, and `release`; it preserves `node_modules` and bundled FFmpeg assets. The existing `desktop:dev` and `desktop:build` entry points still preserve incremental caches.

Use `npm run desktop:build:clean -- --dry-run` or `npm run desktop:dev:clean -- --dry-run` to preview without cleaning, installing, or launching. Build arguments are forwarded to the existing build entry point, for example `npm run desktop:build:clean -- --arch x64`. The workflow consumes its own `--dry-run` option; an additional `--` passed to the workflow marks the start of arguments to forward verbatim.

## When to launch

After code changes, auto-run the **run** command unless the user asked for **build** or said not to launch.

If the user asked to build, use the **build** command instead of **run**.

Cursor terminals may not inherit rustup's user PATH (`%USERPROFILE%\.cargo\bin`). `desktop:dev` and `desktop:build` prepend that directory when `cargo` is missing from the current process; do not treat a stale Cursor PATH as a missing toolchain.

## Before launch

Stop any existing `desktop:dev`, Vite, or Tauri process listening on port **1420**, then run the command.

PowerShell:

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue |
  ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
Get-CimInstance Win32_Process |
  Where-Object {
    $_.CommandLine -match 'desktop:dev|tauri-dev|vite|tauri .*dev' -and
    $_.Name -match 'node|cargo|privacy-watermark'
  } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
```

## Git

Git only when the user asks. Do not commit, push, or create PRs otherwise.

On this Windows/PowerShell repo, pass the commit message with a here-string:

```powershell
git commit -m @"
Commit message here.

"@
```

Do not use bash `$(cat <<'EOF'` on PowerShell.
