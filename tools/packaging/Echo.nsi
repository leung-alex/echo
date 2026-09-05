Unicode true
!include "LogicLib.nsh"
!include "FileFunc.nsh"
Name "Echo"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Echo"
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Echo"
VIAddVersionKey "LegalCopyright" "Echo contributors"
VIAddVersionKey "FileDescription" "Echo native Slint installer"
VIAddVersionKey "FileVersion" "${VERSION}"
Var integration
Function .onInit
  SetShellVarContext current
  StrCpy $integration "1"
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/NOINTEGRATION=" $1
  ${IfNot} ${Errors}
    ${If} $1 == "1"
      StrCpy $integration "0"
    ${EndIf}
  ${EndIf}
FunctionEnd
Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles
Section "Install"
  !include "${INSTALL_FILES}"
  WriteINIStr "$INSTDIR\install-mode.ini" "install" "integration" "$integration"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ${If} $integration == "1"
    CreateShortcut "$SMPROGRAMS\Echo.lnk" "$INSTDIR\Echo.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "DisplayName" "Echo"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "NoRepair" 1
  ${EndIf}
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  ReadINIStr $integration "$INSTDIR\install-mode.ini" "install" "integration"
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation"
  ${If} $integration == "1"
  ${AndIf} $0 == "$INSTDIR"
    Delete "$SMPROGRAMS\Echo.lnk"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative"
  ${EndIf}
  !include "${UNINSTALL_FILES}"
  Delete "$INSTDIR\install-mode.ini"
  Delete "$INSTDIR\Uninstall.exe"
  ; No recursive delete: unknown files and the separate Echo data directory survive.
  RMDir "$INSTDIR"
SectionEnd
