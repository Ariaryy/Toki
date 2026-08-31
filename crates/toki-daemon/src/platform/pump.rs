//! The one thread that talks to Windows about what is happening.
//!
//! It owns two accessibility hooks, a hidden window, the tray icon on it, and the message
//! loop all of them need. Everything it learns leaves through a channel; nothing is decided here.
//!
//! The hook callback runs inside `GetMessage` on this thread. Windows removes hooks whose
//! callbacks are slow, and a collector that has been silently unhooked still looks alive,
//! so the callback does the least work that produces a usable event and returns: a
//! foreground check, a cached process lookup, and a send.
//!
//! Events are timestamped here rather than where they are consumed. Anything the run loop
//! is busy with — a resolver pass, a slow disk — would otherwise be charged to whichever
//! app you happened to switch to.
//!
//! `EVENT_OBJECT_NAMECHANGE` is the interesting hook. A browser tab switch changes no
//! window, only its caption, so without it every tab collapses into whichever was open when
//! the window came forward. It also fires for progress bars, clocks, and every keystroke in
//! an address bar, which is why it is filtered three ways before anything is resolved.

use std::cell::RefCell;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use chrono::Utc;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterSuspendResumeNotification;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, PostQuitMessage, PostThreadMessageW, RegisterClassW, TranslateMessage,
    CHILDID_SELF, DEVICE_NOTIFY_WINDOW_HANDLE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_NAMECHANGE,
    EVENT_SYSTEM_FOREGROUND, HMENU, MSG, OBJID_WINDOW, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
    WM_ENDSESSION, WM_POWERBROADCAST, WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_POPUP, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF, WTS_SESSION_UNLOCK,
};

use toki_core::event::Millis;

use super::window::{Window, WindowCache};

/// What the pump saw. Interpreting it is the run loop's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raw {
    /// A different window came forward, or the foreground window's caption changed.
    Focus {
        at: Millis,
        window: Window,
    },
    Suspend {
        at: Millis,
    },
    /// Woken, or unlocked. The run loop responds by re-reading the foreground window,
    /// because Windows does not reliably re-announce it.
    Resume {
        at: Millis,
    },
    Lock {
        at: Millis,
    },
    /// Stop recording. `until` is a duration in milliseconds; `None` means until the user
    /// says otherwise. Sent by the tray, never by Windows.
    Pause {
        at: Millis,
        until: Option<Millis>,
    },
    Unpause {
        at: Millis,
    },
    /// A new idle threshold from Settings, to take effect immediately rather than waiting
    /// for the next restart. Sent by IPC, never by Windows or the tray.
    SetConfig {
        idle_threshold_ms: Millis,
    },
    /// Logging off or shutting down. The last chance to close cleanly.
    Stop {
        at: Millis,
    },
}

/// Posted to the pump thread to make `GetMessage` return so the loop can exit.
const WM_TOKI_QUIT: u32 = WM_APP + 1;

/// How many window handles to remember before assuming the map is holding dead ones.
const CACHE_LIMIT: usize = 2_048;

/// State the hook callback reaches through. The callback takes no argument of its own, and
/// there is exactly one pump thread, so a thread-local is the only route to the channel.
struct HookState {
    tx: Sender<Raw>,
    cache: RefCell<WindowCache>,
    last: RefCell<Option<(String, String)>>,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// Published once the loop is running, so the thread can be woken from outside.
static PUMP_THREAD: OnceLock<u32> = OnceLock::new();

/// Ask the pump to shut down. Safe to call before it has started.
pub fn stop() {
    if let Some(&tid) = PUMP_THREAD.get() {
        let _ = unsafe { PostThreadMessageW(tid, WM_TOKI_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

fn now() -> Millis {
    Utc::now().timestamp_millis()
}

/// Run the message loop until [`stop`] is called or the session ends.
///
/// Blocks. Intended as the body of a dedicated thread.
pub fn run(tx: Sender<Raw>) {
    let _ = PUMP_THREAD.set(unsafe { GetCurrentThreadId() });

    STATE.with(|s| {
        *s.borrow_mut() = Some(HookState {
            tx,
            cache: RefCell::new(WindowCache::new()),
            last: RefCell::new(None),
        });
    });

    // `WINEVENT_SKIPOWNPROCESS` keeps the tray and the GUI from reporting on themselves.
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let foreground = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_win_event),
            0,
            0,
            flags,
        )
    };
    // A second hook rather than one wide range: a range spanning from EVENT_SYSTEM_FOREGROUND
    // to EVENT_OBJECT_NAMECHANGE would subscribe to every object event in between, for every
    // process on the desktop.
    let names = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_NAMECHANGE,
            None,
            Some(on_win_event),
            0,
            0,
            flags,
        )
    };

    let hwnd = create_window();
    if let Some(hwnd) = hwnd {
        // Power notifications are opt-in whatever kind of window asks, and both of these
        // are targeted posts rather than broadcasts.
        let _ = unsafe {
            RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
        };
        let _ = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
        super::tray::add(hwnd);
    }

    // Whatever is in front when we start is the first session.
    emit_focus(unsafe { GetForegroundWindow() });

