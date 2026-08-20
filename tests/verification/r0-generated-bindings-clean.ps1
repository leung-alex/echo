$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$previousAcceptance = [Environment]::GetEnvironmentVariable(
  "ECHO_WINDOWS_ACCEPTANCE",
  "Process"
)

Push-Location $repo
try {
  Remove-Item Env:ECHO_WINDOWS_ACCEPTANCE -ErrorAction SilentlyContinue
  & .\echo.cmd bindings --check
  if ($LASTEXITCODE -ne 0) {
    throw "checked-in generated bindings are not fresh"
  }
} finally {
  if ($null -eq $previousAcceptance) {
    Remove-Item Env:ECHO_WINDOWS_ACCEPTANCE -ErrorAction SilentlyContinue
  } else {
    $env:ECHO_WINDOWS_ACCEPTANCE = $previousAcceptance
  }
  Pop-Location
}
