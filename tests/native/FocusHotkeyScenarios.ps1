# Dot-sourced only by the isolated, explicitly authorized native integration harness.
function Start-FocusProcess([string]$Operation,[string[]]$Arguments) {
    $info=[Diagnostics.ProcessStartInfo]::new($FocusDriver)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true
    foreach($arg in @($Operation,$EvidenceRoot)+$Arguments){$info.ArgumentList.Add($arg)}
    return [Diagnostics.Process]::Start($info)
}
function Wait-Saved {
    Wait-Until { !(D dump).Contains('Unsaved changes') -and (D dump).Contains('Your settings are saved') } 'settings save completion' 10000|Out-Null
}
function Discard-Settings {
    Click 'Cancel'
    if((D dump).Contains('Discard unsaved changes?')){Click 'Discard changes'}
    Wait-Text 'space'
}
function Pick-Space([string]$Name) {
    Click 'Choose space'
    Wait-Text 'Choose space'
    $dump=D dump
    $line=@($dump -split "`n"|Where-Object {$_ -like "ControlType.Button | $Name*"})[0]
    if(!$line){throw "Space option not found: $Name"}
    Click (($line -split ' \| ')[1])
    Wait-Text "$Name space"
}
Check 'default-global-hotkey-and-settings-ui' {
    Assert-Binding 'Alt+V' $false
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Alt+V'){throw 'Default shortcut is not Alt+V.'}
    Wait-Text 'Enable global quick insert shortcut'
    if($NativeTest){Shot '01-keyboard-settings-default'}
    'Alt+V registered; editable field and enable checkbox present'
}
Check 'invalid-global-shortcut-cannot-save' {
    Set-Value 'Global quick insert shortcut' 'V'
    Wait-Until {(D dump) -match 'Save changes \| enabled=False'} 'invalid shortcut disables Save'|Out-Null
    Assert-Binding 'Alt+V' $false
    Discard-Settings
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Alt+V'){throw 'Discard did not restore Alt+V.'}
    Discard-Settings
    'Bare key rejected without changing active or persisted shortcut'
}
$target=Start-Target
Check 'actual-alt-v-caret-anchored-popup' {
    Open-FromTarget
    Assert-Anchored '02-caret-popup'
    if((D window-count) -ne 1){throw 'Hotkey created more than one visible Echo window.'}
    'Real system hotkey opens one card near the independent native caret'
}
Check 'repeat-hotkey-dismisses-and-restores-input' {
    Wait-Until {(F geometry @([string]$echoProcess.Id,$mainTitle)).foreground} 'Echo foreground'|Out-Null
    F hotkey @([string]$echoProcess.Id,$mainTitle,'Alt+V')|Out-Null
    Wait-Hidden
    Wait-Until {(Target-Command $target 'primary-focused') -eq 'true'} 'original input focus restored'|Out-Null
    'Second Alt+V hides Quick Insert and restores the captured input'
}
Check 'held-v-auto-repeat-does-not-toggle-repeatedly' {
    Target-Command $target 'focus-primary'|Out-Null
    F hotkey @([string]$targetProcess.Id,$target.title,'Alt+V','20')|Out-Null
    Wait-Until {D exists} 'popup after held key'|Out-Null
    Start-Sleep -Milliseconds 150
    if(!(D exists)){throw 'Repeated V keydown toggled the popup away.'}
    'MOD_NOREPEAT retained one visible popup'
}

