# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           packaging/tests/install.Tests.ps1
# Responsibility:   Pester tests of packaging/install/install.ps1 against a release tree in a.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

# Pester tests of packaging/install/install.ps1 against a release tree in a
# temporary directory. The script runs in a child process, as it does when a
# user starts it, so that its effect on the session is part of what is tested.
# Run on Windows: Invoke-Pester packaging/tests/install.Tests.ps1

BeforeAll {
    $script:Installer = (Resolve-Path (Join-Path $PSScriptRoot '..' 'install' 'install.ps1')).Path
    $script:Shell = (Get-Process -Id $PID).Path
    $arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
        'X64' { 'x86_64' }
        'Arm64' { 'aarch64' }
        default { throw 'unsupported architecture' }
    }
    $script:Target = "$arch-pc-windows-msvc"
    $script:Variables = 'NURT_VERSION', 'NURT_SCOPE', 'NURT_INSTALL_DIR', 'NURT_NO_PATH_UPDATE', 'NURT_RELEASES_URL'

    # New-Release ROOT VERSION: archive with nurt.exe and documentation, and
    # the checksum list, in the layout of GitHub releases.
    function New-Release {
        param([string]$Root, [string]$Version, [switch]$WithoutExecutable, [switch]$WithLatest)
        $stage = Join-Path $Root "stage-$Version"
        $top = Join-Path $stage "nurt-$Version-$script:Target"
        New-Item -ItemType Directory -Force -Path (Join-Path $top 'doc') | Out-Null
        if (-not $WithoutExecutable) { Set-Content -LiteralPath (Join-Path $top 'nurt.exe') -Value "nurt $Version" }
        Set-Content -LiteralPath (Join-Path $top 'NOTICE') -Value 'notice'
        Set-Content -LiteralPath (Join-Path $top 'doc' 'guide.md') -Value 'guide'
        $directory = Join-Path $Root "download/v$Version"
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
        $archive = "nurt-$Version-$script:Target.zip"
        Compress-Archive -Path $top -DestinationPath (Join-Path $directory $archive)
        $digest = (Get-FileHash -LiteralPath (Join-Path $directory $archive) -Algorithm SHA256).Hash.ToLowerInvariant()
        Set-Content -LiteralPath (Join-Path $directory 'checksums.txt') -Value "$digest  $archive"
        if ($WithLatest) {
            $latest = Join-Path $Root 'latest/download'
            New-Item -ItemType Directory -Force -Path $latest | Out-Null
            Copy-Item -LiteralPath (Join-Path $directory 'checksums.txt') -Destination $latest
        }
    }

    # Invoke-Installer ENVIRONMENT [COMMAND]: runs the installer in a child
    # shell with the given variables; COMMAND replaces the plain run.
    function Invoke-Installer {
        param([hashtable]$Environment, [string]$Command)
        $saved = @{}
        foreach ($name in $script:Variables) {
            $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
        foreach ($name in $Environment.Keys) { [Environment]::SetEnvironmentVariable($name, $Environment[$name], 'Process') }
        try {
            if ($Command) {
                $output = & $script:Shell -NoProfile -NonInteractive -Command $Command 2>&1
            }
            else {
                $output = & $script:Shell -NoProfile -NonInteractive -File $script:Installer 2>&1
            }
            [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = ($output | Out-String) }
        }
        finally {
            foreach ($name in $script:Variables) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
        }
    }
}

