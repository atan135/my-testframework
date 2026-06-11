[CmdletBinding()]
param(
    [string]$Remote = "origin",
    [switch]$RequireClean,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot

$repositories = @(
    @{
        Name = "qatestframework"
        Path = $root
    }
)

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    if ($DryRun) {
        Write-Host "DRY RUN: git $($Arguments -join ' ')"
        return $null
    }

    & git @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

function Get-GitOutput {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $output = & git @Arguments 2>$null
    if ($LASTEXITCODE -ne 0) {
        return $null
    }

    return $output
}

foreach ($repo in $repositories) {
    Write-Host ""
    Write-Host "==> $($repo.Name)"

    if (-not (Test-Path -LiteralPath $repo.Path -PathType Container)) {
        throw "Repository path not found: $($repo.Path)"
    }

    Push-Location -LiteralPath $repo.Path
    try {
        $insideWorkTree = Get-GitOutput @("rev-parse", "--is-inside-work-tree")
        if ($insideWorkTree -ne "true") {
            throw "Not a git work tree: $($repo.Path)"
        }

        $branch = Get-GitOutput @("branch", "--show-current")
        if ([string]::IsNullOrWhiteSpace($branch)) {
            throw "Repository is not on a branch: $($repo.Path)"
        }

        $remoteUrl = Get-GitOutput @("remote", "get-url", $Remote)
        if ([string]::IsNullOrWhiteSpace($remoteUrl)) {
            if ($DryRun) {
                Write-Warning "Remote '$Remote' is not configured for $($repo.Name)."
                continue
            }
            throw "Remote '$Remote' is not configured for $($repo.Name)"
        }

        $dirty = Get-GitOutput @("status", "--porcelain")
        if ($dirty) {
            Write-Warning "$($repo.Name) has uncommitted changes. git push will only push committed changes."
            if ($RequireClean) {
                throw "$($repo.Name) is not clean. Commit or stash changes first."
            }
        }

        $upstream = Get-GitOutput @("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}")
        if ([string]::IsNullOrWhiteSpace($upstream)) {
            Write-Host "Pushing $($repo.Name): $branch -> $Remote/$branch"
            Invoke-Git @("push", "-u", $Remote, $branch)
        }
        else {
            Write-Host "Pushing $($repo.Name): $branch -> $upstream"
            Invoke-Git @("push")
        }
    }
    finally {
        Pop-Location
    }
}

Write-Host ""
Write-Host "Repository processed."
