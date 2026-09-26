#requires -Version 7.0
<##
One-shot WeChat evidence collector. The default path is inventory-only: it records
owned EXE/DLL files, Git state, and environment, then writes WC01-WC12 as NOT_RUN.
It never sends keys, clicks, clipboard content, or UI Automation commands.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$RepositoryRoot,
  [Parameter(Mandatory)][string]$EvidenceRoot,
  [string]$WeChatExecutable,
  [switch]$LaunchWeChat
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'A real Windows host is required.' }
$repo = [IO.Path]::GetFullPath($RepositoryRoot)
$out = [IO.Path]::GetFullPath($EvidenceRoot)
if (-not (Test-Path -LiteralPath (Join-Path $repo 'echo.cmd') -PathType Leaf)) { throw 'RepositoryRoot must contain echo.cmd.' }
if (Test-Path -LiteralPath $out) { throw 'EvidenceRoot must be new; existing evidence is never overwritten.' }
New-Item -ItemType Directory -Force -Path $out | Out-Null
function Write-NewJson([string]$Path, [object]$Value) {
  if (Test-Path -LiteralPath $Path) { throw "Refusing to overwrite $Path" }
  $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
}
function Read-Git([string[]]$GitArgs) { $v = & git -C $repo @GitArgs 2>&1; if ($LASTEXITCODE -ne 0) { throw "git failed: $($GitArgs -join ' ')" }; ($v | Out-String).Trim() }
$head = Read-Git @('rev-parse','HEAD')
$branch = Read-Git @('branch','--show-current')
$status = Read-Git @('status','--porcelain','--untracked-files=all')
$existing = @(Get-CimInstance Win32_Process -Filter "Name = 'WeChat.exe' OR Name = 'Weixin.exe'")
$wechatBlocked = $existing.Count -gt 0
$launched = $null
$launchRefused = $false
try {
  if ($LaunchWeChat) {
    if (-not $WeChatExecutable) { throw '-LaunchWeChat requires -WeChatExecutable.' }
    if ($wechatBlocked) {
      $launchRefused = $true
    } else {
      $exe = [IO.Path]::GetFullPath($WeChatExecutable)
      if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "WeChat executable not found: $exe" }
      $launched = Start-Process -FilePath $exe -PassThru
      Start-Sleep -Milliseconds 1200
    }
  }
  $files = @(Get-ChildItem -LiteralPath $repo -Recurse -File -Include *.exe,*.dll -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\.local\\' } |
    ForEach-Object { [ordered]@{ path=$_.FullName.Substring($repo.Length).TrimStart('\\'); bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() } })
  $envFile = Join-Path $out 'environment.txt'
  Get-ComputerInfo -Property WindowsProductName,WindowsVersion,OsBuildNumber,OsArchitecture,CsName | Out-File -LiteralPath $envFile -Encoding utf8
  Get-Command git,rustc,cargo,go,pwsh | Select-Object Name,Source,Version | Out-File -LiteralPath $envFile -Append -Encoding utf8
  $blockReason = if ($wechatBlocked) { 'An existing WeChat process is present; the collector refused to attach, close, or launch another instance.' } elseif ($launchRefused) { 'Launch was refused because an existing WeChat process was present.' } else { $null }
  $caseReason = if ($blockReason) { $blockReason } else { 'Real WeChat input/UI automation is intentionally excluded from the default collector.' }
  Write-NewJson (Join-Path $out 'source.json') ([ordered]@{ schema='echo.wechat.source.v1'; captured_at_utc=[DateTime]::UtcNow.ToString('o'); repository=$repo; branch=$branch; head=$head; status=$status; files=$files; launched=([bool]$launched); existing_process_count=$existing.Count; blocked_reason=$blockReason; note='Inventory only; no real WeChat input or clipboard operation was performed.' })
  $cases = 1..12 | ForEach-Object { [ordered]@{ id=('WC{0:D2}' -f $_); status='NOT_RUN'; reason=$caseReason } }
  Write-NewJson (Join-Path $out 'wc-cases.json') ([ordered]@{ schema='echo.wechat.cases.v1'; cases=$cases; compatibility_boundaries=@([ordered]@{ target='ChatGPT'; status='REJECTED'; reason='No compatibility claim or input automation is permitted.' },[ordered]@{ target='Codex'; status='REJECTED'; reason='The collector must never inject into the Codex task/composer.' }) })
} finally {
  if ($launched -and -not $launched.HasExited) { Stop-Process -Id $launched.Id -Force; $launched.WaitForExit() }
}
Write-Output "WeChat evidence written to $out; WC01-WC12 remain NOT_RUN."
