# Dot-sourced by Invoke-CoverFlowAcceptance.ps1 against its owned synthetic instance.
function Settings-Category([string]$Label) {
    if((Ui 'metrics').route -ne 'settings'){Wait-SpaceReady;Invoke-Ui 'Settings'}
    Invoke-Ui $Label
    Start-Sleep -Milliseconds 50
}
function Change-Combo([string]$Label,[int]$Index) {
    Ensure-Control $Label
    Ui 'combo' @($Label,[string]$Index) | Out-Null
    Start-Sleep -Milliseconds 40
}
function Save-SettingsAndReturn {
    if((Ui 'metrics').settings.dirty){Invoke-Ui 'Save changes';Wait-Text 'Your settings are saved' | Out-Null}
    Invoke-Ui 'Cancel'
    Wait-SpaceReady
}
Check 'configured-ctrl-tab-and-reverse-do-not-steal-plain-tab' {
    Open-Space 'History'
    Settings-Category 'Keyboard & insertion'
    Change-Combo 'Space switching shortcut' 1
    Save-SettingsAndReturn
    Ui 'key' @('9') | Out-Null
    Start-Sleep -Milliseconds 80
    if((Ui 'metrics').space -ne 1){throw 'Plain Tab switched spaces in Ctrl+Tab mode.'}
    Ui 'key' @('70','ctrl') | Out-Null
    Ui 'key' @('9','ctrl') | Out-Null
    Wait-Text 'ControlType.Group | Favorites space' | Out-Null;Wait-SpaceReady
    Ui 'key' @('9','ctrl','shift') | Out-Null
    Wait-Text 'ControlType.Group | History space' | Out-Null;Wait-SpaceReady
}
Check 'no-loop-stops-at-endpoints-and-query-clear-is-explicit' {
    Settings-Category 'Spaces & content'
    Ensure-Control 'Cycle from the last space back to the first'
    Ui 'toggle' @('Cycle from the last space back to the first','false') | Out-Null
    Change-Combo 'Query on space switch' 1
    Save-SettingsAndReturn
    Ui 'key' @('9','ctrl','shift') | Out-Null;Start-Sleep -Milliseconds 80
    if((Ui 'metrics').space -ne 1){throw 'Non-looping History moved before the first space.'}
    Set-Ui 'Search clipboard history' '0013';Wait-SpaceReady
    Ui 'key' @('9','ctrl') | Out-Null;Wait-SpaceReady
    if((Ui 'metrics').space -ne 2 -or (Ui 'read' @('Search clipboard history')) -ne ''){throw 'Query clear did not apply to the destination.'}
    Ui 'key' @('9','ctrl') | Out-Null;Start-Sleep -Milliseconds 80
    if((Ui 'metrics').space -ne 2){throw 'Non-looping Favorites moved after the final space.'}
    Settings-Category 'Spaces & content'
    Ensure-Control 'Cycle from the last space back to the first'
    Ui 'toggle' @('Cycle from the last space back to the first','true') | Out-Null
    Change-Combo 'Query on space switch' 0
    Settings-Category 'Keyboard & insertion';Change-Combo 'Space switching shortcut' 0
    Save-SettingsAndReturn
}
Check 'rapid-tab-and-enter-only-resolve-the-latest-space' {
    Open-Space 'History'
    Ui 'key' @('70','ctrl') | Out-Null
    for($n=0;$n -lt 7;$n++){Ui 'key' @('9') | Out-Null}
    Ui 'key' @('13') | Out-Null
    Wait-SpaceReady
    $state=Ui 'metrics'
    if($state.space -ne 2 -or $state.requested -ne '2' -or $state.presented -ne '2' -or $state.interaction -ne '2'){
        throw 'Latest-target insertion barrier resolved a previous space.'
    }
}
Check 'reduced-motion-flat-mode-and-privacy-are-real-settings' {
    Settings-Category 'Appearance & motion'
    Change-Combo 'Motion preference' 1
    Save-SettingsAndReturn
    Ui 'key' @('70','ctrl') | Out-Null;Ui 'key' @('9') | Out-Null;Wait-SpaceReady
    $state=Ui 'metrics'
    if($state.flow_timer -or $state.settings.ui.motion -ne 'reduced'){throw 'Reduced motion still starts a deck timer.'}
    Settings-Category 'Appearance & motion';Change-Combo 'Space view' 1
    Save-SettingsAndReturn
    if((Ui 'metrics').actual -notmatch 'flat'){throw 'Flat preference was not reflected by the actual mode.'}
    Shot '08-flat-native'
    Settings-Category 'Appearance & motion';Change-Combo 'Space view' 0
    Change-Combo 'Motion preference' 0;Change-Combo 'Side-panel content' 1
    Save-SettingsAndReturn
    Ui 'key' @('70','ctrl') | Out-Null;Ui 'key' @('9') | Out-Null;Wait-SpaceReady
    $state=Ui 'metrics'
    if($state.flow_timer -or $state.settings.ui.side_content -ne 'titles_only'){throw 'Private side panels retained full animation policy.'}
    Shot '09-private-side-panels'
    Settings-Category 'Appearance & motion';Change-Combo 'Side-panel content' 0
    Save-SettingsAndReturn
}
Check 'drafted-theme-cancel-and-local-content-editor-own-tab' {
    Settings-Category 'Appearance & motion'
    Ui 'theme' @('dark') | Out-Null;Wait-Text 'Unsaved changes' | Out-Null
    Invoke-Ui 'Cancel';Wait-Text 'Discard unsaved changes?' | Out-Null
    Invoke-Ui 'Discard changes';Wait-SpaceReady
    Open-Space 'Favorites';Invoke-Ui 'New content';Wait-Text 'Favorite content' | Out-Null
    $space=(Ui 'metrics').space;Ui 'key' @('9') | Out-Null
    if((Ui 'metrics').space -ne $space){throw 'Tab escaped the content editor and changed spaces.'}
    Invoke-Ui 'Cancel';Wait-SpaceReady
}
function Create-TestSpace([string]$Name) {
    Wait-SpaceReady;Invoke-Ui 'New space';Wait-Text 'Space name' | Out-Null
    Set-Ui 'Space name' $Name;Invoke-Ui 'Save space'
    Wait-Text "ControlType.Group | $Name space" | Out-Null;Wait-SpaceReady
}
Check 'custom-space-edit-and-add-existing-use-real-ui' {
    Create-TestSpace 'Scenario Alpha'
    Invoke-Ui 'Space options';Invoke-Ui 'Edit this space';Wait-Text 'Space name' | Out-Null
    Set-Ui 'Space name' 'Scenario Team';Set-Ui 'Space icon' 'Book'
    Ui 'combo' @('Space accent','1') | Out-Null
    Invoke-Ui 'Save space';Wait-Text 'ControlType.Group | Scenario Team space' | Out-Null;Wait-SpaceReady
    Invoke-Ui 'Space options';Invoke-Ui 'Add existing saved content'
    Wait-Text 'Search saved content' | Out-Null
    Set-Ui 'Search saved content' 'flow-updated';Wait-Text 'Scenario Contact' | Out-Null
    Pick-Prefix 'Scenario Contact';Wait-Text 'flow-updated@example.test' | Out-Null;Wait-SpaceReady
    $space=@((Ui 'metrics').spaces | Where-Object title -eq 'Scenario Team')[0]
    if($space.icon -ne 'Book' -or $space.accent -ne 'blue' -or $space.count -ne 1){throw 'Saved space metadata/membership did not match the editor.'}
    Shot '10-custom-space-edited'
}
Check 'custom-space-reorder-and-system-space-protection' {
    Create-TestSpace 'Scenario Beta'
    Settings-Category 'Spaces & content'
    Ensure-Control 'Space Scenario Beta'
    Ui 'group-invoke' @('Space Scenario Beta','Move space up') | Out-Null
    Start-Sleep -Milliseconds 180
    $spaces=(Ui 'metrics').spaces
    if($spaces[0].id -ne 1 -or $spaces[1].id -ne 2 -or $spaces[2].title -ne 'Scenario Beta' -or $spaces[3].title -ne 'Scenario Team'){
        throw 'Custom ordering changed system spaces or lost a collection.'
    }
    Save-SettingsAndReturn
}
Check 'history-selection-is-preserved-per-space' {
    Open-Space 'History';Ui 'key' @('70','ctrl') | Out-Null
    Ui 'key' @('40') | Out-Null;Ui 'key' @('40') | Out-Null
    $selected=(Ui 'metrics').selection
    Ui 'key' @('9') | Out-Null;Wait-SpaceReady
    Ui 'key' @('9','','shift') | Out-Null;Wait-SpaceReady
    if((Ui 'metrics').selection -ne $selected){throw 'Switching spaces lost the History selection.'}
}
Check 'move-history-to-a-space-preserves-the-full-saved-payload' {
    Open-Space 'History';Set-Ui 'Search clipboard history' 'echo-fixture-0015'
    Wait-Text 'echo-fixture-0015@example.net' | Out-Null;Wait-SpaceReady
    Invoke-Ui 'Item options';Invoke-Ui 'Move into a space…';Invoke-Ui 'Scenario Team'
    Wait-Text 'No matches in this space' | Out-Null
    Set-Ui 'Search clipboard history' '';Open-Space 'Scenario Team'
    Set-Ui 'Search clipboard history' 'echo-fixture-0015'
    Wait-Text 'echo-fixture-0015@example.net' | Out-Null;Wait-SpaceReady
    Invoke-Ui 'Edit favorite';Wait-Text 'Favorite content' | Out-Null
    if((Ui 'read' @('Favorite content')) -ne 'echo-fixture-0015@example.net'){throw 'Moving History substituted a display-only preview.'}
    Invoke-Ui 'Cancel';Wait-SpaceReady;Set-Ui 'Search clipboard history' '';Wait-SpaceReady
}
Check 'resize-during-navigation-settles-to-an-interactive-card' {
    Ui 'key' @('70','ctrl') | Out-Null;Ui 'key' @('9') | Out-Null
    Ui 'resize' @('900','680') | Out-Null
    Wait-SpaceReady
    $state=Ui 'metrics'
    if($state.flow_timer -or $state.requested -ne $state.interaction){throw 'Resize left the deck in a noninteractive intermediate state.'}
    Shot '11-resized-card'
    Ui 'resize' @('1120','800') | Out-Null;Wait-SpaceReady
}

