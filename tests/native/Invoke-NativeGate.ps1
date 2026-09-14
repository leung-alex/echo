[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][ValidateSet('smoke')][string]$Scope,
    [Parameter(Mandatory)][string]$EvidenceRoot,
    [ValidateSet('software')][string]$Renderer='software'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$Root = [IO.Path]::GetFullPath($Root)
$Executable = [IO.Path]::GetFullPath($Executable)
$EvidenceRoot = [IO.Path]::GetFullPath($EvidenceRoot)
if (!(Test-Path -LiteralPath (Join-Path $Root 'AGENTS.md') -PathType Leaf)) { throw "Repository root is invalid: $Root" }
if (!(Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "Echo executable is missing: $Executable" }
if (Test-Path -LiteralPath $EvidenceRoot) { throw "Evidence root must be new and immutable-by-convention: $EvidenceRoot" }

$native = Join-Path $Root 'tests/native'
$tools = Join-Path $EvidenceRoot 'test-tools'
$data = Join-Path $EvidenceRoot 'data'
$fixtureSets = Join-Path $EvidenceRoot 'fixture-datasets'
New-Item -ItemType Directory -Path $EvidenceRoot,$tools | Out-Null

$oldEnvironment = @{}
foreach ($name in @('ECHO_DATA_DIR','ECHO_ACCEPTANCE_RUN_ROOT','ECHO_ACCEPTANCE_PID','ECHO_RENDERER','ECHO_NATIVE_TEST_ROOT','ECHO_WINDOWS_ACCEPTANCE')) {
    $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Write-JsonNew([string]$Path, [object]$Value) {
    if (Test-Path -LiteralPath $Path) { throw "Refusing to overwrite evidence: $Path" }
    $Value | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
}

$summary = [ordered]@{
    schema = 'echo.native.gate.v2'
    status = 'FAIL'
    scope = $Scope
    started_utc = [DateTime]::UtcNow.ToString('o')
    evidence_root = $EvidenceRoot
    checks = @()
    limitations = @(
        [ordered]@{ name='physical-ime'; status='NOT_RUN'; reason='UI Automation and synthetic key events do not certify a physical IME.' },
        [ordered]@{ name='physical-dpi-matrix'; status='NOT_RUN'; reason='Screenshots record only the DPI configuration actually used by the run.' }
    )
}

try {
    $framework = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319'
    $compiler = Join-Path $framework 'csc.exe'
    if (!(Test-Path -LiteralPath $compiler)) { throw 'The .NET Framework C# compiler is required for UIAutomationClient/UIAutomationTypes.' }
    $driver = Join-Path $tools 'EchoSmokeDriver.exe'
    & $compiler /nologo /target:exe /out:$driver "/reference:$framework/WPF/UIAutomationClient.dll" "/reference:$framework/WPF/UIAutomationTypes.dll" "/reference:$framework/WPF/WindowsBase.dll" /reference:System.Drawing.dll /reference:System.Windows.Forms.dll /reference:System.Web.Extensions.dll (Join-Path $native 'EchoUi.cs') (Join-Path $native 'EchoSmokeDriver.cs')
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $driver)) { throw 'EchoDriver compilation failed.' }

    $templateRoot = $env:ECHO_NATIVE_FIXTURE_DIR
    if ([string]::IsNullOrWhiteSpace($templateRoot)) {
        $generator = $env:ECHO_NATIVE_FIXTURE_EXE
        if ([string]::IsNullOrWhiteSpace($generator)) { throw 'Set ECHO_NATIVE_FIXTURE_DIR to prebuilt D0/D1/D2, or ECHO_NATIVE_FIXTURE_EXE to the prebuilt echo-storage native_fixture example.' }
        $generator = [IO.Path]::GetFullPath($generator)
        if (!(Test-Path -LiteralPath $generator -PathType Leaf)) { throw "Fixture generator is missing: $generator" }
        & $generator $fixtureSets
        if ($LASTEXITCODE -ne 0) { throw 'Synthetic fixture generation failed.' }
        $templateRoot = $fixtureSets
    }
    $templateRoot = [IO.Path]::GetFullPath($templateRoot)
    $dataset = 'D0'
    $template = Join-Path $templateRoot $dataset
    if (!(Test-Path -LiteralPath (Join-Path $template 'echo.sqlite3') -PathType Leaf)) { throw "Fixture dataset is invalid: $template" }
    Copy-Item -LiteralPath $template -Destination $data -Recurse

    $identity = [ordered]@{
        repository_head = (& git -C $Root rev-parse HEAD).Trim()
        repository_status = @(& git -C $Root status --short)
        executable = [ordered]@{ path=$Executable; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $Executable).Hash; version=(Get-Item -LiteralPath $Executable).VersionInfo.FileVersion }
        driver = [ordered]@{ path=$driver; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $driver).Hash }
        dataset = [ordered]@{ name=$dataset; path=$template; database_sha256=(Get-FileHash -LiteralPath (Join-Path $template 'echo.sqlite3') -Algorithm SHA256).Hash }
        environment = [ordered]@{ os=[Environment]::OSVersion.VersionString; powershell=$PSVersionTable.PSVersion.ToString(); process_architecture=[Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString(); machine=$env:COMPUTERNAME; user_interactive=[Environment]::UserInteractive }
    }
    Write-JsonNew (Join-Path $EvidenceRoot 'identity.json') $identity

    [Environment]::SetEnvironmentVariable('ECHO_DATA_DIR', $data, 'Process')
    [Environment]::SetEnvironmentVariable('ECHO_ACCEPTANCE_RUN_ROOT', $EvidenceRoot, 'Process')
    [Environment]::SetEnvironmentVariable('ECHO_RENDERER', $Renderer, 'Process')
    # Read-only smoke authorizes only this isolated synthetic native-test instance.
    [Environment]::SetEnvironmentVariable('ECHO_NATIVE_TEST_ROOT', $EvidenceRoot, 'Process')
    [Environment]::SetEnvironmentVariable('ECHO_WINDOWS_ACCEPTANCE', '1', 'Process')

    & (Join-Path $native 'Invoke-Smoke.ps1') -Root $Root -Executable $Executable -Driver $driver -Scope $Scope -EvidenceRoot $EvidenceRoot
    if ($LASTEXITCODE -ne 0) { throw "Native $Scope acceptance failed." }
    $detail = Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot 'checks.json') | ConvertFrom-Json
    $summary.checks = @($detail)
    $summary.status = if (@($detail | Where-Object status -eq 'FAIL').Count -eq 0) { 'PASS' } else { 'FAIL' }
    if($summary.status -ne 'PASS'){throw 'One or more native checks failed; see checks.json'}
}
catch {
    $summary.error = $_.ToString()
    throw
}
finally {
    $summary.finished_utc = [DateTime]::UtcNow.ToString('o')
    $summaryPath = Join-Path $EvidenceRoot 'summary.json'
    if (!(Test-Path -LiteralPath $summaryPath)) { Write-JsonNew $summaryPath $summary }
    foreach ($name in $oldEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name], 'Process') }
}
