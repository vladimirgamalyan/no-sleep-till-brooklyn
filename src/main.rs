// No Sleep Till Brooklyn — a tiny "caffeine" utility for Windows 11.
//
// The app has no visible window: it lives as a system-tray icon. While it runs it
// periodically taps the F15 key and asks the system to stay awake, so the
// display never dims and the machine never locks or sleeps on idle.
// Right-click the tray icon and choose Exit to stop and quit.
#![windows_subsystem = "windows"]

extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use std::rc::Rc;
use std::time::Duration;

use nwd::NwgUi;
use nwg::NativeUi;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenMutexW, SetEvent, WaitForSingleObject, INFINITE,
    SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_F15,
};
use windows::Win32::UI::WindowsAndMessaging::{
    RegisterWindowMessageW, SendMessageW, HWND_BROADCAST, SC_MONITORPOWER, WM_SYSCOMMAND,
};

/// Interval between F15 keypresses, in seconds.
const KEYPRESS_INTERVAL_SECS: u64 = 59;

/// Tooltip shown when hovering the tray icon.
const TRAY_TIP: &str = "No Sleep Till Brooklyn";

/// Id of the raw handler that watches for `TaskbarCreated`. Ids up to 0xFFFF
/// are reserved by native-windows-gui.
const TASKBAR_CREATED_HANDLER_ID: usize = 0x1_0000;

/// Event a plain second launch signals to make the running copy re-show its
/// tray banner and restore its icon.
const BANNER_EVENT: PCWSTR = w!("NoSleepTillBrooklyn-ShowBanner-8f3c2a17");

/// Event a `--quit` launch signals to make the running copy shut down.
const QUIT_EVENT: PCWSTR = w!("NoSleepTillBrooklyn-Quit-8f3c2a17");

/// The single-instance mutex. Also serves as the liveness marker of the running
/// copy: a named object exists only while some handle to it is open.
const SINGLE_INSTANCE_MUTEX: PCWSTR = w!("NoSleepTillBrooklyn-SingleInstance-8f3c2a17");

/// Command-line flag: stop the running copy and exit, doing nothing if none is
/// running. Never starts the app.
const QUIT_FLAG: &str = "--quit";

/// Command-line flag: everything `--quit` does, and then switch the display off.
/// The display goes off whether or not a copy was running. Never starts the app.
const MONITOR_OFF_FLAG: &str = "--monitor-off";

#[derive(Default, NwgUi)]
pub struct NoSleepTray {
    // Hosts the tray icon and menu. Created without VISIBLE and never shown, so
    // there is no window on screen and no taskbar button.
    //
    // It must be a real top-level window rather than a MessageWindow: the shell
    // announces a rebuilt notification area by broadcasting `TaskbarCreated`,
    // and message-only windows do not receive broadcast messages.
    #[nwg_control(flags: "WINDOW", title: "No Sleep Till Brooklyn")]
    #[nwg_events(OnInit: [NoSleepTray::on_init(RC_SELF)])]
    window: nwg::Window,

    // Loads the app icon compiled into the exe (icon.rc, RT_GROUP_ICON id 1).
    #[nwg_resource]
    embed: nwg::EmbedResource,

    #[nwg_resource(source_embed: Some(&data.embed), source_embed_id: 1)]
    icon: nwg::Icon,

    #[nwg_control(icon: Some(&data.icon), tip: Some(TRAY_TIP))]
    #[nwg_events(MousePressLeftUp: [NoSleepTray::show_menu], OnContextMenu: [NoSleepTray::show_menu])]
    tray: nwg::TrayNotification,

    #[nwg_control(parent: window, popup: true)]
    tray_menu: nwg::Menu,

    #[nwg_control(parent: tray_menu, text: "Exit")]
    #[nwg_events(OnMenuItemSelected: [NoSleepTray::exit])]
    menu_exit: nwg::MenuItem,

    // Lets the background watcher thread wake the UI thread to re-show the banner.
    #[nwg_control(parent: window)]
    #[nwg_events(OnNotice: [NoSleepTray::show_already_running_banner])]
    notice: nwg::Notice,

    // Lets the background watcher thread shut the app down when a `--quit`
    // launch asks for it, exactly as the tray menu's Exit does.
    #[nwg_control(parent: window)]
    #[nwg_events(OnNotice: [NoSleepTray::exit])]
    quit_notice: nwg::Notice,

    // Fires every KEYPRESS_INTERVAL_SECS on the UI thread; each tick taps F15.
    #[nwg_control(interval: Duration::from_secs(KEYPRESS_INTERVAL_SECS), active: false)]
    #[nwg_events(OnTimerTick: [NoSleepTray::on_tick])]
    timer: nwg::AnimationTimer,

