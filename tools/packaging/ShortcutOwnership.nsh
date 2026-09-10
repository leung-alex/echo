!ifndef ECHO_SHORTCUT_OWNERSHIP
!define ECHO_SHORTCUT_OWNERSHIP

; Read the stored target without resolving/launching the link. Fail closed for
; unreadable links and reparse points. Uses only NSIS's bundled System plug-in.
; Input: link path on the stack. Output: 1 iff it targets this installation.
!macro EchoShortcutTargetFunction PREFIX
Function ${PREFIX}ShortcutTargetsThisInstall
  System::Store S
  Pop $9
  StrCpy $7 "0"
  System::Call 'kernel32::GetFileAttributesW(w r9) i.r0'
  ${If} $0 != -1
    IntOp $0 $0 & 0x400
    ${If} $0 == 0
      System::Call 'ole32::CoInitializeEx(p 0, i 2) i.r5'
      System::Call "ole32::CoCreateInstance(g '{00021401-0000-0000-C000-000000000046}', p 0, i 1, g '{000214F9-0000-0000-C000-000000000046}', *p.r1) i.r0"
      ${If} $0 == 0
        ; IShellLinkW::QueryInterface(IPersistFile), then IPersistFile::Load.
        System::Call "$1->0(g '{0000010B-0000-0000-C000-000000000046}', *p.r2) i.r0"
        ${If} $0 == 0
          System::Call '$2->5(w r9, i 0) i.r0'
          ${If} $0 == 0
            ; IShellLinkW::GetPath(..., SLGP_RAWPATH). Never call Resolve.
            System::Call '$1->3(w .r4, i ${NSIS_MAX_STRLEN}, p 0, i 4) i.r0'
            ${If} $0 == 0
            ${AndIf} $4 != ""
              GetFullPathName $4 "$4"
              GetFullPathName $6 "$INSTDIR\Echo.exe"
              ${If} $4 == $6
                StrCpy $7 "1"
              ${EndIf}
            ${EndIf}
          ${EndIf}
          System::Call '$2->2()'
        ${EndIf}
        System::Call '$1->2()'
      ${EndIf}
      ${If} $5 >= 0
        System::Call 'ole32::CoUninitialize()'
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Push $7
  System::Store L
FunctionEnd
!macroend

!insertmacro EchoShortcutTargetFunction ""
!insertmacro EchoShortcutTargetFunction "un."

; Adopt legacy links only after proving their target. Never overwrite a foreign
; Echo.lnk, even when install-mode.ini says an older link was owned.
!macro EchoCreateOwnedShortcut PATH KEY
  StrCpy $0 "1"
  IfFileExists "${PATH}" 0 +4
    Push "${PATH}"
    Call ShortcutTargetsThisInstall
    Pop $0
  ${If} $0 == "1"
    SetOutPath "$INSTDIR"
    ClearErrors
    CreateShortcut "${PATH}" "$INSTDIR\Echo.exe" "" "$INSTDIR\Echo.exe" 0
    ${If} ${Errors}
      StrCpy $0 "0"
      DetailPrint "Could not create shortcut: ${PATH}"
    ${EndIf}
  ${Else}
    DetailPrint "Preserved shortcut owned elsewhere or unreadable: ${PATH}"
  ${EndIf}
  WriteINIStr "$INSTDIR\install-mode.ini" "shortcuts" "${KEY}" "$0"
!macroend

!macro EchoDeleteOwnedShortcut PREFIX PATH OWNED
  ${If} ${OWNED} == "1"
    Push "${PATH}"
    Call ${PREFIX}ShortcutTargetsThisInstall
    Pop $0
    ${If} $0 == "1"
      ClearErrors
      Delete "${PATH}"
      ${If} ${Errors}
        DetailPrint "Could not delete owned shortcut: ${PATH}"
      ${EndIf}
    ${Else}
      DetailPrint "Preserved absent, changed or unreadable shortcut: ${PATH}"
    ${EndIf}
  ${EndIf}
!macroend
!endif
