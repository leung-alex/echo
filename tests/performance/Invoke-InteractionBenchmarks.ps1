#requires -Version 7.0
[CmdletBinding()]
param(
 [Parameter(Mandatory)][string]$Root,
 [Parameter(Mandatory)][string]$Executable,
 [Parameter(Mandatory)][string]$Fixture,
 [Parameter(Mandatory)][string]$EvidenceRoot,
 [ValidateSet('search','cycles')][string]$Scope='search',
 [string]$Variant='native',
 [int]$Count=50
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
if($env:ECHO_WINDOWS_ACCEPTANCE -ne '1'){throw 'ECHO_WINDOWS_ACCEPTANCE=1 is required'}
$Root=[IO.Path]::GetFullPath($Root);$Executable=[IO.Path]::GetFullPath($Executable);$Fixture=[IO.Path]::GetFullPath($Fixture)
. (Join-Path $Root 'tools/perf-native/Common.ps1')
Assert-EchoNoReparsePoint $Executable
Assert-EchoNoReparsePoint $Fixture
Assert-EchoOutsideRoot $Root $EvidenceRoot
$EvidenceRoot=New-EchoEvidenceDirectory $EvidenceRoot
$data=Join-Path $EvidenceRoot 'data'
Copy-Item -LiteralPath $Fixture -Destination $data -Recurse
if($Scope -eq 'cycles'){
 # This isolates repeated UI image/model lifecycle from unrelated user copies.
 # The separate resident measurements keep normal capture enabled.
 & python -c "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('update clipboard_settings set history_enabled=0');c.commit();c.close()" (Join-Path $data 'echo.sqlite3')
 if($LASTEXITCODE){throw 'Cannot isolate the synthetic cycle fixture'}
}
$framework=Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319'
$driver=Join-Path $EvidenceRoot 'EchoDriver.exe'
& "$framework/csc.exe" /nologo /target:exe /out:$driver "/reference:$framework/WPF/UIAutomationClient.dll" "/reference:$framework/WPF/UIAutomationTypes.dll" "/reference:$framework/WPF/WindowsBase.dll" /reference:System.Drawing.dll /reference:System.Windows.Forms.dll /reference:System.Web.Extensions.dll (Join-Path $Root 'tests/native/EchoUi.cs') (Join-Path $Root 'tests/native/EchoDriver.cs') (Join-Path $Root 'tests/native/EchoBenchmarks.cs') (Join-Path $Root 'tests/native/EchoComposition.cs') *> (Join-Path $EvidenceRoot 'driver-build.log')
if($LASTEXITCODE){throw 'Benchmark driver compilation failed'}
$previous=@{}
foreach($name in @('ECHO_DATA_DIR','ECHO_ACCEPTANCE_RUN_ROOT','ECHO_RENDERER','WEBVIEW2_USER_DATA_FOLDER')){$previous[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null
$result=[ordered]@{schema='echo.native.interaction-benchmark.v1';scope=$Scope;variant=$Variant;status='FAIL';count=$Count;started_utc=[DateTime]::UtcNow.ToString('o');binary_sha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant();fixture=$Fixture;capture_disabled_for_ui_stress=($Scope -eq 'cycles');shutdown='NOT_RUN'}
try{
 $env:ECHO_DATA_DIR=$data;$env:ECHO_ACCEPTANCE_RUN_ROOT=$EvidenceRoot;$env:ECHO_RENDERER='software'
 if($Variant.StartsWith('A0')){$env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $EvidenceRoot 'baseline-webview-profile'}
 $app=Start-Process -FilePath $Executable -PassThru -RedirectStandardOutput (Join-Path $EvidenceRoot 'app.stdout.log') -RedirectStandardError (Join-Path $EvidenceRoot 'app.stderr.log')
 $timer=[Diagnostics.Stopwatch]::StartNew();$ready=$false
 while($timer.ElapsedMilliseconds -lt 10000){
  if($app.HasExited){throw 'Owned benchmark application exited during startup'}
  $probe=& $driver ready $app.Id 'Echo Recall' 2>&1
  if($LASTEXITCODE -eq 0 -and ($probe|ConvertFrom-Json).value){$ready=$true;break}
  Start-Sleep -Milliseconds 50
 }
 if(!$ready){throw 'Semantic startup not observed'}
 $arguments=if($Scope -eq 'cycles'){@('benchmark-cycles',"$($app.Id)",'Echo Recall',$Executable,"$Count")}else{@('benchmark-search',"$($app.Id)",'Echo Recall',"$Count")}
 $raw=& $driver @arguments 2>&1
 if($LASTEXITCODE){$raw|Set-Content -LiteralPath (Join-Path $EvidenceRoot 'driver-failure.json') -Encoding utf8NoBOM;throw 'Interaction benchmark failed; see driver-failure.json'}
 $measurement=$raw|ConvertFrom-Json
 Write-EchoJsonNew (Join-Path $EvidenceRoot 'measurements.json') $measurement
 if($measurement.status -ne 'PASS'){throw 'Driver did not return measured results'}
 if($Scope -eq 'cycles' -and !$measurement.value.within_growth_budget){throw 'Finite-cycle private memory/handle/thread growth exceeded the recorded budget'}
 $result.status='PASS'
}
catch{$result.error=$_.Exception.Message;throw}
finally{
 if($null -ne $app -and !$app.HasExited){
  if(!$Variant.StartsWith('A0')){
   $quit=Start-Process -FilePath $Executable -ArgumentList '--quit' -PassThru
   if(!$quit.WaitForExit(5000)){$quit.Kill();$quit.WaitForExit(1000)|Out-Null}
   $quit.Dispose()
   if($app.WaitForExit(5000)){$result.shutdown='GRACEFUL'}
  }
  if(!$app.HasExited){$app.Kill();$app.WaitForExit(5000)|Out-Null;$result.shutdown='OWNED_TEST_PROCESS_TERMINATED'}
  $app.Dispose()
 }
 foreach($name in $previous.Keys){[Environment]::SetEnvironmentVariable($name,$previous[$name],'Process')}
 $result.finished_utc=[DateTime]::UtcNow.ToString('o')
 Write-EchoJsonNew (Join-Path $EvidenceRoot 'result.json') $result
}
Write-Output "$Scope $Variant $($result.status): $EvidenceRoot"
