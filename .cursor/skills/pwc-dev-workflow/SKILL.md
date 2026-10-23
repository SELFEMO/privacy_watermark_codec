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
npm run clean; npm install --registry=https://registry.npmmirror.com; npm run desktop:dev
```

## Build

```
npm run clean; npm install --registry=https://registry.npmmirror.com; npm run desktop:build
```

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