Check 'hidden-deck-stops-and-releases-rebuildable-textures' {
    Open-Space 'History'
    Start-Sleep -Milliseconds 1000
    Ui 'close' | Out-Null
    Start-Sleep -Milliseconds 200
    $state=Ui 'metrics'
    if($state.visible -or $state.flow_timer -or $state.preview_timer -or $state.phase -ne 'Suspended'){
        throw 'The hidden deck retained a live motion loop.'
    }
    if(!$state.settings.ui.trim_when_hidden){throw 'This scenario requires the default hidden-cache policy.'}
    Start-Sleep -Seconds 32
    $trimmed=Ui 'metrics'
    if($null -ne $trimmed.graphics -and $null -ne $trimmed.graphics.stats){
        if($trimmed.graphics.stats[0] -ne 0 -or $trimmed.graphics.stats[1] -ne 0){
            throw 'Hidden panel textures were not released after the configured timeout.'
        }
    }
    if($trimmed.flow_timer -or $trimmed.preview_timer){throw 'A hidden motion timer restarted.'}
    & $Executable --history
    if($LASTEXITCODE -ne 0){throw 'Activation after cache trimming failed.'}
    Wait-SpaceReady
    Wait-Text 'ControlType.Group | History space' | Out-Null
}

Check 'settings-motion-preview-finishes-without-a-permanent-timer' {
    Settings-Category 'Appearance & motion'
    Ensure-Control 'Play once'
    Invoke-Ui 'Play once'
    Start-Sleep -Milliseconds 500
    $state=Ui 'metrics'
    if($state.preview_timer){throw 'The one-shot settings preview did not stop.'}
    Shot '12-settings-motion-preview'
    Save-SettingsAndReturn
}
function Restart-ForGraphics([int]$Index,[string]$Expected) {
    Settings-Category 'Storage & diagnostics'
    Change-Combo 'Graphics mode' $Index
    Invoke-Ui 'Save changes'
    Wait-Text 'Restart required' | Out-Null
    Ensure-Control 'Restart Echo'
    Invoke-Ui 'Restart Echo'
    Wait-Text 'Restart Echo?' | Out-Null
    $oldId=$owned.Id
    Ui 'group-invoke' @('Restart Echo?','Restart Echo') | Out-Null
    if(!$owned.WaitForExit(10000) -or $owned.ExitCode -ne 0){throw 'The old renderer host did not exit cleanly.'}
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $replacement=$null
    do {
        $replacement=Get-CimInstance Win32_Process -Filter "ParentProcessId=$oldId" |
            Where-Object {$_.ExecutablePath -eq $Executable -and $_.ProcessId -ne $oldId} |
            Select-Object -First 1
        if(!$replacement){Start-Sleep -Milliseconds 50}
    }while(!$replacement -and $watch.ElapsedMilliseconds -lt 10000)
    if(!$replacement){throw 'The renderer restart did not create its owned replacement host.'}
    $script:owned=[Diagnostics.Process]::GetProcessById([int]$replacement.ProcessId)
    $null=$script:owned.Handle # Retain the handle before this replacement can exit.
    $script:owned.EnableRaisingEvents=$true
    [string]$script:owned.Id | Add-Content (Join-Path $EvidenceRoot 'restart-pids.txt')
    $watch.Restart()
    while(!(Ui 'exists')) {
        if($script:owned.HasExited){throw "Replacement exited before its window appeared: $($script:owned.ExitCode)"}
        if($watch.ElapsedMilliseconds -gt 15000){throw 'Replacement window did not appear.'}
        Start-Sleep -Milliseconds 50
    }
    Wait-Text 'ControlType.Group | History space' | Out-Null
    Wait-SpaceReady
    $state=Ui 'metrics'
    if($state.renderer -ne $Expected -or $state.space -ne 1){
        throw "Restart selected an incorrect renderer or replayed the previous space: $($state.renderer)"
    }
}
if($Renderer -eq 'femtovg-wgpu') {
    Check 'graphics-preference-restarts-safely-and-persists' {
        Restart-ForGraphics 1 'software'
        Restart-ForGraphics 0 'femtovg-wgpu'
    }
}
