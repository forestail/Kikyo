!macro NSIS_HOOK_PREINSTALL
  ClearErrors
  ReadRegStr $0 HKCU "SOFTWARE\Microsoft\Windows\CurrentVersion\Run" "Kikyo"
  CreateDirectory "$APPDATA\com.forestail.kikyo"
  FileOpen $1 "$APPDATA\com.forestail.kikyo\autostart.pref" w
  IfErrors kikyo_autostart_preinstall_done
  StrCmp $0 "" 0 kikyo_autostart_preinstall_enabled
  FileWrite $1 "0"
  Goto kikyo_autostart_preinstall_close
kikyo_autostart_preinstall_enabled:
  FileWrite $1 "1"
kikyo_autostart_preinstall_close:
  FileClose $1
kikyo_autostart_preinstall_done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$APPDATA\com.forestail.kikyo\autostart.pref" 0 kikyo_autostart_postinstall_done
  ClearErrors
  FileOpen $0 "$APPDATA\com.forestail.kikyo\autostart.pref" r
  IfErrors kikyo_autostart_postinstall_done
  FileRead $0 $1
  FileClose $0
  StrCpy $1 $1 1
  StrCmp $1 "1" 0 kikyo_autostart_postinstall_done
  WriteRegStr HKCU "SOFTWARE\Microsoft\Windows\CurrentVersion\Run" "Kikyo" "$\"$INSTDIR\kikyo.exe$\""
  WriteRegBin HKCU "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Kikyo" 020000000000000000000000
kikyo_autostart_postinstall_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "SOFTWARE\Microsoft\Windows\CurrentVersion\Run" "Kikyo"
  DeleteRegValue HKCU "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Kikyo"
!macroend
