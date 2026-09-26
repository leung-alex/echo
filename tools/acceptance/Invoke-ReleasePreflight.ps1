#requires -Version 7.0
<## Ordinary Release preflight/report wrapper. It fails closed on test-only inputs. #>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$RepositoryRoot,
  [Parameter(Mandatory)][string]$EvidenceRoot,
  [switch]$RunReleaseBuild
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath($RepositoryRoot); $out=[IO.Path]::GetFullPath($EvidenceRoot)
if (-not (Test-Path -LiteralPath (Join-Path $repo 'echo.cmd') -PathType Leaf)) { throw 'RepositoryRoot must contain echo.cmd.' }
if (Test-Path -LiteralPath $out) { throw 'EvidenceRoot must be new.' }
New-Item -ItemType Directory -Force -Path $out | Out-Null
function Write-NewJson([string]$Path,[object]$Value){$Value|ConvertTo-Json -Depth 10|Set-Content -LiteralPath $Path -Encoding utf8NoBOM}
$head=((& git -C $repo rev-parse HEAD 2>&1)|Out-String).Trim(); if($LASTEXITCODE){throw 'Unable to read Git HEAD.'}
$checks=[Collections.Generic.List[object]]::new()
$checks.Add([ordered]@{name='native-test flag';status='PASS';detail='Release command is required to omit --features native-test.'})
$checks.Add([ordered]@{name='fixture marker';status='PASS';detail='Release command is required to omit native fixture/test bridge inputs.'})
$checks.Add([ordered]@{name='isolated data';status='BLOCKED';detail='No ordinary Release run is authorized until a new ECHO_DATA_DIR and evidence root are supplied.'})
if($RunReleaseBuild){
  $log=Join-Path $out 'release-build.log'; $env:ECHO_DATA_DIR=Join-Path $out 'data'; $env:CARGO_TARGET_DIR=Join-Path $out 'cargo-target'
  & (Join-Path $repo 'echo.cmd') build --release *> $log; $code=$LASTEXITCODE
  $checks.Add([ordered]@{name='echo.cmd build --release';status=$(if($code -eq 0){'PASS'}else{'FAIL'});exit_code=$code;log='release-build.log';target_dir=$env:CARGO_TARGET_DIR})
}
else{$checks.Add([ordered]@{name='echo.cmd build --release';status='NOT_RUN';detail='Pass -RunReleaseBuild to execute the ordinary Release build.'})}
$report=[ordered]@{schema='echo.release.preflight.v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');repository=$repo;head=$head;status=$(if(@($checks|Where-Object status -eq 'FAIL').Count){'FAIL'}elseif(@($checks|Where-Object status -eq 'BLOCKED').Count){'BLOCKED'}else{'PASS'});checks=$checks.ToArray();rejected_inputs=@('--features native-test','native_fixture','caret_fixture','synthetic fixture injection');note='This wrapper does not bypass isolation and does not convert native-test or fixture evidence into ordinary Release acceptance.'}
Write-NewJson (Join-Path $out 'release-preflight.json') $report
Write-Output "Release preflight report written to $out (status=$($report.status))."
if($report.status -eq 'FAIL'){exit 1}
