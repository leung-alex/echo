#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Fixture,
    [Parameter(Mandatory)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:ECHO_WINDOWS_ACCEPTANCE -ne '1' -or $env:ECHO_CLIPBOARD_BACKUP_READY -ne '1' -or !(Get-Command Register-EchoCaptureProcess -ErrorAction SilentlyContinue)) {throw 'Run in the authorized STA clipboard preservation wrapper'}
if(Test-Path -LiteralPath $OutputDirectory){throw 'Evidence directory must be new'}
if(@(Get-Process echo-desktop,Echo -ErrorAction SilentlyContinue).Count){throw 'Another Echo process owns the desktop'}
$marker=Get-Content (Join-Path $Fixture 'synthetic-fixture.json') -Raw|ConvertFrom-Json
if(!$marker.synthetic -or $marker.capture_enabled -or $marker.history -ne 2000 -or $marker.favorites -ne 200){throw 'Requires the approved isolated synthetic fixture'}
New-Item -ItemType Directory -Path $OutputDirectory|Out-Null
$OutputDirectory=(Resolve-Path $OutputDirectory).Path;$Executable=(Resolve-Path $Executable).Path
Copy-Item -LiteralPath $Fixture -Destination (Join-Path $OutputDirectory 'data') -Recurse
$db=Join-Path $OutputDirectory 'data/echo.sqlite3'
python (Join-Path $PSScriptRoot 'database-signature.py') $db|Set-Content (Join-Path $OutputDirectory 'original-before.json')
if($LASTEXITCODE){throw 'Original payload verification failed'}
python -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute("UPDATE clipboard_settings SET history_enabled=1"); c.commit()' $db
if($LASTEXITCODE){throw 'Cannot enable isolated capture'}
$marker.capture_enabled=$true;$marker|ConvertTo-Json|Set-Content (Join-Path $OutputDirectory 'data/synthetic-fixture.json')
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Path (Join-Path $PSScriptRoot 'NativeProbe.cs')
# Seed before capture starts, so a restored personal clipboard is never ingested.
[Windows.Forms.Clipboard]::SetText('echo-software-capture-bootstrap')
$psi=[Diagnostics.ProcessStartInfo]::new($Executable);$psi.UseShellExecute=$false;$psi.Arguments='--history'
$psi.Environment['ECHO_DATA_DIR']=Join-Path $OutputDirectory 'data'
$psi.Environment['ECHO_WINDOWS_ACCEPTANCE']='1';$psi.Environment['ECHO_MEMORY_TRACE_DIR']=$OutputDirectory
foreach($key in @('ECHO_NATIVE_TEST_ROOT','ECHO_RENDERER','SLINT_BACKEND')){$psi.Environment.Remove($key)|Out-Null}
$product=$null
$result=[ordered]@{status='NOT_RUN';sha256=(Get-FileHash $Executable).Hash;scope='Three controlled captures after deep display reclamation; not a duration soak'}
try{
    $product=[Diagnostics.Process]::Start($psi);Register-EchoCaptureProcess $product
    $result.pid=$product.Id;$result.created_ticks=$product.StartTime.ToUniversalTime().Ticks
    $watch=[Diagnostics.Stopwatch]::StartNew()
    do{$windows=@([EchoProbeNative]::Windows($product.Id)|Where-Object {$_.Visible -and $_.Title -eq 'Echo Recall'});if($windows.Count -eq 1){break};Start-Sleep -Milliseconds 50}while(!$product.HasExited -and $watch.Elapsed.TotalSeconds -lt 15)
    if($windows.Count -ne 1){throw 'Owned window did not become visible'}
    Start-Sleep -Seconds 1
    [EchoProbeNative]::PostMessage([IntPtr]$windows[0].Handle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null
    for($i=0;$i -lt 35;$i++){Start-Sleep -Seconds 1;if($product.HasExited){throw 'Root exited while hidden'}}
    $trace=@(Get-Content (Join-Path $OutputDirectory "lifecycle-$($product.Id).jsonl")|ForEach-Object {$_|ConvertFrom-Json})
    $hide=@($trace|Where-Object state -eq 'hidden_warm')[-1];$ack=@($trace|Where-Object state -eq 'hidden_reclaimed')[-1]
    $delay=([decimal]$ack.utc_ns-[decimal]$hide.utc_ns)/1e9
    if($delay -lt 30 -or $delay -gt 35 -or !$ack.details.worker_cache_acknowledged -or $ack.details.software_frame_bytes -ne 0){throw 'Deep display reclaim did not complete before capture'}
    $result.reclaim_seconds=$delay
    for($i=0;$i -lt 3;$i++){[Windows.Forms.Clipboard]::SetText("echo-software-after-reclaim-$i");Start-Sleep -Milliseconds 750}
    $verify=@'
import sqlite3,sys,json
c=sqlite3.connect(sys.argv[1]); texts={r[0] for r in c.execute('SELECT searchable_text FROM clipboard_entries')}
missing=[i for i in range(3) if f'echo-software-after-reclaim-{i}' not in texts]
h=c.execute('SELECT COUNT(*) FROM clipboard_entries').fetchone()[0]; s=c.execute('SELECT COUNT(*) FROM saved_items').fetchone()[0]
print(json.dumps(dict(capture_count=3-len(missing),history_count=h,saved_count=s)))
sys.exit(bool(missing) or h!=2000 or s!=200)
'@
    $watch.Restart()
    do{$captured=& python -c $verify $db;$ok=$LASTEXITCODE -eq 0;if($ok){break};Start-Sleep -Milliseconds 100}while($watch.Elapsed.TotalSeconds -lt 10)
    $captured|Set-Content (Join-Path $OutputDirectory 'captured.json')
    if(!$ok){throw 'Captured content, History cap or Saved Items independence failed'}
    $result.status='PASS'
}catch{$result.status='FAIL';$result.error=$_.ToString()}
finally{
    if($null -ne $product -and !$product.HasExited){
        $psi.Arguments='--quit';$quit=[Diagnostics.Process]::Start($psi)
        if(!$quit.WaitForExit(10000) -or !$product.WaitForExit(10000) -or $product.ExitCode -ne 0){$result.status='FAIL';$result.error='Graceful capture shutdown failed'}
        $quit.Dispose()
    }
    python (Join-Path $PSScriptRoot 'database-signature.py') $db|Set-Content (Join-Path $OutputDirectory 'original-after.json')
    if($LASTEXITCODE){$result.status='FAIL';$result.error='Retained original blob verification failed'}
    $before=Get-Content (Join-Path $OutputDirectory 'original-before.json') -Raw|ConvertFrom-Json
    $after=Get-Content (Join-Path $OutputDirectory 'original-after.json') -Raw|ConvertFrom-Json
    foreach($key in @('saved_items','saved_item_representations','tags','saved_item_tags','spaces','space_memberships')){
        if(($before.$key|ConvertTo-Json -Compress) -ne ($after.$key|ConvertTo-Json -Compress)){$result.status='FAIL';$result.error="Saved content changed during History capture: $key"}
    }
    $result|ConvertTo-Json -Depth 12|Set-Content (Join-Path $OutputDirectory 'result.json');$result|ConvertTo-Json
}
if($result.status -ne 'PASS'){exit 1}