Describe 'install.ps1' {
    BeforeEach {
        $script:Work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
        $script:Releases = Join-Path $script:Work 'releases'
        $script:Bin = Join-Path $script:Work 'prefix' 'bin'
        New-Item -ItemType Directory -Force -Path $script:Releases | Out-Null
    }

    AfterEach {
        Remove-Item -LiteralPath $script:Work -Recurse -Force -ErrorAction SilentlyContinue
    }

    Context 'installation' {
        It 'installs an exact version with its documentation' {
            New-Release -Root $script:Releases -Version '1.2.3'
            $result = Invoke-Installer @{ NURT_VERSION = 'v1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Be 0 -Because $result.Output
            Get-Content -LiteralPath (Join-Path $script:Bin 'nurt.exe') | Should -Be 'nurt 1.2.3'
            Join-Path (Split-Path -Parent $script:Bin) 'doc' 'NOTICE' | Should -Exist
            Join-Path (Split-Path -Parent $script:Bin) 'doc' 'guide.md' | Should -Exist
            $result.Output | Should -Match 'installed nurt 1\.2\.3'
        }

        It 'resolves the latest release from its checksum list' {
            New-Release -Root $script:Releases -Version '1.2.3'
            New-Release -Root $script:Releases -Version '1.4.0' -WithLatest
            $result = Invoke-Installer @{ NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Be 0 -Because $result.Output
            Get-Content -LiteralPath (Join-Path $script:Bin 'nurt.exe') | Should -Be 'nurt 1.4.0'
        }

        It 'refuses an archive whose digest differs from the checksum list' {
            New-Release -Root $script:Releases -Version '1.2.3'
            Add-Content -LiteralPath (Join-Path $script:Releases 'download/v1.2.3' "nurt-1.2.3-$script:Target.zip") -Value 'tampered'
            $result = Invoke-Installer @{ NURT_VERSION = '1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match 'SHA-256 mismatch'
            Join-Path $script:Bin 'nurt.exe' | Should -Not -Exist
        }

        It 'names the missing archive of a version' {
            New-Release -Root $script:Releases -Version '1.2.3'
            $result = Invoke-Installer @{ NURT_VERSION = '9.9.9'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match '9\.9\.9'
        }

        It 'refuses an archive without nurt.exe' {
            New-Release -Root $script:Releases -Version '1.2.3' -WithoutExecutable
            $result = Invoke-Installer @{ NURT_VERSION = '1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match 'does not contain nurt\.exe'
        }

        It 'rejects a version with path separators' {
            $result = Invoke-Installer @{ NURT_VERSION = '1.2.3\..\x'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match 'invalid version'
        }

        It 'rejects an unknown scope' {
            $result = Invoke-Installer @{ NURT_SCOPE = 'global'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match 'invalid NURT_SCOPE'
        }

        It 'limits the machine scope to Program Files' {
            $result = Invoke-Installer @{ NURT_SCOPE = 'machine'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin }
            $result.ExitCode | Should -Not -Be 0
            $result.Output | Should -Match 'installs only below Program Files|runs as administrator'
        }
    }

    Context 'effect on the session' {
        It 'leaves the preferences of the session unchanged when run through Invoke-Expression' {
            New-Release -Root $script:Releases -Version '1.2.3'
            $command = '$ErrorActionPreference = "Continue"; $ProgressPreference = "Continue"; ' +
                "Invoke-Expression (Get-Content -Raw -LiteralPath '$script:Installer'); " +
                'Write-Output "preferences=$ErrorActionPreference/$ProgressPreference"'
            $result = Invoke-Installer @{ NURT_VERSION = '1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' } $command
            $result.Output | Should -Match 'preferences=Continue/Continue'
        }
    }

    Context 'PATH of the user' {
        BeforeEach {
            $script:Environment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
            $script:SavedPath = $script:Environment.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $script:SavedKind = if ($null -ne $script:SavedPath) { $script:Environment.GetValueKind('Path') } else { $null }
            $script:Environment.SetValue('Path', '%USERPROFILE%\nurt-test-tools;C:\nurt-test', [Microsoft.Win32.RegistryValueKind]::ExpandString)
        }

        AfterEach {
            if ($null -eq $script:SavedPath) { $script:Environment.DeleteValue('Path', $false) }
            else { $script:Environment.SetValue('Path', $script:SavedPath, $script:SavedKind) }
            $script:Environment.Close()
        }

        It 'appends the directory once and keeps unexpanded entries' {
            New-Release -Root $script:Releases -Version '1.2.3'
            $variables = @{ NURT_VERSION = '1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin }
            (Invoke-Installer $variables).ExitCode | Should -Be 0
            (Invoke-Installer $variables).ExitCode | Should -Be 0
            $current = $script:Environment.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $current | Should -Be "%USERPROFILE%\nurt-test-tools;C:\nurt-test;$script:Bin"
            $script:Environment.GetValueKind('Path') | Should -Be ([Microsoft.Win32.RegistryValueKind]::ExpandString)
        }

        It 'leaves the PATH unchanged when asked' {
            New-Release -Root $script:Releases -Version '1.2.3'
            $variables = @{ NURT_VERSION = '1.2.3'; NURT_RELEASES_URL = $script:Releases; NURT_INSTALL_DIR = $script:Bin; NURT_NO_PATH_UPDATE = '1' }
            (Invoke-Installer $variables).ExitCode | Should -Be 0
            $script:Environment.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) |
                Should -Be '%USERPROFILE%\nurt-test-tools;C:\nurt-test'
        }
    }
}

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
