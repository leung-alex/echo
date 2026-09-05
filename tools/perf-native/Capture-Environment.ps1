#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputFile,
    [string]$BinaryPath,
    [string]$Variant = 'unassigned'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This collector must run on Windows.' }
. (Join-Path $PSScriptRoot 'Common.ps1')
$os = Get-CimInstance Win32_OperatingSystem
$machine = Get-CimInstance Win32_ComputerSystem
$cpu = @(Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors)
$gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, DriverDate, CurrentHorizontalResolution, CurrentVerticalResolution, CurrentRefreshRate)
$binary = $null
if ($BinaryPath) {
    Assert-EchoNoReparsePoint $BinaryPath
    $file = Get-Item -LiteralPath $BinaryPath
    if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $file.PSIsContainer) { throw 'Binary must be a regular file.' }
    $binary = @{ name=$file.Name; bytes=$file.Length; sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$report = [ordered]@{
    schema = 'echo.environment.v1'; captured_at_utc = [DateTime]::UtcNow.ToString('o'); variant = $Variant
    os = @{ caption=$os.Caption; version=$os.Version; build=$os.BuildNumber; architecture=$os.OSArchitecture }
    cpu = $cpu; logical_processors=$machine.NumberOfLogicalProcessors; physical_memory_bytes=$machine.TotalPhysicalMemory
    gpu = $gpu; power_scheme = (Get-EchoToolVersion 'powercfg' @('/getactivescheme'))
    tools = @{ powershell=$PSVersionTable.PSVersion.ToString(); rustc=(Get-EchoToolVersion 'rustc' @('-Vv')); cargo=(Get-EchoToolVersion 'cargo' @('-V')); go=(Get-EchoToolVersion 'go' @('version')); nsis=(Get-EchoToolVersion 'makensis' @('/VERSION')) }
    binary=$binary
    manual_fields_required=@('WebView2 version for baseline','Slint/render backend for candidate','MSVC/Windows SDK','Per-window DPI/monitor/refresh rate','Mica actually visible','Theme/wallpaper','Power supply/background load','Signed/unsigned','Fixture and comparison profile IDs')
    note='Environment inventory only; manual fields must be completed before matched-run comparison. GPU RAM counters are NOT collected.'
}
$full = [IO.Path]::GetFullPath($OutputFile)
if ($BinaryPath -and $full -eq [IO.Path]::GetFullPath($BinaryPath)) { throw 'Output cannot overwrite binary.' }
Write-EchoJsonNew $full $report
Write-Output $full
