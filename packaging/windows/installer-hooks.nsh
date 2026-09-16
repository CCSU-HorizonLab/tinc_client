!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing Tinc Connect local service and network driver..."
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\payload\install-client.ps1" -PayloadRoot "$INSTDIR\payload"'
  Pop $0
  Pop $1
  DetailPrint "$1"
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "Tinc Connect local service installation failed.$\r$\n$\r$\n$1"
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing Tinc Connect local service..."
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\payload\uninstall-client.ps1"'
  Pop $0
  Pop $1
  DetailPrint "$1"
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "Tinc Connect local service removal failed.$\r$\n$\r$\n$1"
    Abort
  ${EndIf}
!macroend
