[CmdletBinding()]
param([Parameter(Mandatory)][string]$Evidence)
$ErrorActionPreference = 'Stop'
if ($env:ECHO_WINDOWS_ACCEPTANCE -ne '1' -or $env:ECHO_CLIPBOARD_BACKUP_READY -ne '1') {
    throw 'Run through Invoke-WithClipboardBackup.ps1 with explicit native acceptance enabled.'
}
$Evidence = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath $Evidence) { throw 'Use a fresh evidence directory.' }
New-Item -ItemType Directory -Path $Evidence | Out-Null
$previous = $env:ECHO_DATA_DIR
try {
    $env:ECHO_DATA_DIR = $Evidence
    cargo test -p echo-windows --test terminal_plain_paste --locked -- --ignored --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'Terminal plain paste acceptance failed.' }
} finally { $env:ECHO_DATA_DIR = $previous }