    // One-shot: shows the launch banner ~300 ms after startup, once the message
    // loop is running and the tray icon has settled in the shell, so the balloon
    // is not swallowed by the race with icon registration.
    #[nwg_control(interval: Duration::from_millis(300), active: false)]
    #[nwg_events(OnTimerTick: [NoSleepTray::on_launch_banner])]
    launch_timer: nwg::AnimationTimer,
}

impl NoSleepTray {
    /// Start watching for `TaskbarCreated`, which the shell broadcasts whenever
    /// it rebuilds the notification area (typically after Explorer restarts or
    /// crashes). Every tray icon is dropped when that happens, and only a fresh
    /// `NIM_ADD` brings it back — otherwise the app keeps running with no icon
    /// and no way to reach its menu.
    fn on_init(rc_self: &Rc<NoSleepTray>) {
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        if taskbar_created == 0 {
            return;
        }

        // Weak, so the handler does not keep the UI alive: a strong reference
        // here would be a cycle through the window the handler is bound to, and
        // TrayNotification's Drop (which removes the icon) would never run.
        let ui = Rc::downgrade(rc_self);
        let _ = nwg::bind_raw_event_handler(
            &rc_self.window.handle,
            TASKBAR_CREATED_HANDLER_ID,
            move |_hwnd, msg, _w, _l| {
                if msg == taskbar_created {
                    if let Some(ui) = ui.upgrade() {
                        ui.readd_tray_icon();
                    }
                }
                None
            },
        );
    }

    /// Add the tray icon to the notification area again.
    ///
    /// `NIM_ADD` is only ever issued by TrayNotification's builder, so the
    /// builder is re-run into a throwaway value. A tray handle is derived from
    /// its parent window, so the rebuilt icon gets the same handle as `self.tray`
    /// and the event bindings made at startup keep matching it.
    ///
    /// The throwaway is forgotten rather than dropped: its Drop would issue
    /// `NIM_DELETE` and undo the icon that was just restored. It owns no memory,
    /// and `self.tray` still removes the icon on exit.
    ///
    /// Safe to call when the icon is already present — the shell rejects a
    /// duplicate `NIM_ADD` and leaves the existing icon alone — so callers do
    /// not need to know whether it is missing.
    fn readd_tray_icon(&self) {
        let mut tray = nwg::TrayNotification::default();
        let rebuilt = nwg::TrayNotification::builder()
            .parent(&self.window)
            .icon(Some(&self.icon))
            .tip(Some(TRAY_TIP))
            .build(&mut tray);

        if rebuilt.is_ok() {
            std::mem::forget(tray);
        }
    }

    fn on_tick(&self) {
        send_f15();
    }

    /// Show a tray balloon with the app title and the given message.
    fn show_banner(&self, text: &str) {
        self.tray.show(
            text,
            Some("No Sleep Till Brooklyn"),
            Some(nwg::TrayNotificationFlags::INFO_ICON),
            None,
        );
    }

    /// Balloon shown when the app is first launched.
    fn show_launch_banner(&self) {
        self.show_banner("Keeping this PC awake. Right-click the tray icon to exit.");
    }

    /// One-shot launch-timer tick: show the launch banner once, then stop.
    fn on_launch_banner(&self) {
        self.launch_timer.stop();
        self.show_launch_banner();
    }

    /// Balloon shown when a second copy is started while this one is running.
    ///
    /// Doubles as a manual recovery hatch. The named event that triggers this
    /// does not go through the tray, so it arrives even when the icon is gone —
    /// and relaunching is exactly what a user does when they cannot find it.
    /// Restoring the icon first also un-breaks the balloon below, which is a
    /// `NIM_MODIFY` on that icon and silently does nothing while it is missing.
    ///
    /// `readd_tray_icon` covers what `TaskbarCreated` cannot: an `NIM_ADD` that
    /// failed at startup is never announced or retried by anything else.
    fn show_already_running_banner(&self) {
        self.readd_tray_icon();
        self.show_banner(
            "Already running — no second copy started. Right-click the tray icon to exit.",
        );
    }

    fn show_menu(&self) {
        let (x, y) = nwg::GlobalCursor::position();
        self.tray_menu.popup(x, y);
    }

    fn exit(&self) {
        set_keep_awake(false);
        self.timer.stop();
        nwg::stop_thread_dispatch();
    }
}

