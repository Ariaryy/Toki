; Hooked into Tauri's generated installer.nsi (bundle.windows.nsis.installerHooks). See
; DESIGN.md §12: "The installer owns the Run key, Start-menu entry and uninstall in one
; place." The Start-menu shortcut and the uninstall entry are Tauri's own NSIS defaults;
; this file is only what Toki adds on top — the daemon's autostart registration, which is a
; toki-daemon concept the stock installer script knows nothing about.

!ifndef WM_ENDSESSION
  !define WM_ENDSESSION 0x0016
!endif

; Stop a running toki-daemon.exe, cleanly if it will cooperate and forcefully if it won't.
;
; Needed because Tauri's generated script guards only ${MAINBINARYNAME}.exe — toki.exe —
; with its CheckIfAppIsRunning. Nothing in it knows the externalBin sidecar is a
; long-running process too, so `File /oname=toki-daemon.exe` writes over an executable the
; running daemon still holds a lock on. That write fails, the installer reports success
; anyway, and the update leaves a new GUI talking to the previous tracker. Any daemon-side
; change then silently doesn't land — the tray icon is painted in code, so it is exactly
; this class of change.
;
; Asks first, forces second. WM_ENDSESSION is the same clean shutdown the tray's "Quit Toki"
; reaches, so the daemon closes its open session and checkpoints the WAL before exiting,
; rather than leaving the session dangling for crash recovery to find on next start.
!macro TOKI_STOP_DAEMON UNIQ
  Push $0
  Push $1

  ; The pump uses a top-level WS_EX_TOOLWINDOW window rather than a message-only one
  ; specifically so WM_ENDSESSION can reach it (message-only windows receive no broadcasts).
  ; That decision is what makes the daemon reachable from here at all.
  FindWindow $0 "TokiPumpWindow" ""
  IntCmp $0 0 toki_force_${UNIQ}
  SendMessage $0 ${WM_ENDSESSION} 1 0 /TIMEOUT=5000

  ; Poll the process, not the window. The window is destroyed while the pump unwinds, but
  ; the final session write happens after that — so a window that has already vanished does
  ; not yet mean it is safe to kill the process out from under that write.
  StrCpy $1 0
  toki_wait_${UNIQ}:
    !if "${INSTALLMODE}" == "currentUser"
      nsis_tauri_utils::FindProcessCurrentUser "toki-daemon.exe"
    !else
      nsis_tauri_utils::FindProcess "toki-daemon.exe"
    !endif
    Pop $0
    ; 0 means still running; anything else means gone.
    IntCmp $0 0 0 toki_done_${UNIQ} toki_done_${UNIQ}
    Sleep 250
    IntOp $1 $1 + 1
    ; 40 * 250ms = 10s, then stop being polite.
    IntCmp $1 40 toki_force_${UNIQ} toki_wait_${UNIQ} toki_force_${UNIQ}

  toki_force_${UNIQ}:
    ; Reached when there was no window to ask, or asking didn't take. Releasing the lock
    ; matters more than a tidy exit: a stale daemon is worse than a session left for
    ; recovery, which the next start already knows how to close.
    nsExec::Exec 'taskkill.exe /F /IM toki-daemon.exe'
    Pop $0
    Sleep 500

  toki_done_${UNIQ}:
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Before the File commands below overwrite it. See TOKI_STOP_DAEMON.
  !insertmacro TOKI_STOP_DAEMON "preinstall"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Says, once, that Toki reads browser history and checks for updates; Settings is where
  ; each stops. Only on a fresh
  ; install: an update runs this installer again, and a database already there means the
  ; person has been told. `IfSilent` covers Tauri's silent runs.
  IfSilent skip_history_notice
  IfFileExists "$LOCALAPPDATA\Toki\toki.db" skip_history_notice
    MessageBox MB_OK|MB_ICONINFORMATION "Toki names the websites you spend time on by reading the history files of your browsers (Chrome, Firefox, Zen and similar) on this computer.$\n$\nOnly each site's domain is kept, and nothing leaves this computer.$\n$\nYou can turn this off at any time in Toki's Settings, under Websites.$\n$\nOnce a day, while its window is open, Toki also asks GitHub whether a newer version exists. Only the version number comes back. Settings, under Updates, turns this off."
  skip_history_notice:

  ; Registers HKCU\...\Run\Toki, the same key `toki-daemon autostart on` writes by hand from
  ; a terminal. Then start it now, not just on the next login, so tracking begins at
  ; install. Opening the GUI is left to the finish page's "Launch Toki" checkbox.
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
  ; Shares the install path's stop, which tries the clean shutdown before forcing: on an
  ; uninstall there may well be no next run to recover a dangling session on, so closing the
  ; last one properly is worth the few seconds it costs.
  !insertmacro TOKI_STOP_DAEMON "preuninstall"

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
