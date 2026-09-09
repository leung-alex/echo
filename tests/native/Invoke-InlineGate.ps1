[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$FixtureGenerator,
    [Parameter(Mandatory)][ValidateSet('clipboard','quick-insert')][string]$Scope,
    [Parameter(Mandatory)][string]$EvidenceRoot
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1') { throw 'Explicit native acceptance authorization required.' }
$Root = [IO.Path]::GetFullPath($Root)
$EvidenceRoot = [IO.Path]::GetFullPath($EvidenceRoot)
if (Test-Path -LiteralPath $EvidenceRoot) { throw 'Evidence directory must be new.' }
$tools = Join-Path $EvidenceRoot 'tools'
New-Item -ItemType Directory -Path $EvidenceRoot,$tools | Out-Null
$code = 1
try {
    Copy-Item -LiteralPath $Executable -Destination (Join-Path $EvidenceRoot 'echo-acceptance.exe')
    $native = Join-Path $Root 'tests/native'
    $f = "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319"
    $refs = @("/reference:$f/WPF/UIAutomationClient.dll", "/reference:$f/WPF/UIAutomationTypes.dll", "/reference:$f/WPF/WindowsBase.dll", '/reference:System.Drawing.dll', '/reference:System.Windows.Forms.dll', '/reference:System.Web.Extensions.dll', '/reference:System.IO.Compression.dll')
    $sets = @{ EchoDriver=@('EchoUi.cs','EchoDriver.cs','EchoBenchmarks.cs','EchoComposition.cs'); EchoInlineDriver=@('EchoUi.cs','EchoInlineDriver.cs'); EchoInlineFixture=@('EchoInlineFixture.cs') }
    foreach ($name in $sets.Keys) {
        $sources = @($sets[$name] | ForEach-Object { Join-Path $native $_ })
        & "$f/csc.exe" /nologo /target:exe "/out:$tools/$name.exe" @refs @sources *> (Join-Path $EvidenceRoot "build-$name.log")
        if ($LASTEXITCODE) { throw "Failed to compile $name" }
    }
    & $FixtureGenerator (Join-Path $EvidenceRoot 'fixtures') *> (Join-Path $EvidenceRoot 'fixtures.log')
    if ($LASTEXITCODE) { throw 'Synthetic fixture generation failed.' }
    $env:ECHO_INLINE_GATE_ROOT = $Root
    $env:ECHO_INLINE_GATE_EVIDENCE = $EvidenceRoot
    $env:ECHO_INLINE_GATE_SCOPE = $Scope
    $script = Join-Path $EvidenceRoot 'run-owned-inputs.ps1'
    @'
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = $env:ECHO_INLINE_GATE_ROOT
$e = $env:ECHO_INLINE_GATE_EVIDENCE
Set-Location -LiteralPath $root
$env:PYTHONUTF8 = '1'
if ($env:ECHO_INLINE_GATE_SCOPE -eq 'clipboard') {
    cargo test -p echo-windows --lib authorized_clipboard_roundtrip_and_capture_exclusions --locked -- --ignored --nocapture --test-threads=1 *> "$e/clipboard-roundtrip.log"
    $code = $LASTEXITCODE
    [IO.File]::WriteAllText("$e/clipboard-roundtrip.exit", [string]$code)
    if ($code) { exit $code }
}
$pythonArgs = @('tests/native/Invoke-InlineAcceptance.py', '--root', $root, '--executable', "$e/echo-acceptance.exe", '--tools', "$e/tools", '--template', "$e/fixtures/D2", '--evidence', "$e/inline-run", '--renderer', 'femtovg-wgpu', '--native-test')
if ($env:ECHO_INLINE_GATE_SCOPE -eq 'clipboard') { $pythonArgs += @('--only', 'manual-history-row-copy-retains-text') }
python @pythonArgs *> "$e/inline-run.log"
$code = $LASTEXITCODE
[IO.File]::WriteAllText("$e/inline-run.exit", [string]$code)
$global:LASTEXITCODE = $code
'@ | Set-Content -LiteralPath $script -Encoding utf8NoBOM
    & pwsh -NoProfile -STA -File (Join-Path $native 'Invoke-WithClipboardBackup.ps1') -Script $script -Report (Join-Path $EvidenceRoot 'clipboard-preservation.json')
    $code = $LASTEXITCODE
    if ($code -eq 0) {
        $detail = Get-Content -Raw (Join-Path $EvidenceRoot 'inline-run/summary.json') | ConvertFrom-Json
        $preservation = Get-Content -Raw (Join-Path $EvidenceRoot 'clipboard-preservation.json') | ConvertFrom-Json
        if ($detail.status -ne 'PASS' -or !$preservation.restored) { $code = 1 }
    }
}
finally {
    $summary = [ordered]@{
        schema = 'echo.native.inline-gate.v1'
        scope = $Scope
        status = if ($code -eq 0) { 'PASS' } else { 'FAIL' }
        exit_code = $code
        executable_sha256 = if (Test-Path "$EvidenceRoot/echo-acceptance.exe") { (Get-FileHash "$EvidenceRoot/echo-acceptance.exe").Hash } else { $null }
        source_head = (& git -C $Root rev-parse HEAD)
        finished_utc = [DateTime]::UtcNow.ToString('o')
        limitations = @('Physical IME, mixed-DPI hardware, third-party application drafts and endurance are separate acceptance scenarios.')
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $EvidenceRoot 'summary.json') -Encoding utf8NoBOM
}
exit $code