Check 'edge-positions-fit-and-flip-without-covering-caret' {
    D close|Out-Null;Wait-Hidden
    Target-Command $target 'focus-primary'|Out-Null
    $g=F geometry @([string]$targetProcess.Id,$target.title);$w=$g.work
    $positions=@(@(($w[0]+40),($w[1]+40)),@(($w[2]-540),($w[1]+40)),@(($w[0]+40),($w[3]-190)),@(($w[2]-540),($w[3]-190)))
    $index=0
    foreach($xy in $positions){
        if(D exists){D close|Out-Null;Wait-Hidden}
        F move @([string]$targetProcess.Id,$target.title,[string]$xy[0],[string]$xy[1])|Out-Null
        Open-FromTarget
        Assert-Anchored ('03-edge-'+$index)
        $index++
    }
    'Four owned target positions; independent caret/card/work-area assertions passed'
}
Check 'space-switch-keeps-original-insertion-target' {
    Open-FromTarget
    Pick-Space 'Favorites'
    Pick-Space 'History'
    Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
    $payload='echo-perf-text-0013 '+[char]0x2014+' Reusable content, available when you need it.'
    Select-Row $payload
    Click 'Insert item'
    Wait-Hidden
    Wait-Until {(Target-Command $target 'read-primary') -eq ('a'+$payload+'c')} 'exact insertion at original caret' 10000|Out-Null
    if((Target-Command $target 'read-secondary') -ne ''){throw 'Wrong input control received text.'}
    'Exact original payload inserted between a and c after History/Favorites round-trip'
}
Check 'quick-insert-focus-loss-dismisses-without-pasting' {
    $before=Target-Command $target 'read-primary'
    Open-FromTarget
    Target-Command $target 'focus-secondary'|Out-Null
    Wait-Hidden
    if((Target-Command $target 'read-primary') -ne $before -or (Target-Command $target 'read-secondary') -ne ''){throw 'Focus loss changed target contents.'}
    'Switching to another input dismissed Echo with no insertion'
}
Check 'settings-rebind-applies-immediately-without-restart' {
    Open-KeyboardSettings
    Set-Value 'Global quick insert shortcut' 'Ctrl+Alt+J'
    Click 'Save changes';Wait-Saved
    Assert-Binding 'Alt+V' $true
    Assert-Binding 'Ctrl+Alt+J' $false
    if($NativeTest){Shot '04-keyboard-settings-rebound'}
    Open-FromTarget 'Ctrl+Alt+J'
    'Ctrl+Alt+J opened Quick Insert; Alt+V released without restarting'
}
Check 'rebound-shortcut-anchor' {Assert-Anchored '05-rebound-shortcut-popup';'Rebound shortcut uses identical caret anchoring'}
Check 'cancel-shortcut-edit-keeps-persisted-and-active-binding' {
    Open-KeyboardSettings
    Set-Value 'Global quick insert shortcut' 'Ctrl+Alt+L'
    Assert-Binding 'Ctrl+Alt+L' $true
    Discard-Settings
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Ctrl+Alt+J'){throw 'Cancel changed persisted shortcut.'}
    Assert-Binding 'Ctrl+Alt+J' $false
    'Cancel retained Ctrl+Alt+J and never reserved draft Ctrl+Alt+L'
}
Check 'occupied-shortcut-preserves-old-registration-and-settings' {
    Assert-Binding 'Ctrl+Alt+K' $true
    $script:blocker=Start-FocusProcess 'block' @('Ctrl+Alt+K')
    Wait-Until {Test-Path (Join-Path $EvidenceRoot 'blocker.ready')} 'owned conflict reservation' 5000|Out-Null
    Set-Value 'Global quick insert shortcut' 'Ctrl+Alt+K'
    Click 'Save changes'
    Wait-Until {(D dump) -match 'Cannot register|already.*use|occupied|Another app|conflict|Could not register|unable to register'} 'visible registration conflict' 10000|Out-Null
    Assert-Binding 'Ctrl+Alt+J' $false
    if($NativeTest){Shot '06-keyboard-conflict'}
    Discard-Settings
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Ctrl+Alt+J'){throw 'Conflicting shortcut was persisted.'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'stop-blocker'),'stop')
    if(!$blocker.WaitForExit(5000) -or $blocker.ExitCode -ne 0){throw 'Conflict fixture failed to release.'}
    Assert-Binding 'Ctrl+Alt+K' $true
    'Occupied shortcut rejected; previous active and saved binding preserved'
}

