# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/install/install.ps1
# Responsibility:   Script install.ps1.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

<#
.SYNOPSIS
Installs the nurt executable of a Danaco Nurt release.

.DESCRIPTION
Downloads the release archive for the processor architecture, verifies its
SHA-256 digest against the checksums.txt file of the release, installs
nurt.exe and the documentation and adds the installation directory to PATH.

The default scope is the current user: no administrator privileges are needed
and the files go to %LOCALAPPDATA%\Programs\Nurt. The scope "machine" installs
for all users into %ProgramFiles%\Nurt, changes the PATH of the machine and
needs a terminal that runs as administrator. nurt.exe runs with the rights of
the terminal that starts it, so operations that need administrator rights
(services, the machine part of the registry, system directories) are run from
a terminal started with "Run as administrator".

    irm https://nurt.danaco-group.pl/install.ps1 | iex

    $env:NURT_SCOPE = 'machine'; irm https://nurt.danaco-group.pl/install.ps1 | iex

Environment variables:
    NURT_VERSION        release version to install (default: the latest release)
    NURT_SCOPE          user (default) or machine
    NURT_INSTALL_DIR    installation directory of nurt.exe
                        (default: %LOCALAPPDATA%\Programs\Nurt\bin, or
                        %ProgramFiles%\Nurt\bin for the scope machine); when
                        its name is bin, the documentation goes to the doc
                        directory next to it
    NURT_NO_PATH_UPDATE set to 1 to leave PATH unchanged
    NURT_RELEASES_URL   release location; URL or directory with the layout of
                        GitHub releases (default:
                        https://github.com/DanacoOfficial/danaco-nurt/releases)
#>

function Get-NurtFile {
    param([string]$Location, [string]$Destination)
    if ($Location -match '^https?://') {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri $Location -OutFile $Destination
    }
    else {
        # file:///C:/dir is the path C:/dir; file:///dir is /dir.
        $localPath = $Location -replace '^file:///(?=[A-Za-z]:)', '' -replace '^file://', ''
        Copy-Item -LiteralPath $localPath -Destination $Destination
    }
}

function Test-NurtAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    return ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Install-Nurt {
    # Set here and not at the top of the file: when the script runs through
    # Invoke-Expression its top level is the caller's scope, and the values
    # would stay set in the session of the user.
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $scope = if ($env:NURT_SCOPE) { $env:NURT_SCOPE.ToLowerInvariant() } else { 'user' }
    if ($scope -notin 'user', 'machine') {
        throw "invalid NURT_SCOPE: $scope; use user or machine"
    }
    if ($scope -eq 'machine' -and -not (Test-NurtAdministrator)) {
        throw 'NURT_SCOPE=machine needs a terminal that runs as administrator: start PowerShell with "Run as administrator"'
    }
    $releasesUrl = if ($env:NURT_RELEASES_URL) { $env:NURT_RELEASES_URL.TrimEnd('/', '\') } else { 'https://github.com/DanacoOfficial/danaco-nurt/releases' }
    $version = if ($env:NURT_VERSION) { $env:NURT_VERSION.TrimStart('v') } else { '' }
    $defaultRoot = if ($scope -eq 'machine') { Join-Path $env:ProgramFiles 'Nurt' } else { Join-Path $env:LOCALAPPDATA 'Programs\Nurt' }
    $installDir = if ($env:NURT_INSTALL_DIR) { $env:NURT_INSTALL_DIR } else { Join-Path $defaultRoot 'bin' }

    if ($scope -eq 'machine') {
        # The directory is added to the PATH of every account; it must not be
        # one that a standard user can write to.
        $programRoots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramW6432) | Where-Object { $_ }
        $fullDir = [IO.Path]::GetFullPath($installDir).TrimEnd('\')
        $underProgramFiles = $programRoots | Where-Object { $fullDir -eq $_.TrimEnd('\') -or $fullDir.StartsWith($_.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) }
        if (-not $underProgramFiles) {
            throw "NURT_SCOPE=machine installs only below Program Files, not to $installDir; use NURT_SCOPE=user or a directory below Program Files"
        }
    }

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
            if ($version -notmatch '^[0-9A-Za-z.+-]+$') { throw "invalid version in the latest release: $version" }
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
        $binary = Get-ChildItem -LiteralPath $extract -Filter 'nurt.exe' -File -Recurse | Select-Object -First 1
        if (-not $binary) { throw "$archive does not contain nurt.exe" }

        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        Copy-Item -LiteralPath $binary.FullName -Destination (Join-Path $installDir 'nurt.exe') -Force

        if ((Split-Path -Leaf $installDir) -eq 'bin') {
            $docDir = Join-Path (Split-Path -Parent $installDir) 'doc'
            New-Item -ItemType Directory -Force -Path $docDir | Out-Null
            foreach ($item in Get-ChildItem -LiteralPath $binary.Directory.FullName -Force | Where-Object { $_.Name -ne 'nurt.exe' }) {
                if ($item.PSIsContainer -and $item.Name -eq 'doc') {
                    Copy-Item -Path (Join-Path $item.FullName '*') -Destination $docDir -Recurse -Force
                }
                else {
                    Copy-Item -LiteralPath $item.FullName -Destination $docDir -Recurse -Force
                }
            }
        }
    }
    finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($env:NURT_NO_PATH_UPDATE -ne '1') {
        $pathScope = if ($scope -eq 'machine') { 'Machine' } else { 'User' }
        # The value is read and written unexpanded and as REG_EXPAND_SZ, so
        # that entries such as %USERPROFILE%\bin keep their form.
        $key = if ($scope -eq 'machine') {
            [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment', $true)
        }
        else {
            [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        }
        $current = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $entries = if ($current) { $current.Split(';') } else { @() }
        if ($entries -notcontains $installDir) {
            $updated = if ($current) { "$current;$installDir" } else { $installDir }
            $key.SetValue('Path', $updated, [Microsoft.Win32.RegistryValueKind]::ExpandString)
            $key.Close()
            # Deleting a variable that does not exist makes the system
            # announce the change of the environment to running programs.
            [Environment]::SetEnvironmentVariable('NURT_PATH_CHANGED', $null, $pathScope)
            Write-Host "added $installDir to the $($pathScope.ToLowerInvariant()) PATH; open a new terminal to use nurt"
        }
    }
    Write-Host "installed nurt $version to $(Join-Path $installDir 'nurt.exe')"
}

Install-Nurt

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