/// Enable or release the system keep-awake request for the current thread.
///
/// `ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED` blocks both sleep
/// and display power-off; `ES_CONTINUOUS` alone releases the request.
fn set_keep_awake(enable: bool) {
    let flags = if enable {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    unsafe {
        SetThreadExecutionState(flags);
    }
}

/// Emit a single F15 key press (down + up) via SendInput.
///
/// F15 is a virtual key that virtually no application reacts to, so it resets
/// the idle timer without disturbing whatever the user is doing.
fn send_f15() {
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_F15,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_F15,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Create — or open, if the running copy already made it — one of the named
/// auto-reset events the two copies talk through.
fn named_event(name: PCWSTR) -> Option<HANDLE> {
    unsafe { CreateEventW(None, BOOL(0), BOOL(0), name).ok() }
}

/// Signal a named event, waking the running copy's watcher thread for it.
fn signal_event(name: PCWSTR) {
    if let Some(event) = named_event(name) {
        unsafe {
            let _ = SetEvent(event);
        }
    }
}

/// Spawn a background watcher that pokes the UI thread through `sender` every
/// time its event is signalled.
///
/// The event is opened by the new thread itself, through `open`, because
/// neither a HANDLE nor the PCWSTR name it comes from is `Send`.
fn watch_event(open: fn() -> Option<HANDLE>, sender: nwg::NoticeSender) {
    std::thread::spawn(move || {
        let Some(event) = open() else { return };
        loop {
            unsafe { WaitForSingleObject(event, INFINITE) };
            sender.notice();
        }
    });
}

/// Switch the display off, the same way the classic "monitor off" desktop
/// shortcut does: broadcast `WM_SYSCOMMAND` with `SC_MONITORPOWER` and lParam 2
/// (`1` would be low power, `-1` powers the display back on). Any mouse move or
/// keypress wakes the display again.
fn monitor_off() {
    unsafe {
        SendMessageW(
            HWND_BROADCAST,
            WM_SYSCOMMAND,
            WPARAM(SC_MONITORPOWER as usize),
            LPARAM(2),
        );
    }
}

/// Block until the copy that was just asked to quit is really gone, giving up
/// after a couple of seconds.
///
/// The display must not be switched off while that copy is still alive: it taps
/// F15 every minute, and any input turns the display straight back on.
///
/// Liveness is read off the single-instance mutex, which the system destroys
/// once the last handle to it closes — so failing to open it by name means the
/// other process has exited. Our own handle, `mutex`, is closed first, as it
/// would otherwise keep the name alive by itself.
fn wait_for_exit(mutex: HANDLE) {
    unsafe {
        let _ = CloseHandle(mutex);
    }

    for _ in 0..100 {
        match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, BOOL(0), SINGLE_INSTANCE_MUTEX) } {
            Ok(handle) => unsafe {
                let _ = CloseHandle(handle);
            },
            Err(_) => return,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn main() {
    let requested = |flag: &str| std::env::args_os().skip(1).any(|arg| arg == flag);
    let quit_requested = requested(QUIT_FLAG);
    let monitor_off_requested = requested(MONITOR_OFF_FLAG);

    // Single-instance guard: a named mutex. If it already exists, another copy
    // is running (handled below). The handle is intentionally held for the whole
    // process so the mutex lives on.
    let mutex = unsafe { CreateMutexW(None, BOOL(0), SINGLE_INSTANCE_MUTEX) };
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    if quit_requested || monitor_off_requested {
        // Ask the running copy to shut down. With nothing running there is
        // nothing to stop — either way this launch never starts the app.
        if already_running {
            signal_event(QUIT_EVENT);

            if monitor_off_requested {
                if let Ok(mutex) = mutex {
                    wait_for_exit(mutex);
                }
            }
        }

        if monitor_off_requested {
            monitor_off();
        }
        return;
    }

    if already_running {
        // Another copy is already running: signal it to re-show its tray banner,
        // then exit silently without a window of our own.
        signal_event(BANNER_EVENT);
        return;
    }

    nwg::init().expect("Failed to init Native Windows GUI");
    let app = NoSleepTray::build_ui(Default::default()).expect("Failed to build UI");

    // Start keeping the machine awake and tapping F15 immediately.
    set_keep_awake(true);
    app.timer.start();

    // Let the user know it launched, since there is no window to see. Deferred by
    // ~300 ms (via launch_timer) so the shell has settled the tray icon first.
    app.launch_timer.start();

    // Background watchers: when another launch signals one of the shared
    // events, wake the UI thread (via Notice) to re-show the banner, as if
    // launched afresh, or to shut down.
    watch_event(|| named_event(BANNER_EVENT), app.notice.sender());
    watch_event(|| named_event(QUIT_EVENT), app.quit_notice.sender());

    nwg::dispatch_thread_events();
}
