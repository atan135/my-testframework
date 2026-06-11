[CmdletBinding()]
param(
    [string]$TaskName = "QA Register Server",
    [string]$TaskPath = "\",
    [string]$DeployDir = "",
    [switch]$NoStart
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Please run this script from an elevated PowerShell window."
    }
}

Assert-Administrator

if ([string]::IsNullOrWhiteSpace($DeployDir)) {
    $DeployDir = $PSScriptRoot
}

$deployPath = Get-FullPath $DeployDir
$exePath = Join-Path $deployPath "qa-register-rustserver.exe"

if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Server executable not found: $exePath"
}

if (-not $TaskPath.StartsWith("\")) {
    $TaskPath = "\$TaskPath"
}
if (-not $TaskPath.EndsWith("\")) {
    $TaskPath = "$TaskPath\"
}

$existingTask = Get-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName -ErrorAction SilentlyContinue
if ($null -ne $existingTask) {
    Stop-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName -Confirm:$false
}

$action = New-ScheduledTaskAction `
    -Execute $exePath `
    -WorkingDirectory $deployPath

$trigger = New-ScheduledTaskTrigger -AtStartup

$settings = New-ScheduledTaskSettingsSet `
    -StartWhenAvailable `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1)

Register-ScheduledTask `
    -TaskPath $TaskPath `
    -TaskName $TaskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -RunLevel Highest `
    -User "SYSTEM" | Out-Null

if (-not $NoStart) {
    Start-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName
}

Write-Host "Scheduled task is ready."
Write-Host "Task: $TaskPath$TaskName"
Write-Host "Executable: $exePath"
Write-Host "Working directory: $deployPath"
if ($NoStart) {
    Write-Host "Task was registered but not started because -NoStart was provided."
} else {
    Write-Host "Task was started."
}
