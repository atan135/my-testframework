[CmdletBinding()]
param(
    [string]$OutputDir = "",
    [string]$ZipPath = "",
    [switch]$SkipNpmCi
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-PathUnder {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Parent
    )

    $fullPath = Get-FullPath $Path
    $fullParent = (Get-FullPath $Parent).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )

    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    if ($fullPath -eq $fullParent -or $fullPath.StartsWith($fullParent + [System.IO.Path]::DirectorySeparatorChar, $comparison)) {
        return $fullPath
    }

    throw "Refusing to write outside registerserver directory: $fullPath"
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE"
    }
}

$registerServerRoot = Get-FullPath $PSScriptRoot
$clientDir = Join-Path $registerServerRoot "client"
$rustServerDir = Join-Path $registerServerRoot "rustserver"
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $registerServerRoot "release\qa-register"
}
$outputPath = Assert-PathUnder -Path $OutputDir -Parent $registerServerRoot
if ([string]::IsNullOrWhiteSpace($ZipPath)) {
    $ZipPath = "$outputPath.zip"
}
$zipOutputPath = Assert-PathUnder -Path $ZipPath -Parent $registerServerRoot

Write-Host "Register server root: $registerServerRoot"
Write-Host "Release output: $outputPath"
Write-Host "Release zip: $zipOutputPath"

Push-Location $clientDir
try {
    if ($SkipNpmCi) {
        Write-Host "Skipping npm ci."
    } else {
        Write-Host "Installing web dependencies with npm ci..."
        Invoke-NativeCommand -FilePath "npm.cmd" -Arguments @("ci")
    }

    Write-Host "Building web console..."
    Invoke-NativeCommand -FilePath "npm.cmd" -Arguments @("run", "build")
} finally {
    Pop-Location
}

Push-Location $rustServerDir
try {
    Write-Host "Building Rust server release executable..."
    Invoke-NativeCommand -FilePath "cargo.exe" -Arguments @("build", "--release")
} finally {
    Pop-Location
}

$sourceExe = Join-Path $rustServerDir "target\release\qa-register-rustserver.exe"
if (-not (Test-Path -LiteralPath $sourceExe)) {
    throw "Rust release executable not found: $sourceExe"
}

$sourceScheduledTaskScript = Join-Path $registerServerRoot "install-scheduled-task.ps1"
if (-not (Test-Path -LiteralPath $sourceScheduledTaskScript)) {
    throw "Scheduled task installer not found: $sourceScheduledTaskScript"
}

$sourceClientDist = Join-Path $clientDir "dist"
if (-not (Test-Path -LiteralPath (Join-Path $sourceClientDist "index.html"))) {
    throw "Web console dist not found or incomplete: $sourceClientDist"
}

New-Item -ItemType Directory -Force -Path $outputPath | Out-Null

$targetExe = Join-Path $outputPath "qa-register-rustserver.exe"
Copy-Item -LiteralPath $sourceExe -Destination $targetExe -Force
$targetScheduledTaskScript = Join-Path $outputPath "install-scheduled-task.ps1"
Copy-Item -LiteralPath $sourceScheduledTaskScript -Destination $targetScheduledTaskScript -Force

$targetClientDist = Assert-PathUnder -Path (Join-Path $outputPath "client\dist") -Parent $outputPath
if (Test-Path -LiteralPath $targetClientDist) {
    Remove-Item -LiteralPath $targetClientDist -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $targetClientDist | Out-Null
Copy-Item -Path (Join-Path $sourceClientDist "*") -Destination $targetClientDist -Recurse -Force

$zipStageRoot = Assert-PathUnder -Path (Join-Path $registerServerRoot "release\.zip-stage") -Parent $registerServerRoot
$zipStagePackage = Join-Path $zipStageRoot "qa-register"
if (Test-Path -LiteralPath $zipStageRoot) {
    Remove-Item -LiteralPath $zipStageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $zipStagePackage | Out-Null

Copy-Item -LiteralPath $targetExe -Destination (Join-Path $zipStagePackage "qa-register-rustserver.exe") -Force
Copy-Item -LiteralPath $targetScheduledTaskScript -Destination (Join-Path $zipStagePackage "install-scheduled-task.ps1") -Force
$zipStageClientDist = Join-Path $zipStagePackage "client\dist"
New-Item -ItemType Directory -Force -Path $zipStageClientDist | Out-Null
Copy-Item -Path (Join-Path $targetClientDist "*") -Destination $zipStageClientDist -Recurse -Force

if (Test-Path -LiteralPath $zipOutputPath) {
    Remove-Item -LiteralPath $zipOutputPath -Force
}

try {
    Compress-Archive -Path (Join-Path $zipStageRoot "qa-register") -DestinationPath $zipOutputPath -Force
} finally {
    if (Test-Path -LiteralPath $zipStageRoot) {
        Remove-Item -LiteralPath $zipStageRoot -Recurse -Force
    }
}

Write-Host ""
Write-Host "Release package prepared."
Write-Host "Output directory: $outputPath"
Write-Host "Zip file: $zipOutputPath"
Write-Host "Copied: qa-register-rustserver.exe"
Write-Host "Copied: install-scheduled-task.ps1"
Write-Host "Copied: client\dist"
Write-Host ".env was not copied or added to the zip. Create or edit it manually on the target machine."
