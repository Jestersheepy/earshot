; Earshot's additions to Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).

!macro NSIS_HOOK_POSTUNINSTALL
  ; Not when an update reinstalls over this version: the models and settings carry over.
  ${If} $UpdateMode <> 1
    ; Start with Windows (tauri-plugin-autostart) would point at the removed app.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Earshot"
    ${If} $DeleteAppDataCheckboxState = 1
      ; The downloaded models (about 4 GB), the log, and settings and dictation history.
      ; Notes stay in Documents\Earshot.
      RMDir /r "$INSTDIR\models"
      Delete "$INSTDIR\earshot.log"
      RMDir /r "$APPDATA\Earshot"
    ${EndIf}
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