Check 'global-shortcut-disable-and-reenable-without-restart' {
    D toggle $mainTitle @('Enable global quick insert shortcut','false')|Out-Null
    Click 'Save changes';Wait-Saved
    Assert-Binding 'Ctrl+Alt+J' $true
    D toggle $mainTitle @('Enable global quick insert shortcut','true')|Out-Null
    Click 'Save changes';Wait-Saved
    Assert-Binding 'Ctrl+Alt+J' $false
    'Disable released the shortcut; reenable restored it immediately'
}
Check 'settings-hotkey-persists-after-clean-restart' {
    Start-ScopedProcess @('--quit')
    if(!$echoProcess.WaitForExit(10000) -or $echoProcess.ExitCode -ne 0){throw 'First instance did not quit.'}
    Assert-Binding 'Ctrl+Alt+J' $true
    Start-OwnedEcho @('--history');Wait-Until {D ready} 'restarted Echo' 30000|Out-Null
    Assert-Binding 'Ctrl+Alt+J' $false;Assert-Binding 'Alt+V' $true
    Open-KeyboardSettings
    if((D read $mainTitle @('Global quick insert shortcut')) -ne 'Ctrl+Alt+J'){throw 'Saved shortcut did not survive restart.'}
    Open-FromTarget 'Ctrl+Alt+J'
    'Same isolated database restored Ctrl+Alt+J; real shortcut still opens Quick Insert'
}
Check 'password-readonly-and-unknown-targets-do-not-insert' {
    foreach($control in @('password','readonly','unknown')) {
        $before=Target-Command $target "read-$control"
        Open-FromTarget 'Ctrl+Alt+J' $control
        Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
        $payload='echo-perf-text-0013 '+[char]0x2014+' Reusable content, available when you need it.'
        Select-Row $payload
        $dump=D dump
        if($dump -match 'Insert item \| enabled=True') {
            Click 'Insert item'
            Wait-Until {(D dump) -match 'no safe paste target|no active target|unavailable|not available|not safe'} 'safe insertion refusal' 10000|Out-Null
        }elseif($dump -notmatch 'Insert item \| enabled=False|Copy item|no.*target'){throw 'Unsafe target did not expose a clear copy-only/refusal state.'}
        if((Target-Command $target "read-$control") -ne $before){throw "Unsafe $control was changed."}
    }
    'Password/read-only/non-input fixtures were not modified'
}
Check 'held-activation-modifiers-do-not-produce-accidental-paste' {
    if(D exists){D close|Out-Null;Wait-Hidden}
    Target-Command $target 'focus-primary'|Out-Null
    $before=Target-Command $target 'read-primary'
    $script:held=Start-FocusProcess 'hold-hotkey' @([string]$targetProcess.Id,$target.title,'Ctrl+Alt+J')
    Wait-Until {Test-Path (Join-Path $EvidenceRoot 'held.ready')} 'owned held modifiers' 5000|Out-Null
    Wait-Until {D exists} 'held-hotkey popup'|Out-Null
    Query 'echo-perf-text-0013';Wait-Text 'echo-perf-text-0013'
    $payload='echo-perf-text-0013 '+[char]0x2014+' Reusable content, available when you need it.'
    Select-Row $payload;Click 'Insert item'
    Wait-Until {(D dump) -match 'modifier|release|Release|held|busy|not accept'} 'held-modifier refusal' 8000|Out-Null
    if((Target-Command $target 'read-primary') -ne $before){throw 'Insertion occurred while activation modifiers remained held.'}
    [IO.File]::WriteAllText((Join-Path $EvidenceRoot 'release-held'),'release owned modifiers')
    if(!$held.WaitForExit(5000) -or $held.ExitCode -ne 0){throw 'Held-key fixture did not finish cleanly.'}
    Start-Sleep -Milliseconds 100
    if((Target-Command $target 'read-primary') -ne $before){throw 'A failed insert was replayed after modifier release.'}
    'Modifier-busy delivery was rejected and never replayed after key release'
}
