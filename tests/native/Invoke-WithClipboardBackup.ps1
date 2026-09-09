[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Script,
    [Parameter(Mandatory)][string]$Report
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1') { throw 'Explicit Windows acceptance authorization is required.' }
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Run this wrapper using pwsh -STA.' }
if (!(Test-Path -LiteralPath $Script -PathType Leaf)) { throw 'Acceptance command script is missing.' }
if (Test-Path -LiteralPath $Report) { throw 'Refusing to overwrite clipboard preservation evidence.' }
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$source = [Windows.Forms.Clipboard]::GetDataObject()
$backup = [Windows.Forms.DataObject]::new()
$owned = [Collections.Generic.List[IDisposable]]::new()
$formats = if ($null -eq $source) { @() } else { @($source.GetFormats($false)) }
# Materialize every advertised original format before allowing any mutation.
# Payloads remain in this STA process and are never logged or written to disk.
foreach ($format in $formats) {
    $value = $source.GetData($format, $false)
    if ($null -eq $value) { throw "Cannot preserve clipboard format: $format" }
    if ($value -is [IO.Stream]) {
        $copy = [IO.MemoryStream]::new()
        if ($value.CanSeek) { $value.Position = 0 }
        $value.CopyTo($copy); $copy.Position = 0
        $owned.Add($copy); $value = $copy
    }
    elseif ($value -is [Drawing.Image]) {
        $value = $value.Clone(); $owned.Add($value)
    }
    elseif ($value -is [Collections.Specialized.StringCollection]) {
        $copy = [Collections.Specialized.StringCollection]::new()
        $copy.AddRange([string[]]$value); $value = $copy
    }
    elseif ($value -is [Array]) { $value = $value.Clone() }
    elseif ($value -isnot [string] -and $value.GetType().IsValueType -eq $false) {
        throw "Cannot safely materialize clipboard format: $format"
    }
    $backup.SetData($format, $false, $value)
}
$oldReady = $env:ECHO_CLIPBOARD_BACKUP_READY
$result = [ordered]@{ schema='echo.clipboard-preservation.v1'; format_count=$formats.Count; test_exit=$null; restored=$false }
try {
    $env:ECHO_CLIPBOARD_BACKUP_READY = '1'
    $global:LASTEXITCODE = 0
    & $Script
    $result.test_exit = $LASTEXITCODE
}
catch {
    $result.test_exit = 1
    $result.error = $_.Exception.Message
    Write-Error $_ -ErrorAction Continue
}
finally {
    $env:ECHO_CLIPBOARD_BACKUP_READY = $oldReady
    for ($attempt = 0; $attempt -lt 5; $attempt++) {
        try {
            if ($formats.Count -eq 0) { [Windows.Forms.Clipboard]::Clear() }
            else { [Windows.Forms.Clipboard]::SetDataObject($backup, $true, 5, 100) }
            $result.restored = $true
            break
        }
        catch {
            if ($attempt -eq 4) { $result.restore_error = $_.Exception.Message }
            Start-Sleep -Milliseconds 100
        }
    }
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Report -Encoding utf8NoBOM
    foreach ($item in $owned) { $item.Dispose() }
}
if (!$result.restored) { throw 'Clipboard restoration failed; see preservation report.' }
if ($result.test_exit -ne 0) { exit $result.test_exit }
