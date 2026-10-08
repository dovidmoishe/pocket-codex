param(
    [switch]$Mock,
    [switch]$ShowKey,
    [string]$PublicOrigin,
    [string]$CodexHome
)

$ErrorActionPreference = 'Stop'
$taskRepo = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $taskRepo
try {
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    $taskArgs = @('--data', (Join-Path $taskRepo 'data'))
    if ($Mock) { $taskArgs += '--mock' }
    if ($ShowKey) { $taskArgs += '--show-key' }
    if ($PublicOrigin) { $taskArgs += @('--public-origin', $PublicOrigin) }
    if ($CodexHome) { $taskArgs += @('--codex-home', $CodexHome) }
    & (Join-Path $taskRepo 'target\release\pocket-codex.exe') @taskArgs
    if ($LASTEXITCODE -ne 0) { throw 'Pocket Codex exited with an error.' }
}
finally { Pop-Location }

