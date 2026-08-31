; Hooked into Tauri's generated installer.nsi (bundle.windows.nsis.installerHooks). See
; DESIGN.md §12: "The installer owns the Run key, Start-menu entry and uninstall in one
; place." The Start-menu shortcut and the uninstall entry are Tauri's own NSIS defaults;
; this file is only what Toki adds on top — the daemon's autostart registration, which is a
; toki-daemon concept the stock installer script knows nothing about.

!macro NSIS_HOOK_POSTINSTALL
  ; Registers HKCU\...\Run\Toki, the same key `toki-daemon autostart on` writes by hand from
  ; a terminal. Then start it now, not just on the next login — DESIGN.md's "First run" is
  ; the daemon noticing it has never opened the GUI before, which needs it running at all.
  ExecWait '"$INSTDIR\toki-daemon.exe" autostart on'
  Exec '"$INSTDIR\toki-daemon.exe" run'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Must run before files are removed: `autostart off` needs toki-daemon.exe to still exist
  ; at $INSTDIR to be told to clear its own Run key. Leaving the key would point Windows at
  ; a path that no longer exists — the exact stale-autostart failure `toki doctor` warns
  ; about, just self-inflicted by the uninstaller instead of a moved install.
  ExecWait '"$INSTDIR\toki-daemon.exe" autostart off'
  ; Releases the running process's lock on toki-daemon.exe so its own file can be deleted.
  ; The daemon's usual clean-shutdown path (WM_ENDSESSION, Ctrl+C) needs something to
  ; deliver it; taskkill's forceful stop skips that, so the last open session is left for
  ; crash recovery instead of closing cleanly — acceptable here since there may be no next
  ; run to recover it on.
  ExecWait 'taskkill.exe /F /IM toki-daemon.exe'

  ; Tracked history lives in %LOCALAPPDATA%\Toki, outside $INSTDIR, so the normal file
  ; removal below never touches it — an uninstall-then-reinstall (an update, in effect)
  ; keeps a person's data by default. Deleting it is offered, not assumed, and only to
  ; someone actually driving the uninstall: `IfSilent` skips both the prompt and the delete,
  ; because Tauri's own installer runs the previous version's uninstaller silently before an
  ; update, and that automated run must never be the thing that wipes history.
  IfSilent skip_data_prompt
    MessageBox MB_YESNO|MB_ICONQUESTION "Also delete your tracked history and settings?$\n$\nThis removes $LOCALAPPDATA\Toki, including the database. This cannot be undone." IDNO skip_data_prompt
    RMDir /r "$LOCALAPPDATA\Toki"
  skip_data_prompt:
!macroend
