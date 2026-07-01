$commands = @("node", "npm", "rustc", "cargo", "ffmpeg", "ffprobe")
foreach ($command in $commands) {
    $resolved = Get-Command $command -ErrorAction SilentlyContinue
    if ($resolved) {
        Write-Host "[OK] $command -> $($resolved.Source)" -ForegroundColor Green
    } else {
        Write-Host "[MISSING] $command" -ForegroundColor Red
    }
}
