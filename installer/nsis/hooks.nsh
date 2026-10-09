; Velox Download Manager — NSIS installer hooks (included by Tauri's template).
;
; Install: register the browser integration (native messaging host) for the
; installing user, so the browser extension can reach Velox before the app
; is started for the first time. The app registers again on every start
; (also for other users of a per-machine installation).
;
; Uninstall: remove the native messaging registration and the "start with
; Windows" entry. Downloaded files are never touched; application data is
; removed only when the user ticks "Delete application data".

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering browser integration..."
  nsExec::ExecToLog '"$INSTDIR\velox-nmh.exe" --register'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Browser integration will be registered when Velox starts ($0)."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing browser integration..."
  nsExec::ExecToLog '"$INSTDIR\velox-nmh.exe" --unregister'
  Pop $0
  ; Autostart entry written by the "Start with Windows" setting.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Velox Download Manager"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Velox Download Manager"
!macroend
