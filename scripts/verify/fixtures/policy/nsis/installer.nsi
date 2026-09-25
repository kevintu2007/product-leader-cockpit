; Policy fixture: the stock uninstaller's "delete app data" branch, which the
; check must refuse. Not taken from any pinned Tauri CLI version.
Var DeleteAppDataCheckboxState

Section Uninstall
  ${If} $DeleteAppDataCheckboxState = 1
    SetShellVarContext current
    RmDir /r "$APPDATA\${BUNDLEID}"
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}"
  ${EndIf}
SectionEnd
