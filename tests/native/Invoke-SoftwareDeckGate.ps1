[CmdletBinding()]
param([Parameter(Mandatory)][string]$Root,[Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Template,[Parameter(Mandatory)][string]$EvidenceRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:ECHO_WINDOWS_ACCEPTANCE -ne '1'){throw 'Explicit native acceptance authorization required'}
if(Test-Path -LiteralPath $EvidenceRoot){throw 'Evidence root must be new'}
$EvidenceRoot=[IO.Path]::GetFullPath($EvidenceRoot)
$runner=$EvidenceRoot+'.runner.ps1';$report=$EvidenceRoot+'.clipboard.json'
if((Test-Path -LiteralPath $runner) -or (Test-Path -LiteralPath $report)){throw 'Evidence companion files already exist'}
$old=$env:ECHO_SOFTWARE_DECK_ARGUMENTS
$code=1
try{
    $env:ECHO_SOFTWARE_DECK_ARGUMENTS=@{Root=$Root;Executable=$Executable;Template=$Template;EvidenceRoot=$EvidenceRoot}|ConvertTo-Json -Compress
    @'
$a=$env:ECHO_SOFTWARE_DECK_ARGUMENTS|ConvertFrom-Json -AsHashtable
& (Join-Path $a.Root 'tests/native/Invoke-SoftwareDeckAcceptance.ps1') @a
$global:LASTEXITCODE=$LASTEXITCODE
'@|Set-Content -LiteralPath $runner -Encoding utf8NoBOM
    & pwsh -NoProfile -STA -File (Join-Path $Root 'tests/native/Invoke-WithClipboardBackup.ps1') -Script $runner -Report $report
    $code=$LASTEXITCODE
}finally{$env:ECHO_SOFTWARE_DECK_ARGUMENTS=$old}
exit $code
