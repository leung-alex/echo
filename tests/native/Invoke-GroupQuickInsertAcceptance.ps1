[CmdletBinding()]
param([string]$Root = (Resolve-Path "$PSScriptRoot/../..").Path,
      [Parameter(Mandatory)][string]$EvidenceRoot)
$ErrorActionPreference = 'Stop'
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1') { throw 'Explicit Windows acceptance authorization required.' }
$EvidenceRoot = [IO.Path]::GetFullPath($EvidenceRoot)
if (Test-Path -LiteralPath $EvidenceRoot) { throw 'Use a new evidence directory.' }
$lease = [Threading.Mutex]::new($false, 'Local\Echo.Completion.Acceptance.Foreground')
if (!$lease.WaitOne(0)) { $lease.Dispose(); throw 'Another foreground acceptance is active.' }
$oldFixture = $env:ECHO_GROUP_FIXTURE_EXE
$oldData = $env:ECHO_DATA_DIR
try {
    New-Item -ItemType Directory $EvidenceRoot | Out-Null
    $framework = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319"
    $references = @('PresentationFramework','PresentationCore','WindowsBase','UIAutomationTypes','UIAutomationProvider') | ForEach-Object { "/reference:$framework\WPF\$_.dll" }
    & "$framework\csc.exe" /nologo /target:winexe "/out:$EvidenceRoot\fixture.exe" /reference:System.Xaml.dll @references (Join-Path $PSScriptRoot 'EchoGroupInputFixture.cs')
    if ($LASTEXITCODE) { throw 'Fixture compilation failed.' }
    $env:ECHO_GROUP_FIXTURE_EXE = Join-Path $EvidenceRoot 'fixture.exe'
    $env:ECHO_DATA_DIR = Join-Path $EvidenceRoot 'synthetic-data'
    $script = Join-Path $EvidenceRoot 'run.ps1'
    @'
cargo test -p echo-windows --test group_quick_insert --locked -- --ignored --nocapture --test-threads=1
'@ | Set-Content -LiteralPath $script
    Push-Location $Root
    try {
        & pwsh -NoProfile -STA -File "$PSScriptRoot/Invoke-WithClipboardBackup.ps1" -Script $script -Report "$EvidenceRoot/clipboard-preservation.json" *> "$EvidenceRoot/result.log"
        $code = $LASTEXITCODE
        Get-Content -LiteralPath "$EvidenceRoot/result.log"
        if ($code) { throw "Group Quick Insert acceptance failed ($code)." }
    } finally { Pop-Location }
} finally {
    $env:ECHO_GROUP_FIXTURE_EXE = $oldFixture
    $env:ECHO_DATA_DIR = $oldData
    $lease.ReleaseMutex(); $lease.Dispose()
}
