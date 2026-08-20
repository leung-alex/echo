$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

Push-Location $repo
try {
  & .\echo.cmd format --check
  if ($LASTEXITCODE -ne 0) {
    throw "canonical format check failed"
  }
} finally {
  Pop-Location
}