    let mut msg = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 || msg.message == WM_TOKI_QUIT || msg.message == WM_QUIT {
            break;
        }
        let _ = unsafe { TranslateMessage(&msg) };
        unsafe { DispatchMessageW(&msg) };
    }

    if let Some(hwnd) = hwnd {
        // Explicitly, and before the window goes: an icon whose owner has died lingers in
        // the tray until something makes the shell notice.
        super::tray::remove(hwnd);
        let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    for hook in [foreground, names] {
        if !hook.is_invalid() {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
    STATE.with(|s| *s.borrow_mut() = None);
}

/// The accessibility callback. This runs for events across the whole desktop, so the cheap
/// tests come first and nothing else happens until one of them passes.
unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // The window itself, not its contents. Without this, a caption is re-read for every
    // button, list item and progress bar that renames itself.
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }

    match event {
        EVENT_SYSTEM_FOREGROUND => emit_focus(hwnd),
        EVENT_OBJECT_NAMECHANGE => {
            // Background windows rename themselves constantly and none of it is time you
            // spent. This is the filter that makes the name hook affordable.
            if hwnd == unsafe { GetForegroundWindow() } {
                emit_focus(hwnd);
            }
        }
        EVENT_OBJECT_DESTROY => STATE.with(|s| {
            if let Some(state) = s.borrow().as_ref() {
                state.cache.borrow_mut().forget(hwnd);
            }
        }),
        _ => {}
    }
}

/// Resolve a window and send it, unless it says nothing new.
///
/// Both hooks come through here, and an unchanged executable and caption is dropped either
/// way. That is not just deduplication: returning to a window after a trip through the
/// Start menu raises a foreground event, and sending it would split the session the ignore
/// list exists to keep whole. A genuine switch to another app always changes the key on the
/// way out, so a real return is never suppressed.
fn emit_focus(hwnd: HWND) {
    STATE.with(|s| {
        let borrowed = s.borrow();
        let Some(state) = borrowed.as_ref() else {
            return;
        };

        let window = {
            let mut cache = state.cache.borrow_mut();
            // A desktop rarely has more than a few hundred windows. Well past that, the map
            // is holding handles that were closed without a destroy event reaching us.
            cache.trim(CACHE_LIMIT);
            match cache.describe(hwnd) {
                Some(w) => w,
                None => return,
            }
        };

        // Shell surfaces produce no event at all, so the session they interrupted stays
        // open. Pressing Start for four seconds is part of what you were doing.
        if toki_core::ignore::is_default_ignored(&window.exe_path) {
            return;
        }
        // explorer.exe is also a real application — a File Explorer window — so it can't go
        // on the list above; only its shell-chrome surfaces (desktop, taskbar, Alt-Tab) are
        // ignored, by window class rather than by executable.
        if toki_core::ignore::is_shell_chrome(&window.exe_path, &window.class) {
            return;
        }

        let key = (window.exe_path.clone(), window.title.clone());
        {
            let mut last = state.last.borrow_mut();
            if last.as_ref() == Some(&key) {
                return;
            }
            *last = Some(key);
        }

        let _ = state.tx.send(Raw::Focus { at: now(), window });
    });
}

fn send(raw: Raw) {
    STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            let _ = state.tx.send(raw);
        }
    });
}

/// A top-level window that is never shown: the tray's owner, and the only thing that hears
/// about power, session and shutdown.
///
/// Deliberately not a message-only window, which is what this was to begin with. Two things
/// forced the change. `HWND_MESSAGE` windows receive no broadcasts, so `WM_ENDSESSION` never
/// arrived and the clean-shutdown path was quietly dead. And a popup menu owned by one
/// cannot take the foreground, so the tray menu would stay on screen after you clicked away
/// from it.
///
/// `WS_EX_TOOLWINDOW` with no `WS_VISIBLE` and no `ShowWindow` keeps it out of the taskbar
/// and out of Alt-Tab, so it is invisible in every sense that matters.
fn create_window() -> Option<HWND> {
    let class = w!("TokiPumpWindow");
    let instance = unsafe { GetModuleHandleW(None) }.ok()?;

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    // Zero means the class already exists, which is fine and needs no handling.
    unsafe { RegisterClassW(&wc) };

    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("Toki"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None::<HMENU>,
            Some(instance.into()),
            None,
        )
    }
    .ok()
}

/// Launch the GUI, if it is installed. Spawned and forgotten: it owns its own lifetime, and
/// the daemon outliving it is the normal case.
///
/// `pub(crate)` rather than private: `run.rs` also calls this once, for the first-run open.
pub(crate) fn open_gui() {
    if let Some(path) = crate::paths::gui_path() {
        let _ = std::process::Command::new(path).spawn();
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Asked first because the tray's messages are `WM_APP` and up plus one registered at
    // run time, none of which can collide with what follows.
    if let Some(cmd) = super::tray::handle(hwnd, msg, wparam, lparam) {
        match cmd {
            super::tray::Command::Open => open_gui(),
            // The tray hands back a duration; when it ends is decided here, against the
            // same clock every other event is stamped with.
            super::tray::Command::Pause(dur) => send(Raw::Pause {
                at: now(),
                until: dur.map(|d| now() + d),
            }),
            super::tray::Command::Resume => send(Raw::Unpause { at: now() }),
            super::tray::Command::Quit => {
                send(Raw::Stop { at: now() });
                unsafe { PostQuitMessage(0) };
            }
        }
        return LRESULT(0);
    }

    match msg {
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_APMSUSPEND => send(Raw::Suspend { at: now() }),
                // Which resume message arrives depends on why the machine woke. The run
                // loop treats a second one as a no-op.
                PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => send(Raw::Resume { at: now() }),
                _ => {}
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            match wparam.0 as u32 {
                WTS_SESSION_LOCK | WTS_SESSION_LOGOFF => send(Raw::Lock { at: now() }),
                WTS_SESSION_UNLOCK => send(Raw::Resume { at: now() }),
                _ => {}
            }
            LRESULT(0)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                send(Raw::Stop { at: now() });
                unsafe { PostQuitMessage(0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The foreground window right now. Used after a resume, when no event fires on its own.
pub fn current_foreground() -> Option<Window> {
    super::window::describe(unsafe { GetForegroundWindow() })
}
