; Havre Desktop installer (NSIS): lets Windows offer Havre as the mail app.
; Settings › Default apps lists applications registered under
; RegisteredApplications with a mailto "URL association"; choosing one is
; up to the user (Windows doesn't let apps make themselves the default).
; Everything goes under HKCU: the installer is per user (no admin rights).

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\Havre.mailto" "" "Havre mail link"
  WriteRegStr HKCU "Software\Classes\Havre.mailto" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\Havre.mailto\DefaultIcon" "" "$INSTDIR\havre-desktop.exe,0"
  WriteRegStr HKCU "Software\Classes\Havre.mailto\shell\open\command" "" '"$INSTDIR\havre-desktop.exe" "%1"'
  WriteRegStr HKCU "Software\Havre\Capabilities" "ApplicationName" "Havre"
  WriteRegStr HKCU "Software\Havre\Capabilities" "ApplicationDescription" "Your Havre mail, agenda and people."
  WriteRegStr HKCU "Software\Havre\Capabilities\URLAssociations" "mailto" "Havre.mailto"
  WriteRegStr HKCU "Software\RegisteredApplications" "Havre" "Software\Havre\Capabilities"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\Havre.mailto"
  DeleteRegKey HKCU "Software\Havre"
  DeleteRegValue HKCU "Software\RegisteredApplications" "Havre"
!macroend
