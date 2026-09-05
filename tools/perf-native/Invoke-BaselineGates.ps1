#requires -Version 7.0
<#
Runs existing Echo gates on an explicitly dedicated Windows test desktop.
This is M0 preparation, NOT a startup/memory benchmark or a G0 approval.
No checkout/reset/clean/push, dependency installation, clipboard backup, or
system WebView2 removal is performed. Evidence remains local and outside Git.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$RepositoryRoot,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [switch]$DedicatedTestDesktop,
    [switch]$IncludeNativeAcceptance
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if (-not $IsWindows) { throw 'A real Windows host is required.' }
if (-not $DedicatedTestDesktop) {
    throw 'Use an isolated test account/desktop, then specify -DedicatedTestDesktop. Tests can change the clipboard.'
}
$baselineSha = '0dc699e42d8d667e502938d71e33216f92513e5a'
. (Join-Path $PSScriptRoot 'Common.ps1')
function Git-Read([string[]]$GitArgs) {
    $result = & git -C $repo @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Git read failed: $($GitArgs -join ' ')" }
    return (($result | Out-String).Trim())
}
function Assert-NoRunningEcho {
    $running = @(Get-CimInstance Win32_Process -Filter "Name = 'echo-desktop.exe' OR Name = 'echo.exe'")
    if ($running.Count -gt 0) {
        throw 'An Echo/tool process is already running. This script will not stop an existing process or risk a single-instance handoff.'
    }
}
$repo = [IO.Path]::GetFullPath($RepositoryRoot)
Assert-EchoNoReparsePoint $repo
$repo = Git-Read @('rev-parse', '--show-toplevel')
if (-not (Test-Path -LiteralPath (Join-Path $repo 'echo.cmd') -PathType Leaf)) {
    throw 'Repository does not contain echo.cmd.'
}
$output = [IO.Path]::GetFullPath($EvidenceRoot)
Assert-EchoNoReparsePoint $output
Assert-EchoOutsideRoot $repo $output
if (Test-Path -LiteralPath $output) { throw 'EvidenceRoot must be new; existing evidence is never overwritten.' }
foreach ($name in @('git', 'go', 'cargo', 'rustc', 'pnpm')) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) { throw "Missing prerequisite: $name" }
}
$goVersion = (& go env GOVERSION | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $goVersion -ne 'go1.26.2') { throw 'The existing repository requires Go 1.26.2; do not change its pinned version for A0.' }
$head = Git-Read @('rev-parse', 'HEAD')
$branch = Git-Read @('branch', '--show-current')
if ($branch -ne 'codex/ui') { throw "Expected codex/ui; found '$branch'. No branch is changed automatically." }
if (Git-Read @('status', '--porcelain', '--untracked-files=all')) { throw 'Worktree is not clean; no reset or clean will be attempted.' }
Assert-NoRunningEcho
$output = New-EchoEvidenceDirectory $output
$steps = [Collections.Generic.List[object]]::new()
$artifacts = [Collections.Generic.List[object]]::new()
$failure = $null
$priorAuthorization = [Environment]::GetEnvironmentVariable('ECHO_WINDOWS_ACCEPTANCE', 'Process')
function Invoke-Gate([string]$Name, [string]$Command, [string[]]$CommandArgs) {
    Assert-NoRunningEcho
    $log = Join-Path $output "$Name.log"
    if (Test-Path -LiteralPath $log) { throw "Refusing to overwrite $Name.log" }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $exitCode = -1
    try {
        & $Command @CommandArgs *> $log
        $exitCode = $LASTEXITCODE
    } finally {
        $clock.Stop()
        $steps.Add([ordered]@{
            name = $Name; status = $(if ($exitCode -eq 0) { 'PASS' } else { 'FAIL' })
            exit_code = $exitCode; gate_wall_ms = $clock.Elapsed.TotalMilliseconds
            log = "$Name.log"; note = 'Gate runtime is NOT product startup latency.'
        })
    }
    if ($exitCode -ne 0) { throw "Gate failed: $Name; exit=$exitCode" }
    if ((Git-Read @('rev-parse', 'HEAD')) -ne $head) { throw 'HEAD changed during baseline collection.' }
    if (Git-Read @('status', '--porcelain', '--untracked-files=all')) { throw "Gate changed tracked/untracked inputs: $Name" }
}
try {
    $env:ECHO_WINDOWS_ACCEPTANCE = '1'
    & (Join-Path $PSScriptRoot 'Capture-Environment.ps1') -OutputFile (Join-Path $output 'environment.json') -Variant 'A0'
    Write-EchoJsonNew (Join-Path $output 'source-manifest.json') ([ordered]@{
        schema = 'echo.baseline.source.v1'; original_product_sha = $baselineSha
        checkout_sha = $head; branch = $branch; captured_at_utc = [DateTime]::UtcNow.ToString('o')
        variant = 'A0'; status = 'NOT_MEASURED'; product_changes_allowed = $false
        note = 'Only M0 tooling/documentation changes may differ from original_product_sha.'
    })
    $helper = Join-Path $output 'perf-native.exe'
    Invoke-Gate 'build-evidence-tool' 'go' @('-C', (Join-Path $repo 'tools/echo'), 'build', '-trimpath', '-o', $helper, './perf-native')
    Invoke-Gate 'preflight' $helper @('preflight', '--repo', $repo, '--out', (Join-Path $output 'preflight.json'))
    $entry = Join-Path $repo 'echo.cmd'
    Invoke-Gate 'self-check' $entry @('self-check')
    Invoke-Gate 'format' $entry @('format', '--check')
    Invoke-Gate 'bindings' $entry @('bindings', '--check')
    Invoke-Gate 'verify' $entry @('verify')
    Invoke-Gate 'release-build' $entry @('build', '--release')
    Invoke-Gate 'storage-perf' $entry @('perf')
    # Existing smoke builds/runs Debug. Preserve it, but never label it Release startup.
    Invoke-Gate 'debug-bootstrap-smoke' $entry @('smoke')
    if ($IncludeNativeAcceptance) {
        Invoke-Gate 'acceptance-clipboard' $entry @('acceptance', 'clipboard')
        Invoke-Gate 'acceptance-quick-insert' $entry @('acceptance', 'quick-insert')
    } else {
        foreach ($name in @('acceptance-clipboard', 'acceptance-quick-insert')) {
            $steps.Add([ordered]@{ name = $name; status = 'NOT_RUN'; reason = '-IncludeNativeAcceptance not provided' })
        }
    }
    Invoke-Gate 'package' $entry @('package')
    $binary = Join-Path $repo 'target/release/echo-desktop.exe'
    Assert-EchoNoReparsePoint $binary
    $binaryCopy = Join-Path $output 'echo-desktop-A0.exe'
    [IO.File]::Copy($binary, $binaryCopy, $false)
    $artifacts.Add([ordered]@{
        name = 'echo-desktop-A0.exe'; bytes = (Get-Item -LiteralPath $binaryCopy).Length
        sha256 = (Get-FileHash -LiteralPath $binaryCopy -Algorithm SHA256).Hash.ToLowerInvariant()
        scope = 'executable only; not complete installed runtime inventory'
    })
} catch {
    $failure = $_.Exception.Message
} finally {
    try {
    if (Test-Path -LiteralPath $output -PathType Container) {
        Write-EchoJsonNew (Join-Path $output 'gate-summary.json') ([ordered]@{
            schema = 'echo.baseline.gates.v1'; status = $(if ($failure) { 'FAIL' } else { 'GATES_RECORDED' })
            g0 = 'NOT_RUN'; source_sha = $baselineSha; checkout_sha = $head
            steps = @($steps.ToArray()); artifacts = @($artifacts.ToArray()); failure = $failure
            remaining = @('D1/D2 dataset snapshots', 'full installed-runtime/installer inventory',
                'real window screenshots and visual review', 'external semantic UI-ready timing',
                'complete-process ownership verification', 'independent memory/CPU runs',
                'observer overhead', 'G0 evidence review')
            note = 'Existing acceptance deletes its temporary run directory; its console log alone is not complete native evidence.'
        })
    }
    } finally {
        [Environment]::SetEnvironmentVariable('ECHO_WINDOWS_ACCEPTANCE', $priorAuthorization, 'Process')
    }
}
if ($failure) { throw $failure }
Write-Output "Gate logs saved to $output. G0 remains NOT_RUN until real measurements and evidence are complete."
