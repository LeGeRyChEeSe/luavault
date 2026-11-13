; Crochets NSIS de Tauri (bundle.windows.nsis.installerHooks).

; An installer before 1.3.0 shipped the author's signing tool beside the app: an update over such
; an install removes it (the new installers do not carry it).
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\lvrelease.exe"
!macroend

; A real uninstall (not the one an update runs, /UPDATE) asks the app to remove what it wrote outside
; its folder: `--uninstall-cleanup` runs before Tauri starts (headless.rs), removes its Windows entries
; (logon values, URL scheme, toast identity, scheduled task), and always exits 0.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup'
  ${EndIf}
!macroend
