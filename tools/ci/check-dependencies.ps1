# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
<#
.SYNOPSIS
Checks that a Windows executable or DLL imports only DLLs of the system.

.DESCRIPTION
Reads the imported DLLs with `dumpbin /dependents` (the tool of Visual Studio)
or, when dumpbin is not on the path, with `objdump -p`. Every DLL must belong
to Windows. The runtime of Visual C++ (VCRUNTIME*, MSVCP*, CONCRT*, UCRTBASE and
the forwarders api-ms-win-crt-*) is not part of the system for this check,
because it is linked into the file statically.

The exit code is 0 when every file passes and 1 otherwise.

.PARAMETER Path
Files to check.

.EXAMPLE
pwsh tools/ci/check-dependencies.ps1 target\release\nurt.exe target\release\nurt.dll
#>
param(
    [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
    [string[]] $Path
)

$ErrorActionPreference = 'Stop'

$denied = @('vcruntime*', 'msvcp*', 'concrt*', 'vcomp*', 'ucrtbase*', 'msvcr*', 'api-ms-win-crt-*')
$allowed = @(
    'kernel32.dll', 'kernelbase.dll', 'ntdll.dll', 'advapi32.dll', 'user32.dll', 'gdi32.dll',
    'shell32.dll', 'ole32.dll', 'oleaut32.dll', 'ws2_32.dll', 'iphlpapi.dll', 'bcrypt.dll',
    'bcryptprimitives.dll', 'crypt32.dll', 'secur32.dll', 'userenv.dll', 'netapi32.dll',
    'psapi.dll', 'pdh.dll', 'powrprof.dll', 'wevtapi.dll', 'rstrtmgr.dll', 'shlwapi.dll',
    'comdlg32.dll', 'comctl32.dll', 'wtsapi32.dll', 'setupapi.dll', 'cfgmgr32.dll',
    'winhttp.dll', 'wininet.dll', 'mswsock.dll', 'dnsapi.dll', 'version.dll', 'imm32.dll',
    'winmm.dll', 'ncrypt.dll', 'sspicli.dll', 'cryptbase.dll', 'wldap32.dll', 'api-ms-win-core-*'
)

function Get-Imports([string] $File) {
    if (Get-Command dumpbin -ErrorAction SilentlyContinue) {
        $text = & dumpbin /dependents $File 2>&1 | Out-String
        $names = [regex]::Matches($text, '(?m)^\s+(\S+\.dll)\s*$') | ForEach-Object { $_.Groups[1].Value }
    } else {
        $tool = if (Get-Command llvm-objdump -ErrorAction SilentlyContinue) { 'llvm-objdump' } else { 'objdump' }
        $text = & $tool -p $File 2>&1 | Out-String
        $names = [regex]::Matches($text, '(?m)^\s*DLL Name:\s*(\S+)') | ForEach-Object { $_.Groups[1].Value }
    }
    $names | ForEach-Object { $_.ToLowerInvariant() }
}

function Test-Allowed([string] $Name) {
    foreach ($pattern in $denied) {
        if ($Name -like $pattern) { return $false }
    }
    foreach ($pattern in $allowed) {
        if ($Name -like $pattern) { return $true }
    }
    return $false
}

$failed = $false
foreach ($file in $Path) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        [Console]::Error.WriteLine("check-dependencies.ps1: $file does not exist")
        $failed = $true
        continue
    }
    $imports = @(Get-Imports $file)
    if ($imports.Count -eq 0) {
        [Console]::Error.WriteLine("${file}: no import table could be read")
        $failed = $true
        continue
    }
    $bad = @($imports | Where-Object { -not (Test-Allowed $_) })
    if ($bad.Count -gt 0) {
        [Console]::Error.WriteLine("${file}: dependencies that are not allowed: $($bad -join ' ')")
        $failed = $true
    } else {
        Write-Output "${file}: ok (pe, $($imports.Count) dependencies)"
    }
}
if ($failed) { exit 1 }
