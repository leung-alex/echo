Unicode true
SetCompressor /SOLID lzma
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"
!include "ShortcutOwnership.nsh"
!ifndef APP_ICON
  !error "APP_ICON must point to the Echo application icon"
!endif
Icon "${APP_ICON}"
UninstallIcon "${APP_ICON}"
WindowIcon on
Name "Echo"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Echo"
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Echo"
VIAddVersionKey "LegalCopyright" "Echo contributors"
VIAddVersionKey "FileDescription" "Echo native Slint installer"
VIAddVersionKey "FileVersion" "${VERSION}"
Var integration
Var desktopChoice
Var desktopOverride
Var desktopCheckbox
Var desktopOwned
Var startMenuOwned
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
  StrCpy $desktopOverride ""
  ClearErrors
  ${GetOptions} $0 "/DESKTOPSHORTCUT=" $1
  ${IfNot} ${Errors}
    ${If} $1 == "0"
    ${OrIf} $1 == "1"
      StrCpy $desktopOverride $1
    ${Else}
      MessageBox MB_OK|MB_ICONSTOP "/DESKTOPSHORTCUT must be 0 or 1." /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
FunctionEnd

Function ReadDesktopChoice
  ReadINIStr $desktopChoice "$INSTDIR\install-mode.ini" "shortcuts" "desktop_choice"
  ${If} $desktopChoice != "0"
  ${AndIf} $desktopChoice != "1"
    StrCpy $desktopChoice "1"
  ${EndIf}
  ${If} $desktopOverride != ""
    StrCpy $desktopChoice $desktopOverride
  ${EndIf}
FunctionEnd

Function DesktopOptionsCreate
  ${If} $integration != "1"
    Abort
  ${EndIf}
  Call ReadDesktopChoice
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateCheckbox} 0 20u 100% 12u "Create desktop shortcut"
  Pop $desktopCheckbox
  ${NSD_SetState} $desktopCheckbox $desktopChoice
  nsDialogs::Show
FunctionEnd

Function DesktopOptionsLeave
  ${NSD_GetState} $desktopCheckbox $desktopOverride
FunctionEnd

Page directory
Page custom DesktopOptionsCreate DesktopOptionsLeave
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles
Section "Install"
  ; Read upgrade preferences before writing this installation's metadata.
  Call ReadDesktopChoice
  ReadINIStr $desktopOwned "$INSTDIR\install-mode.ini" "shortcuts" "desktop_owned"
  !include "${INSTALL_FILES}"
  WriteINIStr "$INSTDIR\install-mode.ini" "install" "integration" "$integration"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ${If} $integration == "1"
    WriteINIStr "$INSTDIR\install-mode.ini" "shortcuts" "desktop_choice" "$desktopChoice"
    !insertmacro EchoCreateOwnedShortcut "$SMPROGRAMS\Echo.lnk" "start_menu_owned"
    ${If} $desktopChoice == "1"
      !insertmacro EchoCreateOwnedShortcut "$DESKTOP\Echo.lnk" "desktop_owned"
    ${Else}
      !insertmacro EchoDeleteOwnedShortcut "" "$DESKTOP\Echo.lnk" $desktopOwned
      ; Retain ownership on a delete failure so a later uninstall can retry.
      Push "$DESKTOP\Echo.lnk"
      Call ShortcutTargetsThisInstall
      Pop $0
      ${If} $desktopOwned != "1"
        StrCpy $0 "0"
      ${EndIf}
      WriteINIStr "$INSTDIR\install-mode.ini" "shortcuts" "desktop_owned" "$0"
    ${EndIf}
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation"
    ${If} $0 == ""
    ${OrIf} $0 == "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "DisplayName" "Echo"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "DisplayIcon" '$\"$INSTDIR\Echo.exe$\",0'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "NoRepair" 1
    ${Else}
      DetailPrint "Preserved application registration belonging to $0"
    ${EndIf}
  ${EndIf}
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  ReadINIStr $integration "$INSTDIR\install-mode.ini" "install" "integration"
  ${If} $integration == "1"
    ReadINIStr $startMenuOwned "$INSTDIR\install-mode.ini" "shortcuts" "start_menu_owned"
    ReadINIStr $desktopOwned "$INSTDIR\install-mode.ini" "shortcuts" "desktop_owned"
    ; Legacy installers recorded only integration + InstallLocation.
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation"
    ${If} $startMenuOwned == ""
    ${AndIf} $0 == "$INSTDIR"
      StrCpy $startMenuOwned "1"
    ${EndIf}
    !insertmacro EchoDeleteOwnedShortcut "un." "$SMPROGRAMS\Echo.lnk" $startMenuOwned
    !insertmacro EchoDeleteOwnedShortcut "un." "$DESKTOP\Echo.lnk" $desktopOwned
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative" "InstallLocation"
    ${If} $0 == "$INSTDIR"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Echo"
      DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EchoNative"
    ${EndIf}
  ${EndIf}
  !include "${UNINSTALL_FILES}"
  Delete "$INSTDIR\install-mode.ini"
  Delete "$INSTDIR\Uninstall.exe"
  ; No recursive delete: unknown files and the separate Echo data directory survive.
  RMDir "$INSTDIR"
SectionEnd
