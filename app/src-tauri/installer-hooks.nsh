; Extra steps for the Runqora installer (Tauri NSIS hooks).

; Record how this install happened (read by the tests, and handy when something goes wrong): "updated in place" when the
; previous version was still registered at this point, i.e. it was NOT uninstalled first; "fresh" otherwise.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayVersion"
  StrCmp $0 "" rq_fresh
  WriteRegStr HKCU "Software\Runqora" "LastInstall" "updated in place from $0"
  Goto rq_recorded
  rq_fresh:
  WriteRegStr HKCU "Software\Runqora" "LastInstall" "fresh"
  rq_recorded:
!macroend

; After a fresh install, ask once whether Runqora should open (hidden, in the tray) when the user signs in to Windows.
; Silent installs never ask, and an update never asks again: the answer is remembered under HKCU\Software\Runqora.
!macro NSIS_HOOK_POSTINSTALL
  IfSilent rq_login_done
  ReadRegStr $0 HKCU "Software\Runqora" "LoginAsked"
  StrCmp $0 "1" rq_login_done
  WriteRegStr HKCU "Software\Runqora" "LoginAsked" "1"
  MessageBox MB_YESNO|MB_ICONQUESTION "Start Runqora automatically when you sign in to Windows?$\r$\n$\r$\nIt opens quietly in the system tray, so your servers can start by themselves. You can change this at any time in Runqora's Settings or from the tray menu." IDNO rq_login_done
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Runqora" '"$INSTDIR\runqora.exe" --hidden'
  rq_login_done:
!macroend

; Uninstalling removes the sign-in entry it may have added. Servers and databases in C:\Runqora are never touched.
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Runqora"
  DeleteRegKey HKCU "Software\Runqora"
!macroend
