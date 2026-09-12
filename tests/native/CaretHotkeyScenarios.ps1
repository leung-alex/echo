# Loaded by Invoke-CaretHotkeyAcceptance; all input belongs to recorded synthetic fixtures.
function Start-FocusAsync([string]$Operation,[string[]]$Arguments) {
    $info=[Diagnostics.ProcessStartInfo]::new($FocusDriver);$info.UseShellExecute=$false;$info.CreateNoWindow=$true
    foreach($arg in @($Operation,$EvidenceRoot)+$Arguments){$info.ArgumentList.Add($arg)}
    return [Diagnostics.Process]::Start($info)
}
function Save-Hotkey([string]$Chord) {
    Set-Value 'Global quick insert shortcut' $Chord
    Click 'Save changes';Assert-Binding $Chord $false
    Wait-Until {!(D dump).Contains('Unsaved changes')} 'settings commit'|Out-Null
}
function Choose-Synthetic {
    Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
    $line=@((D dump) -split "`r?`n"|Where-Object {$_ -match '^ControlType\.(ListItem|Group|Button) \|' -and $_.Contains('echo-perf-text-0013')})[0]
    if(!$line){throw 'Synthetic result row missing.'}
    $script:syntheticRow=($line -split ' \| ')[1]
    Select-Row $syntheticRow
}
function Expect-Quick([bool]$HasTarget) {
    if($NativeTest){$m=D metrics;if(!$m.quick_insert.active -or $m.quick_insert.has_target -ne $HasTarget){throw "Unexpected Quick Insert session: $($m.quick_insert|ConvertTo-Json -Compress)"}}
}
Check 'default-global-binding-and-settings' {
    Open-KeyboardSettings;Assert-Binding 'Alt+V' $false
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Alt+V'){throw 'Default is not Alt+V.'}
    if($NativeTest){Shot 'settings-global-shortcut'}
    'Alt+V is registered and represented by editable Settings controls'
}
Check 'invalid-shortcut-does-not-change-live-binding' {
    Set-Value 'Global quick insert shortcut' 'V';Wait-Text 'must include Ctrl or Alt'
    $saveLine=@((D dump) -split "`r?`n"|Where-Object {$_ -match 'Save changes \| enabled=False'})
    if(!$saveLine.Count){throw 'Invalid shortcut can still be saved.'}
    Assert-Binding 'Alt+V' $false;Set-Value 'Global quick insert shortcut' 'Alt+V'
    'plain V rejected; live Alt+V unchanged'
}
Check 'registration-conflict-keeps-old-binding-and-settings' {
    $script:blocker=Start-FocusAsync 'block' @('Ctrl+Alt+J')
    Wait-Until {Test-Path (Join-Path $EvidenceRoot 'blocker.ready')} 'conflict fixture ready'|Out-Null
    Set-Value 'Global quick insert shortcut' 'Ctrl+Alt+J';Click 'Save changes';Wait-Text 'Cannot register Ctrl+Alt+J'
    Assert-Binding 'Alt+V' $false
    if($NativeTest){$m=D metrics;if($m.settings.ui.global_hotkey -ne 'Alt+V'){throw 'Failed registration changed persisted settings.'};Shot 'settings-shortcut-conflict'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'stop-blocker'),'release')
    if(!$blocker.WaitForExit(5000) -or $blocker.ExitCode -ne 0){throw 'Conflict fixture did not release.'}
    Save-Hotkey 'Ctrl+Alt+J';Assert-Binding 'Alt+V' $true;Assert-Binding 'Ctrl+Alt+J' $false
    'conflict visible, original binding retained, retry applies new binding without restart'
}
$target=Start-Target
Check 'global-hotkey-opens-near-native-caret' {
    Open-FromTarget 'Ctrl+Alt+J';Expect-Quick $true;Assert-Anchored 'native-caret-popup'
    'native caret independently measured; visible card in work area and adjacent to caret'
}
Check 'repeat-keydown-and-toggle-are-single-window' {
    F 'hotkey' @([string]$echoProcess.Id,$mainTitle,'Ctrl+Alt+J')|Out-Null;Wait-Hidden
    Target-Command $target 'focus-primary'|Out-Null
    F 'hotkey' @([string]$targetProcess.Id,$target.title,'Ctrl+Alt+J','12')|Out-Null
    Wait-Until {D ready} 'one no-repeat popup'|Out-Null
    if((D window-count) -ne 1){throw 'Repeated keydown created extra windows or hid the popup.'}
    F 'hotkey' @([string]$echoProcess.Id,$mainTitle,'Ctrl+Alt+J')|Out-Null;Wait-Hidden
    Target-Command $target 'focus-primary'|Out-Null
    F 'hotkey' @([string]$targetProcess.Id,$target.title,'Alt+V')|Out-Null;Start-Sleep -Milliseconds 250
    if(D exists){throw 'Old Alt+V binding still activates Echo.'}
    '12 repeat keydowns yield one popup; second press hides; old binding no longer activates'
}
Check 'disabled-global-shortcut-does-not-activate' {
    Open-KeyboardSettings;D toggle $mainTitle @('Enable global quick insert shortcut','false')|Out-Null
    Click 'Save changes';Assert-Binding 'Ctrl+Alt+J' $true
    D close|Out-Null;Wait-Hidden;Target-Command $target 'focus-primary'|Out-Null
    F 'hotkey' @([string]$targetProcess.Id,$target.title,'Ctrl+Alt+J')|Out-Null;Start-Sleep -Milliseconds 250
    if(D exists){throw 'Disabled shortcut still activates.'}
    Open-KeyboardSettings;Click 'Reset to Alt+V';Click 'Save changes';Assert-Binding 'Alt+V' $false
    'disable releases the binding; reset restores Alt+V on Save'
}
Check 'settings-persist-and-first-background-popup-is-anchored' {
    Save-Hotkey 'Ctrl+Alt+J'
    Start-ScopedProcess @('--quit');if(!$echoProcess.WaitForExit(10000)){throw 'Restart shutdown failed.'}
    Assert-Binding 'Ctrl+Alt+J' $true
    Start-OwnedEcho @('--background');Assert-Binding 'Ctrl+Alt+J' $false
    if(D exists){throw '--background unexpectedly showed a manager window.'}
    Open-FromTarget 'Ctrl+Alt+J';Expect-Quick $true;Assert-Anchored 'first-background-popup'
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Ctrl+Alt+J'){throw 'Shortcut did not persist across process restart.'}
    Save-Hotkey 'Alt+V'
    'custom setting survives real process restart; first shown Quick Insert is anchored'
}
Check 'startup-registration-conflict-survives-and-retries' {
    Open-KeyboardSettings;Save-Hotkey 'Ctrl+Alt+J'
    Start-ScopedProcess @('--quit');if(!$echoProcess.WaitForExit(10000)){throw 'Conflict restart did not exit.'}
    Assert-Binding 'Ctrl+Alt+J' $true
    foreach($name in @('stop-blocker','blocker.ready')){Remove-Item (Join-Path $EvidenceRoot $name) -ErrorAction SilentlyContinue}
    $script:blocker=Start-FocusAsync 'block' @('Ctrl+Alt+J')
    $clock=[Diagnostics.Stopwatch]::StartNew();while(!(Test-Path (Join-Path $EvidenceRoot 'blocker.ready'))){if($clock.ElapsedMilliseconds -gt 5000){throw 'Startup blocker did not become ready.'};Start-Sleep -Milliseconds 20}
    Start-OwnedEcho @('--background')
    Open-KeyboardSettings;Wait-Text 'Cannot register Ctrl+Alt+J'
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Ctrl+Alt+J'){throw 'Startup conflict changed the saved preference.'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'stop-blocker'),'release')
    if(!$blocker.WaitForExit(5000)){throw 'Startup conflict fixture did not stop.'}
    Click 'Retry saved shortcut';Assert-Binding 'Ctrl+Alt+J' $false
    Save-Hotkey 'Alt+V'
    'Startup survives a reserved hotkey and retries the saved preference without losing settings'
}
Check 'native-insert-delivers-to-original-control' {
    Open-FromTarget;Choose-Synthetic;Click 'Insert item';Wait-Hidden
    Wait-Until {(Target-Command $target 'read-primary').Contains('echo-perf-text-0013')} 'original control received retained payload'|Out-Null
    'actual original native control contains the retained fixture text'
}
Check 'caret-near-bottom-flips-above' {
    Target-Command $target 'focus-secondary'|Out-Null
    $g=F 'geometry' @([string]$targetProcess.Id,$target.title)
    $windowWidth=$g.window[2]-$g.window[0];$windowHeight=$g.window[3]-$g.window[1]
    $x=[int]($g.work[2]-$windowWidth-24);$y=[int]($g.work[3]-$windowHeight-24)
    F 'move' @([string]$targetProcess.Id,$target.title,[string]$x,[string]$y)|Out-Null
    Open-FromTarget 'Alt+V' 'secondary';Assert-Anchored 'bottom-edge-popup';Expect-Quick $true
    'secondary caret and lower-screen position use the same anchoring rules without covering the input line'
}
Check 'unverified-password-and-readonly-targets-are-copy-only' {
    foreach($control in @('password','readonly','unknown')) {
        $before=Target-Command $target "read-$control"
        Open-FromTarget 'Alt+V' $control;Expect-Quick $false;Wait-Text 'no safe paste target'
        Choose-Synthetic;Click 'Copy item';Wait-Text 'Copied to clipboard'
        if((Target-Command $target "read-$control") -ne $before){throw "Copy-only path changed $control input."}
        if(!(Fixture @('clipboard','--operation','read-text')).Contains('echo-perf-text-0013')){throw 'Copy-only path failed to copy retained text.'}
    }
    'password, read-only and unknown controls never receive simulated paste; explicit Copy remains usable'
}
Check 'held-alt-preflight-preserves-clipboard-and-input' {
    if(D exists){D close|Out-Null;Wait-Hidden}
    Fixture @('clipboard','--operation','copy-text','--value','echo-modifier-sentinel')|Out-Null
    $before=Target-Command $target 'read-primary';Target-Command $target 'focus-primary'|Out-Null
    $script:held=Start-FocusAsync 'hold-hotkey' @([string]$targetProcess.Id,$target.title,'Alt+V')
    Wait-Until {Test-Path (Join-Path $EvidenceRoot 'held.ready')} 'held modifier fixture'|Out-Null
    Wait-Until {D ready} 'held modifier popup'|Out-Null
    Choose-Synthetic;Click 'Insert item';Wait-Text 'ModifierKeysBusy'
    if((Fixture @('clipboard','--operation','read-text')) -ne 'echo-modifier-sentinel'){throw 'Modifier preflight changed the clipboard.'}
    if((Target-Command $target 'read-primary') -ne $before){throw 'Held Alt still inserted.'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'release-held'),'release')
    if(!$held.WaitForExit(5000) -or $held.ExitCode -ne 0){throw 'Test-owned modifiers were not released.'}
    'held Alt rejects Insert before clipboard staging, without forcing physical modifier release'
}
Check 'changed-control-dismisses-without-paste' {
    Open-FromTarget;Choose-Synthetic
    $before=Target-Command $target 'read-secondary';Target-Command $target 'focus-secondary'|Out-Null
    Wait-Hidden
    if((Target-Command $target 'read-secondary') -ne $before){throw 'Focus-loss dismissal changed the new control.'}
    'switching external input closes Quick Insert and cancels the old session without paste'
}
Check 'anchor-setting-switches-layout-without-losing-hotkey' {
    Open-KeyboardSettings;D toggle $mainTitle @('Open Quick Insert near the text cursor','false')|Out-Null
    Click 'Save changes';Wait-Text 'Your settings are saved'
    Open-FromTarget;Expect-Quick $true
    if($NativeTest){$m=D metrics;if($m.settings.ui.caret_anchor){throw 'Anchor preference was not saved.'}}
    Open-KeyboardSettings;D toggle $mainTitle @('Open Quick Insert near the text cursor','true')|Out-Null
    Click 'Save changes';Wait-Text 'Your settings are saved'
    'caret anchoring can be disabled separately from global shortcut and safe insertion'
}
if($WpfFixture) {
    Check 'uia-only-wpf-caret-and-paste' {
        if(D exists){D close|Out-Null;Wait-Hidden}
        $title='Echo owned UIA fixture '+[guid]::NewGuid().ToString('N')
        $info=[Diagnostics.ProcessStartInfo]::new($WpfFixture);$info.UseShellExecute=$false
        $info.ArgumentList.Add($EvidenceRoot);$info.ArgumentList.Add($title)
        $script:wpfProcess=[Diagnostics.Process]::Start($info);Record-Owned $wpfProcess $title
        Wait-Until {Test-Path (Join-Path $EvidenceRoot 'wpf.ready')} 'WPF UIA-only target'|Out-Null
        F 'focus-edit' @([string]$wpfProcess.Id,$title,'Owned UIA text input')|Out-Null
        [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'wpf.command'),'suppress-native-caret')
        Wait-Until {Test-Path (Join-Path $EvidenceRoot 'wpf.no-native-caret')} 'owned fixture native-caret suppression'|Out-Null
        $uiaGeometry=F 'geometry' @([string]$wpfProcess.Id,$title)
        if($uiaGeometry.caret){throw 'UIA fallback fixture still exposes a native caret.'}
        F 'hotkey' @([string]$wpfProcess.Id,$title,'Alt+V')|Out-Null;Wait-Until {D ready} 'UIA popup'|Out-Null
        Expect-Quick $true
        $state=Get-Content (Join-Path $EvidenceRoot 'wpf.state.json') -Raw|ConvertFrom-Json
        $card=F 'card' @([string]$echoProcess.Id,$mainTitle,'History space')
        if($card[1] -lt $state.caret[3]-3 -and $card[3] -gt $state.caret[1]+3){throw 'UIA popup covers the WPF caret line.'}
        if($NativeTest){$m=D metrics;if($m.quick_insert.anchor_source -notin @('uia-caret','adjacent-character','msaa-caret')){throw "UIA caret unexpectedly fell back: $($m.quick_insert.anchor_source)"};Shot 'uia-only-caret-popup';$m|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceRoot 'uia-caret-metrics.json') -Encoding utf8NoBOM}
        Choose-Synthetic;Click 'Insert item';Wait-Hidden
        [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'wpf.command'),'read')
        Wait-Until {(Get-Content (Join-Path $EvidenceRoot 'wpf.state.json') -Raw|ConvertFrom-Json).text.Contains('echo-perf-text-0013')} 'UIA-only actual pasted content'|Out-Null
        [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'wpf.stop'),'stop');if(!$wpfProcess.WaitForExit(5000)){throw 'WPF fixture shutdown failed.'}
        'a real WPF TextBox without an Edit HWND is anchored via accessibility and receives Ctrl+V safely'
    }
}
Check 'closed-original-window-is-never-replaced-by-current-focus' {
    Open-FromTarget;Choose-Synthetic
    Target-Command $target 'shutdown'|Out-Null;if(!$targetProcess.WaitForExit(5000)){throw 'Target fixture did not close.'}
    Click 'Insert item';Wait-Text 'OriginalWindowUnavailable'
    'destroyed original target rejected; Echo does not paste into whichever window is now focused'
}
