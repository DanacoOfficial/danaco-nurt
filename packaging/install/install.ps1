# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl

<#
.SYNOPSIS
Installs the nurt executable of a Danaco Nurt release.

.DESCRIPTION
Downloads the release archive for the processor architecture, verifies its
SHA-256 digest against the checksums.txt file of the release, installs
nurt.exe and adds the installation directory to the PATH of the current user.
No administrator privileges are needed.

    irm https://nurt.danaco-group.pl/install.ps1 | iex

Environment variables:
    NURT_VERSION        release version to install (default: the latest release)
    NURT_INSTALL_DIR    installation directory
                        (default: %LOCALAPPDATA%\Programs\Nurt\bin)
    NURT_NO_PATH_UPDATE set to 1 to leave PATH unchanged
    NURT_RELEASES_URL   release location; URL or directory with the layout of
                        GitHub releases (default:
                        https://github.com/DanacoOfficial/nurt/releases)
#>

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Get-NurtFile {
    param([string]$Location, [string]$Destination)
    if ($Location -match '^https?://') {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri $Location -OutFile $Destination
    }
    else {
        Copy-Item -LiteralPath ($Location -replace '^file://', '') -Destination $Destination
    }
}

function Install-Nurt {
    $releasesUrl = if ($env:NURT_RELEASES_URL) { $env:NURT_RELEASES_URL.TrimEnd('/', '\') } else { 'https://github.com/DanacoOfficial/nurt/releases' }
    $version = if ($env:NURT_VERSION) { $env:NURT_VERSION.TrimStart('v') } else { '' }
    $installDir = if ($env:NURT_INSTALL_DIR) { $env:NURT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Nurt\bin' }

    if ($version -and $version -notmatch '^[0-9A-Za-z.+-]+$') {
        throw "invalid version: $version"
    }

    $processor = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    switch ($processor) {
        'X64' { $arch = 'x86_64' }
        'Arm64' { $arch = 'aarch64' }
        default { throw "unsupported architecture: $processor" }
    }
    $target = "$arch-pc-windows-msvc"

    $work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        if (-not $version) {
            $latest = Join-Path $work 'latest.txt'
            Get-NurtFile "$releasesUrl/latest/download/checksums.txt" $latest
            $pattern = '^[0-9A-Fa-f]{64}\s+\*?nurt-(?<version>.+)-' + [regex]::Escape($target) + '\.zip$'
            $match = Get-Content -LiteralPath $latest | ForEach-Object { [regex]::Match($_, $pattern) } | Where-Object { $_.Success } | Select-Object -First 1
            if (-not $match) { throw "the latest release has no archive for $target" }
            $version = $match.Groups['version'].Value
        }

        $archive = "nurt-$version-$target.zip"
        $downloadUrl = "$releasesUrl/download/v$version"

        $checksums = Join-Path $work 'checksums.txt'
        Get-NurtFile "$downloadUrl/checksums.txt" $checksums
        $entry = Get-Content -LiteralPath $checksums | ForEach-Object { [regex]::Match($_, '^(?<digest>[0-9A-Fa-f]{64})\s+\*?(?<name>\S+)$') } | Where-Object { $_.Success -and $_.Groups['name'].Value -eq $archive } | Select-Object -First 1
        if (-not $entry) { throw "version $version has no archive for $target" }
        $expected = $entry.Groups['digest'].Value.ToLowerInvariant()

        $archivePath = Join-Path $work $archive
        Get-NurtFile "$downloadUrl/$archive" $archivePath
        $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw "SHA-256 mismatch for ${archive}: expected $expected, found $actual" }

        $extract = Join-Path $work 'extract'
        Expand-Archive -LiteralPath $archivePath -DestinationPath $extract
        $binary = Join-Path $extract 'nurt.exe'
        if (-not (Test-Path -LiteralPath $binary)) { throw "$archive does not contain nurt.exe" }

        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        Copy-Item -LiteralPath $binary -Destination (Join-Path $installDir 'nurt.exe') -Force
    }
    finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($env:NURT_NO_PATH_UPDATE -ne '1') {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = if ($userPath) { $userPath.Split(';') } else { @() }
        if ($entries -notcontains $installDir) {
            $updated = if ($userPath) { "$userPath;$installDir" } else { $installDir }
            [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
            Write-Host "added $installDir to the user PATH; open a new terminal to use nurt"
        }
    }
    Write-Host "installed nurt $version to $(Join-Path $installDir 'nurt.exe')"
}

Install-Nurt
