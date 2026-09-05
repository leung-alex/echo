param([Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Fixture,[Parameter(Mandatory)][string]$OutDir,[Parameter(Mandatory)][string]$Variant,[ValidateSet('hidden','visible')][string]$Mode,[int]$Runs=1,[int]$DurationSeconds=300)
Set-StrictMode -Version Latest; $ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot 'NativeProbe.cs')
if(Get-Process -Name echo-desktop -ErrorAction SilentlyContinue){throw 'An Echo process already exists'}
if(Test-Path $OutDir){throw 'Evidence output already exists'}
New-Item -ItemType Directory -Path $OutDir|Out-Null
$exeHash=(Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
$results=[Collections.Generic.List[object]]::new()
$expected=@{D1='echo-fixture-4999@example.net';D2='SELECT id'}[(Split-Path $Fixture -Leaf)]
if(-not $expected){throw 'Only verified D1/D2 fixtures are supported'}
function Signature([string]$Path){$r=& python (Join-Path $PSScriptRoot 'database-signature.py') $Path;if($LASTEXITCODE -ne 0){throw 'Fixture signature failed'};return ($r|Out-String).Trim()}
function Save-Ownership([Diagnostics.Process]$App,[string]$Path) {
 $snapshot=@(Get-CimInstance Win32_Process); $owned=@{}; $root=@($snapshot|Where-Object {$_.ProcessId -eq $App.Id})
 if($root.Count -ne 1){throw 'Owned root identity unavailable'}
 $owned[[int]$App.Id]=$root[0];$changed=$true
 while($changed){$changed=$false;foreach($item in $snapshot){$id=[int]$item.ProcessId;$parent=[int]$item.ParentProcessId
   if(!$owned.ContainsKey($id) -and $owned.ContainsKey($parent) -and $item.CreationDate -ge $owned[$parent].CreationDate){$owned[$id]=$item;$changed=$true}
 }}
 $processes=@($owned.Values|Sort-Object ProcessId|ForEach-Object{
   [ordered]@{pid=$_.ProcessId;parent_pid=$_.ParentProcessId;name=$_.Name;created_utc=$_.CreationDate.ToUniversalTime().ToString('o');executable=$_.ExecutablePath;command_line=$_.CommandLine}
 })
 $modules=@();$moduleError=$null
 try{$App.Refresh();$modules=@($App.Modules|ForEach-Object{[ordered]@{name=$_.ModuleName;path=$_.FileName}})}catch{$moduleError=$_.Exception.Message}
 $result=[ordered]@{schema='echo.process.ownership.snapshot.v1';root_pid=$App.Id;observed_utc=[DateTime]::UtcNow.ToString('o');processes=$processes;root_modules=$modules;module_error=$moduleError;note='Owned descendants only. Compare with collector identities; polling cannot certify short-lived unobserved children.'}
 [IO.File]::WriteAllText($Path,($result|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
}

for($i=0;$i -lt $Runs;$i++){
 $run=Join-Path $OutDir ('run-{0:d2}' -f $i);New-Item -ItemType Directory -Path $run|Out-Null;Copy-Item $Fixture (Join-Path $run 'data') -Recurse
 $db=Join-Path $run 'data/echo.sqlite3';$before=Signature $db
 $psi=[Diagnostics.ProcessStartInfo]::new();$psi.FileName=$Executable;$psi.WorkingDirectory=Split-Path $Executable;$psi.UseShellExecute=$false
 $psi.EnvironmentVariables['ECHO_DATA_DIR']=Join-Path $run 'data';$psi.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER']=Join-Path $run 'webview2';$psi.EnvironmentVariables.Remove('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS')
 $app=[Diagnostics.Process]::Start($psi);$result=[ordered]@{run=$i;variant=$Variant;mode=$Mode;dataset=(Split-Path $Fixture -Leaf);root_pid=$app.Id;exe_sha256=$exeHash;status='FAIL';error=$null;data_unchanged=$false;duration_seconds=$DurationSeconds}
 try{
  Start-Sleep -Seconds 5
  $main=@([EchoProbeNative]::Windows($app.Id)|Where-Object {$_.Title -eq 'Echo Recall' -and $_.Visible})
  if($main.Count -ne 1){throw 'Main Echo window did not become visible'}
  $uia=[Windows.Automation.AutomationElement]::FromHandle([IntPtr]$main[0].Handle)
  $elements=$uia.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  $found=$false;foreach($e in $elements){try{if(-not $e.Current.IsOffscreen -and $e.Current.Name.Contains($expected)){$found=$true}}catch{}}
  if(-not $found){throw 'Expected real fixture content is absent'}
  if($Mode -eq 'hidden'){
   [EchoProbeNative]::PostMessage([IntPtr]$main[0].Handle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null
   $limit=[Diagnostics.Stopwatch]::StartNew()
   while(@([EchoProbeNative]::Windows($app.Id)|Where-Object {$_.Visible -and $_.Title -like 'Echo*'}).Count -gt 0){if($limit.Elapsed.TotalSeconds -gt 5){throw 'Composition did not hide'};Start-Sleep -Milliseconds 10}
  }
  Save-Ownership $app (Join-Path $run 'ownership-before.json')
  Start-Sleep -Seconds 30
  $collector=Join-Path $PSScriptRoot '../../tools/perf-native/Measure-EchoProcessTree.ps1'
  & pwsh.exe -NoProfile -File $collector -RootProcessId $app.Id -OutputDirectory (Join-Path $run 'resources') -DurationSeconds $DurationSeconds
  if($LASTEXITCODE -ne 0){throw 'Resource collector failed'}
  if($app.HasExited){throw 'Product exited during measurement'}
  Save-Ownership $app (Join-Path $run 'ownership-after.json')
  $after=Signature $db
  $result.data_unchanged=($before -eq $after)
  $result.before_signature=$before; $result.after_signature=$after
  $visible=@([EchoProbeNative]::Windows($app.Id)|Where-Object {$_.Visible -and $_.Title -like 'Echo*'}).Count
  if(($Mode -eq 'hidden' -and $visible -ne 0) -or ($Mode -eq 'visible' -and $visible -ne 2)){throw 'Window visibility changed during measurement'}
  if(-not $result.data_unchanged){throw 'Fixture data changed during measurement; not valid idle evidence'}
  $result.status='MEASURED_OWNERSHIP_REVIEW_REQUIRED'
 }catch{$result.error=$_.Exception.Message;Write-Output $_}
 finally{
  if(-not $app.HasExited){$app.Kill();$app.WaitForExit(5000)|Out-Null};$app.Dispose()
  $results.Add($result)
  [IO.File]::WriteAllText((Join-Path $OutDir 'results.json'),($results.ToArray()|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
 }
 Write-Output "$Variant $Mode $i $($result.status)"
}

if (@($results | Where-Object status -eq "FAIL").Count -gt 0) { exit 1 }
