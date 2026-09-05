#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Root,[Parameter(Mandatory)][string]$Executable,[string]$OutputDirectory='')
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$Root=[IO.Path]::GetFullPath($Root);$Executable=[IO.Path]::GetFullPath($Executable)
. (Join-Path $Root 'tools/perf-native/Common.ps1')
Assert-EchoNoReparsePoint $Executable
if(!(Test-Path -LiteralPath $Executable -PathType Leaf)){throw 'Release executable missing'}
$versionMatch=[regex]::Match((Get-Content -Raw -LiteralPath (Join-Path $Root 'Cargo.toml')),'(?ms)\[workspace\.package\]\s*.*?^version\s*=\s*"([0-9A-Za-z.+-]+)"')
if(!$versionMatch.Success){throw 'Workspace version missing'}
$version=$versionMatch.Groups[1].Value
foreach($license in Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'licenses') -File){
    if($license.BaseName -notmatch '^[a-f0-9]{64}$' -or (Get-FileHash -LiteralPath $license.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $license.BaseName){throw "Upstream license bytes changed: $($license.Name)"}
}

if(!$OutputDirectory){
    $parent=Join-Path $Root ('target/echo-package/'+$version)
    Assert-EchoNoReparsePoint $parent
    New-Item -ItemType Directory -Path $parent -Force|Out-Null
    $OutputDirectory=Join-Path $parent ([DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff')+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
}
$output=New-EchoEvidenceDirectory $OutputDirectory
$runtime=Join-Path $output 'portable';New-Item -ItemType Directory $runtime|Out-Null
Copy-Item -LiteralPath $Executable -Destination (Join-Path $runtime 'Echo.exe')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'THIRD-PARTY-NOTICES.txt') -Destination $runtime
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'resolved-licenses.json') -Destination $runtime
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'licenses') -Destination $runtime -Recurse
$files=@(Get-ChildItem -LiteralPath $runtime -Recurse -File|Sort-Object FullName)
$inventory=@($files|ForEach-Object{
    Assert-EchoNoReparsePoint $_.FullName
    [ordered]@{path=[IO.Path]::GetRelativePath($runtime,$_.FullName).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
})
$commit=(& git -C $Root rev-parse HEAD|Out-String).Trim();if($LASTEXITCODE){throw 'Cannot identify source commit'}
$dirty=@(& git -C $Root status --porcelain);if($LASTEXITCODE){throw 'Cannot identify source status'}
Write-EchoJsonNew (Join-Path $runtime 'echo-files.json') ([ordered]@{schema='echo.installed-files.v1';app_id='Echo.Native';version=$version;commit=$commit;source_dirty=($dirty.Count -gt 0);files=$inventory})
$files=@(Get-ChildItem -LiteralPath $runtime -Recurse -File|Sort-Object FullName)
$zipPath=Join-Path $output "Echo-$version-windows-portable.zip"
$stream=[IO.File]::Open($zipPath,[IO.FileMode]::CreateNew)
$zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Create,$true)
try{
    foreach($file in $files){
        $relative=[IO.Path]::GetRelativePath($runtime,$file.FullName).Replace('\','/')
        $entry=$zip.CreateEntry($relative,[IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime=[DateTimeOffset]::new(2000,1,1,0,0,0,[TimeSpan]::Zero)
        $source=[IO.File]::OpenRead($file.FullName);$destination=$entry.Open()
        try{$source.CopyTo($destination)}finally{$source.Dispose();$destination.Dispose()}
    }
}finally{$zip.Dispose();$stream.Dispose()}
$artifacts=[Collections.Generic.List[string]]::new();$artifacts.Add($zipPath)
$compiler=$env:ECHO_NSIS_EXE
if(!$compiler){$found=Get-Command makensis.exe -ErrorAction SilentlyContinue;if($found){$compiler=$found.Source}}
if(!$compiler){foreach($candidate in @('C:\Program Files (x86)\NSIS\makensis.exe','C:\Program Files\NSIS\makensis.exe')){if(Test-Path -LiteralPath $candidate){$compiler=$candidate;break}}}
$installerStatus='NOT_BUILT: no independent NSIS compiler configured; portable application is available'
if($compiler){
    Assert-EchoNoReparsePoint $compiler
    if(!(Test-Path -LiteralPath $compiler -PathType Leaf)){throw 'ECHO_NSIS_EXE is invalid'}
    function Q([string]$Text){return $Text.Replace('$','$$').Replace('"','$\"')}
    $install=[Collections.Generic.List[string]]::new();$uninstall=[Collections.Generic.List[string]]::new()
    foreach($file in $files){
        $relative=[IO.Path]::GetRelativePath($runtime,$file.FullName)
        $directory=[IO.Path]::GetDirectoryName($relative)
        $install.Add('SetOutPath "$INSTDIR'+$(if($directory){'\'+(Q $directory)}else{''})+'"')
        $install.Add('File "/oname='+(Q $file.Name)+'" "'+(Q $file.FullName)+'"')
        $uninstall.Add('Delete "$INSTDIR\'+(Q $relative)+'"')
    }
    foreach($directory in @(Get-ChildItem -LiteralPath $runtime -Directory -Recurse|Sort-Object {$_.FullName.Length} -Descending)){
        $uninstall.Add('RMDir "$INSTDIR\'+(Q ([IO.Path]::GetRelativePath($runtime,$directory.FullName)))+'"')
    }
    $installFile=Join-Path $output 'install-files.nsh';$uninstallFile=Join-Path $output 'uninstall-files.nsh'
    [IO.File]::WriteAllLines($installFile,$install,[Text.UTF8Encoding]::new($true));[IO.File]::WriteAllLines($uninstallFile,$uninstall,[Text.UTF8Encoding]::new($true))
    $setup=Join-Path $output "Echo-$version-windows-setup.exe"
    & $compiler '/V2' "/DVERSION=$version" "/DOUT=$setup" "/DINSTALL_FILES=$installFile" "/DUNINSTALL_FILES=$uninstallFile" (Join-Path $PSScriptRoot 'Echo.nsi') *> (Join-Path $output 'nsis.log')
    if($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $setup)){throw 'Independent NSIS packaging failed; see nsis.log'}
    $artifacts.Add($setup);$installerStatus='BUILT: per-user independent NSIS installer'
}
$hashes=@($artifacts|ForEach-Object{[ordered]@{path=$_.Substring($output.Length+1);bytes=(Get-Item -LiteralPath $_).Length;sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()}})
Write-EchoJsonNew (Join-Path $output 'package.json') ([ordered]@{schema='echo.native.package.v1';version=$version;commit=$commit;source_dirty=($dirty.Count -gt 0);installer=$installerStatus;artifacts=$hashes;runtime_bytes=($files|Measure-Object -Property Length -Sum).Sum;runtime='portable';shared_webview_runtime='not required; never uninstalled';user_data='preserved outside installation directory';installation_test='NOT_RUN'})
$hashes|ForEach-Object{"$($_.sha256)  $($_.path)"}|Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding utf8NoBOM
Write-Output "Package directory: $output"
Write-Output $installerStatus
