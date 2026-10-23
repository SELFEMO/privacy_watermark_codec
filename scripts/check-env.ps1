$commands = @("node", "npm", "rustc", "cargo", "ffmpeg", "ffprobe")
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$cargoExe = Join-Path $cargoBin "cargo.exe"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path -LiteralPath $cargoExe)) {
    # Cursor 终端经常继承不到 rustup 的用户 PATH；临时前置后才能如实报告本机已安装的 cargo。
    # Cursor terminals often miss rustup's user PATH; prepending it here reports the installed cargo instead of a false missing status.
    $env:Path = "$cargoBin;$env:Path"
    Write-Host "[dev] Prepended $cargoBin to PATH (cargo was missing from this process)."
}
foreach ($command in $commands) {
    $resolved = Get-Command $command -ErrorAction SilentlyContinue
    if ($resolved) {
        Write-Host "[OK] $command -> $($resolved.Source)" -ForegroundColor Green
    } else {
        Write-Host "[MISSING] $command" -ForegroundColor Red
    }
}
