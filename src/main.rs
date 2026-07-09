// No Sleep Till Brooklyn — a tiny "caffeine" utility for Windows 11.
//
// The app has no window: it lives as a system-tray icon. While it runs it
// periodically taps the F15 key and asks the system to stay awake, so the
// display never dims and the machine never locks or sleeps on idle.
// Right-click the tray icon and choose Exit to stop and quit.
#![windows_subsystem = "windows"]

extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

use std::time::Duration;

use nwd::NwgUi;
use nwg::NativeUi;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, BOOL, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_F15,
};

/// Interval between F15 keypresses, in seconds.
const KEYPRESS_INTERVAL_SECS: u64 = 59;

#[derive(Default, NwgUi)]
pub struct NoSleepTray {
    // Message-only window: it hosts the tray icon and menu but is never shown,
    // so there is no window and no taskbar button.
    #[nwg_control]
    window: nwg::MessageWindow,

    // Loads the app icon compiled into the exe (icon.rc, RT_GROUP_ICON id 1).
    #[nwg_resource]
    embed: nwg::EmbedResource,

    #[nwg_resource(source_embed: Some(&data.embed), source_embed_id: 1)]
    icon: nwg::Icon,

    #[nwg_control(icon: Some(&data.icon), tip: Some("No Sleep Till Brooklyn"))]
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
    fn show_already_running_banner(&self) {
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

fn main() {
    // Single-instance guard: a named mutex. If it already exists, another copy
    // is running (handled below). The handle is intentionally held for the whole
    // process so the mutex lives on.
    let _mutex = unsafe {
        CreateMutexW(
            None,
            BOOL(0),
            w!("NoSleepTillBrooklyn-SingleInstance-8f3c2a17"),
        )
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // Another copy is already running: signal it to re-show its tray banner,
        // then exit silently without a window of our own.
        unsafe {
            if let Ok(event) = CreateEventW(
                None,
                BOOL(0),
                BOOL(0),
                w!("NoSleepTillBrooklyn-ShowBanner-8f3c2a17"),
            ) {
                let _ = SetEvent(event);
            }
        }
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

    // Background watcher: when a second copy signals the shared event, wake the
    // UI thread (via Notice) to re-show the banner, as if launched afresh.
    let sender = app.notice.sender();
    std::thread::spawn(move || {
        let event = match unsafe {
            CreateEventW(
                None,
                BOOL(0),
                BOOL(0),
                w!("NoSleepTillBrooklyn-ShowBanner-8f3c2a17"),
            )
        } {
            Ok(handle) => handle,
            Err(_) => return,
        };
        loop {
            unsafe { WaitForSingleObject(event, INFINITE) };
            sender.notice();
        }
    });

    nwg::dispatch_thread_events();
}
