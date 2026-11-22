# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
<#
Runs the cross-system tests of the system, log, network, schedule, service
and program crates on Windows.

Environment:
  NURT_IT_SRC         root of a clone of the repository; default: the clone
                      that contains this script
  NURT_IT_OUT         directory of the results; default: target\it below
                      NURT_IT_SRC
  NURT_IT_SUITES      suites of matrix.toml separated by spaces or commas:
                      container, system, scenarios; default: container
  NURT_IT_PRIVILEGED  1 runs the tests that change the configuration of the
                      system; default: 0
  NURT_IT_NURT        program installed as the service by the service
                      scenarios; without it these scenarios are skipped

Results: test-<suite>.log, junit-<suite>.xml when cargo-nextest and the
nextest profile `ci` are available, and summary.json. The exit code is 0
when every suite passed.
#>

$ErrorActionPreference = 'Stop'

$Src = if ($env:NURT_IT_SRC) { $env:NURT_IT_SRC } else { (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path }
$Out = if ($env:NURT_IT_OUT) { $env:NURT_IT_OUT } else { Join-Path $Src 'target\it' }
$Suites = if ($env:NURT_IT_SUITES) { $env:NURT_IT_SUITES } else { 'container' }
if (-not $env:NURT_IT_PRIVILEGED) { $env:NURT_IT_PRIVILEGED = '0' }

$Crates = @('-p', 'nurt-sys', '-p', 'nurt-syslog', '-p', 'nurt-net', '-p', 'nurt-sched', '-p', 'nurt-service', '-p', 'nurt-programs')

# Test objects of the registry and service tests, and the entries of the
# service scenarios that did not exist before the run.
$TestKey = 'HKCU:\Software\NurtTest'
$TestService = 'NurtTestSvc'
$HadService = $null -ne (Get-Service -Name 'NurtService' -ErrorAction SilentlyContinue)
$HadTask = $null -ne (Get-ScheduledTask -TaskName 'Nurt Service' -ErrorAction SilentlyContinue)

function Remove-TestObjects {
    if (Test-Path $TestKey) {
        Remove-Item -Path $TestKey -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (Get-Service -Name $TestService -ErrorAction SilentlyContinue) {
        Stop-Service -Name $TestService -Force -ErrorAction SilentlyContinue
        & sc.exe delete $TestService | Out-Null
    }
    if (-not $HadService -and (Get-Service -Name 'NurtService' -ErrorAction SilentlyContinue)) {
        Stop-Service -Name 'NurtService' -Force -ErrorAction SilentlyContinue
        & sc.exe delete 'NurtService' | Out-Null
    }
    if (-not $HadTask -and (Get-ScheduledTask -TaskName 'Nurt Service' -ErrorAction SilentlyContinue)) {
        Unregister-ScheduledTask -TaskName 'Nurt Service' -Confirm:$false -ErrorAction SilentlyContinue
    }
}

function Test-NextestProfile {
    $config = Join-Path $Src '.config\nextest.toml'
    if (-not (Get-Command cargo-nextest -ErrorAction SilentlyContinue)) { return $false }
    return (Test-Path $config) -and (Select-String -Path $config -Pattern '^\[profile\.ci' -Quiet)
}

# Sums the counters of the `test result:` lines of a log.
function Get-Counts([string] $Log) {
    $counts = @{ passed = 0; failed = 0; ignored = 0 }
    foreach ($line in Select-String -Path $Log -Pattern '^test result:') {
        foreach ($name in @('passed', 'failed', 'ignored')) {
            if ($line.Line -match "(\d+) $name") { $counts[$name] += [int] $Matches[1] }
        }
    }
    return $counts
}

function Invoke-Suite([string] $Suite) {
    switch ($Suite) {
        'container' { $arguments = @(); $runIgnored = 'default' }
        'system' { $arguments = @('--include-ignored'); $runIgnored = 'all' }
        'scenarios' { $arguments = @('--ignored'); $runIgnored = 'only' }
        default { throw "unknown suite: $Suite; expected container, system or scenarios" }
    }
    $log = Join-Path $Out "test-$Suite.log"
    $status = 0
    Push-Location $Src
    try {
        if (Test-NextestProfile) {
            & cargo nextest run --locked @Crates --run-ignored $runIgnored --profile ci *> $log
            if ($LASTEXITCODE -ne 0) { $status = $LASTEXITCODE }
            Copy-Item -Path 'target\nextest\ci\junit.xml' -Destination (Join-Path $Out "junit-$Suite.xml") -ErrorAction SilentlyContinue
        }
        & cargo test --locked @Crates -- @arguments *>> $log
        if ($LASTEXITCODE -ne 0) { $status = $LASTEXITCODE }
    }
    finally {
        Pop-Location
    }
    $counts = Get-Counts $log
    return [ordered]@{ suite = $Suite; exit_code = $status; passed = $counts.passed; failed = $counts.failed; ignored = $counts.ignored }
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$results = @()
$failed = $false
try {
    foreach ($suite in ($Suites -split '[,\s]+' | Where-Object { $_ })) {
        $result = Invoke-Suite $suite
        if ($result.exit_code -ne 0) { $failed = $true }
        $results += $result
    }
}
finally {
    Remove-TestObjects
    [ordered]@{ system = 'windows'; suites = $results } | ConvertTo-Json -Depth 4 | Set-Content -Path (Join-Path $Out 'summary.json') -Encoding utf8
}
if ($failed) { exit 1 }
exit 0
