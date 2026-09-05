# Shared evidence helpers. Dot-sourcing this file performs no collection or writes.
Set-StrictMode -Version Latest

function Assert-EchoNoReparsePoint([Parameter(Mandatory)][string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -Force -LiteralPath $current -ErrorAction Stop
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Reparse-point paths are not permitted: $current"
            }
        }
        $parent = [IO.Path]::GetDirectoryName($current)
        if ($parent -eq $current) { break }
        $current = $parent
    }
}

function Assert-EchoOutsideRoot([string]$Root, [string]$Output) {
    Assert-EchoNoReparsePoint $Root
    Assert-EchoNoReparsePoint $Output
    $relative = [IO.Path]::GetRelativePath([IO.Path]::GetFullPath($Root), [IO.Path]::GetFullPath($Output))
    if ($relative -eq '.' -or (-not [IO.Path]::IsPathRooted($relative) -and $relative -ne '..' -and -not $relative.StartsWith("..$([IO.Path]::DirectorySeparatorChar)"))) {
        throw 'Evidence output must be outside the input directory.'
    }
}

function New-EchoEvidenceDirectory([Parameter(Mandatory)][string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    Assert-EchoNoReparsePoint $full
    if (Test-Path -LiteralPath $full) { throw 'Use a new evidence directory; existing evidence is never overwritten.' }
    if (-not (Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($full)) -PathType Container)) {
        throw 'Create the parent evidence directory explicitly before collecting.'
    }
    # CreateNew reserves this path between cooperating collectors. The empty
    # marker remains after completion so failed or partial evidence is not reused.
    $claim = [IO.File]::Open(($full + '.claim'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        if (Test-Path -LiteralPath $full) { throw 'Evidence directory appeared during reservation.' }
        [IO.Directory]::CreateDirectory($full) | Out-Null
    } finally { $claim.Dispose() }
    return $full
}

function Write-EchoJsonNew([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)]$Value) {
    Assert-EchoNoReparsePoint $Path
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 16) + "`n")
    $stream = [IO.File]::Open([IO.Path]::GetFullPath($Path), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) }
    finally { $stream.Dispose() }
}

function Get-EchoToolVersion([string]$Name, [string[]]$Arguments) {
    if (-not (Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue)) { return 'NOT_FOUND' }
    try {
        $text = ((& $Name @Arguments 2>&1) | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { return "ERROR_EXIT_$LASTEXITCODE" }
        return $text
    } catch { return "ERROR: $($_.Exception.GetType().Name)" }
}
