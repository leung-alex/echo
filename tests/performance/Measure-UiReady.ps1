param(
 [Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Fixture,
 [Parameter(Mandatory)][string]$OutDir,[Parameter(Mandatory)][string]$Variant,
 [int]$Runs=30,[int]$HotRuns=50,[int]$TimeoutSeconds=20,
 [string]$ExpectedText='echo-fixture-4999@example.net'
)
Set-StrictMode -Version Latest; $ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot 'NativeProbe.cs')
if(Get-Process -Name echo-desktop -ErrorAction SilentlyContinue){throw 'An Echo process is already running; refusing a possible handoff'}
if(Test-Path $OutDir){throw 'Evidence output already exists'}
New-Item -ItemType Directory -Path $OutDir|Out-Null
$records=[Collections.Generic.List[object]]::new(); $probeCosts=[Collections.Generic.List[double]]::new()
$cache=[Windows.Automation.CacheRequest]::new()
foreach($property in @([Windows.Automation.AutomationElement]::NameProperty,[Windows.Automation.AutomationElement]::IsOffscreenProperty,[Windows.Automation.AutomationElement]::IsEnabledProperty)){$cache.Add($property)}
$cache.TreeScope=[Windows.Automation.TreeScope]::Element
function Inspect-Window([long]$Handle,[string]$Expected) {
 $root=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Handle)
 $scope=$cache.Activate()
 try{$items=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}finally{$scope.Dispose()}
 $search=$false; $content=$false
 foreach($e in $items){try{$c=$e.Cached; if(-not $c.IsOffscreen -and $c.IsEnabled){if($c.Name -like 'Search*'){$search=$true}; if($Expected -eq '' -or $c.Name.Contains($Expected)){$content=$true}}}catch{}}
 return $search -and $content
}
function Wait-Ready([Diagnostics.Process]$App) {
 $limit=[Diagnostics.Stopwatch]::StartNew(); $attempts=0
 while($limit.Elapsed.TotalSeconds -lt $TimeoutSeconds){
  if($App.HasExited){throw "Echo exited: $($App.ExitCode)"}
  $attempts++; $p=[Diagnostics.Stopwatch]::StartNew(); $ready=$false
  try{
   $windows=@([EchoProbeNative]::Windows($App.Id)|Where-Object {$_.Visible})
   $main=@($windows|Where-Object {$_.Title -eq 'Echo Recall'})
   $fav=@($windows|Where-Object {$_.Title -eq 'Echo Favorites'})
   if($main.Count -eq 1 -and $fav.Count -eq 1){$ready=(Inspect-Window $main[0].Handle $ExpectedText) -and (Inspect-Window $fav[0].Handle '')}
  }catch{}
  $p.Stop(); $probeCosts.Add($p.Elapsed.TotalMilliseconds)
  if($ready){return $attempts}; Start-Sleep -Milliseconds 10
 }
 throw 'UIA semantic readiness timed out (visible dual windows, search, first expected result)'
}
function New-StartInfo([string]$Data,[string]$Profile,[string]$Arguments='') {
 $psi=[Diagnostics.ProcessStartInfo]::new(); $psi.FileName=$Executable; $psi.WorkingDirectory=(Split-Path $Executable)
 $psi.UseShellExecute=$false; $psi.Arguments=$Arguments
 $psi.EnvironmentVariables['ECHO_DATA_DIR']=$Data; $psi.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER']=$Profile
 $psi.EnvironmentVariables.Remove('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')
 return $psi
}
function Stop-Owned([Diagnostics.Process]$App) {
 if(-not $App.HasExited){$App.Kill(); if(-not $App.WaitForExit(5000)){throw 'Owned test process did not exit'}}
 $App.Dispose(); Start-Sleep -Milliseconds 200
}
function Write-Results {
 $o=[ordered]@{schema='echo.external.ui-ready.v1';variant=$Variant;exe=$Executable;exe_sha256=(Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant();fixture=$Fixture;clock='QPC';frequency=[Diagnostics.Stopwatch]::Frequency;endpoint='Windows UIA: dual visible windows + enabled search + expected first result';records=$records.ToArray();probe_durations_ms=$probeCosts.ToArray();note='External observation includes UIA query and 10ms polling overhead. Repeated process starts use a warm OS file cache; not boot-cold starts. Same probe must be used for both variants.'}
 [IO.File]::WriteAllText((Join-Path $OutDir 'results.json'),($o|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
}
for($i=0;$i -lt $Runs;$i++){
 $run=Join-Path $OutDir ('start-{0:d3}' -f $i); New-Item -ItemType Directory -Path $run|Out-Null
 Copy-Item $Fixture (Join-Path $run 'data') -Recurse
 $psi=New-StartInfo (Join-Path $run 'data') (Join-Path $run 'webview2'); $app=$null
 $result=[ordered]@{kind='process-start';run_id=$i;status='FAIL';ms=$null;attempts=0;error=$null}
 try{$sw=[Diagnostics.Stopwatch]::StartNew();$app=[Diagnostics.Process]::Start($psi);$n=Wait-Ready $app;$sw.Stop();$result.status='PASS';$result.ms=$sw.Elapsed.TotalMilliseconds;$result.attempts=$n}
 catch{$result.error=$_.Exception.Message}
 finally{if($null -ne $app){Stop-Owned $app};$records.Add($result);Write-Results}
 Write-Output "start $i $($result.status) $($result.ms)"
}
if($HotRuns -gt 0){
 $run=Join-Path $OutDir 'hot';New-Item -ItemType Directory -Path $run|Out-Null;Copy-Item $Fixture (Join-Path $run 'data') -Recurse
 $data=Join-Path $run 'data';$profile=Join-Path $run 'webview2';$app=[Diagnostics.Process]::Start((New-StartInfo $data $profile))
 try{
  Wait-Ready $app|Out-Null
  for($i=0;$i -lt $HotRuns;$i++){
   foreach($w in [EchoProbeNative]::Windows($app.Id)){if($w.Title -eq 'Echo Recall'){[EchoProbeNative]::PostMessage([IntPtr]$w.Handle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null}}
   $hide=[Diagnostics.Stopwatch]::StartNew()
   while(@([EchoProbeNative]::Windows($app.Id)|Where-Object {$_.Visible -and $_.Title -like 'Echo*'}).Count -gt 0){if($hide.Elapsed.TotalSeconds -gt 5){throw 'Closing did not hide the composition'};Start-Sleep -Milliseconds 10}
   Start-Sleep -Milliseconds 100
   $envelope=@{version=1;request_id=[Guid]::NewGuid().ToString();action='echo.open';origin=@{platform='windows'};payload=@{}}|ConvertTo-Json -Compress
   $encoded=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($envelope)).TrimEnd('=').Replace('+','-').Replace('/','_')
   $sw=[Diagnostics.Stopwatch]::StartNew();$activation=[Diagnostics.Process]::Start((New-StartInfo $data $profile ("--echo-activate "+$encoded)))
   $n=Wait-Ready $app;$sw.Stop();if(-not $activation.WaitForExit(5000)){Stop-Owned $activation;throw 'Secondary activation failed to exit'}
   $code=$activation.ExitCode;$activation.Dispose();$records.Add([ordered]@{kind='hot-show';run_id=$i;status=$(if($code -eq 0){'PASS'}else{'FAIL'});ms=$sw.Elapsed.TotalMilliseconds;attempts=$n;activation_exit=$code});Write-Results
   Write-Output "hot $i $($sw.Elapsed.TotalMilliseconds)"
  }
 }finally{Stop-Owned $app;Write-Results}
}
