$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$distDir = Join-Path $repoRoot "dist"

Push-Location $repoRoot
try {
    cargo build --release

    New-Item -ItemType Directory -Path $distDir -Force | Out-Null

    $windowsExe = Join-Path $repoRoot "target\release\qamcp.exe"
    $unixExe = Join-Path $repoRoot "target\release\qamcp"
    if (Test-Path -LiteralPath $windowsExe) {
        $source = $windowsExe
        $target = Join-Path $distDir "qamcp.exe"
    } elseif (Test-Path -LiteralPath $unixExe) {
        $source = $unixExe
        $target = Join-Path $distDir "qamcp"
    } else {
        throw "Cargo release binary was not found under target/release."
    }

    Copy-Item -LiteralPath $source -Destination $target -Force
    Write-Host "Built $target"
} finally {
    Pop-Location
}
