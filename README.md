# No Sleep Till Brooklyn

![No Sleep Till Brooklyn — a monitor glowing through the Brooklyn night](assets/hero.png)

[![build](https://github.com/vladimirgamalyan/no-sleep-till-brooklyn/actions/workflows/build.yml/badge.svg)](https://github.com/vladimirgamalyan/no-sleep-till-brooklyn/actions/workflows/build.yml)

A tiny "caffeine" utility for Windows 11 — an analogue of the Mac *Caffeine*
app. It has **no visible window**: it lives as a system-tray icon. While it runs
it keeps the machine awake and the display on; exit from the tray menu to stop.

A prebuilt `NoSleepTillBrooklyn.exe` is attached to every
[release](https://github.com/vladimirgamalyan/no-sleep-till-brooklyn/releases),
built by GitHub Actions. To build it yourself, see [Build](#build) below.

## What it does

- Runs with **no visible window and no taskbar button** — only a system-tray
  icon, hosted by a hidden top-level window (see [System tray](#system-tray)).
- **Survives an Explorer restart**: when the shell rebuilds the notification
  area, the app registers its tray icon again instead of vanishing.
- Every 59 seconds it taps the **F15** key (`VK_F15`, `0x7E`) via `SendInput`,
  so the idle timer never fires. F15 is a key virtually no application reacts
  to.
- It also calls `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED |
  ES_DISPLAY_REQUIRED)`, which blocks both system sleep and monitor power-off,
  not merely the idle auto-lock.
- **Exit** (tray icon → right-click → *Exit*) releases the keep-awake request
  (`ES_CONTINUOUS`), stops the timer and terminates the process.
- Launching it with **`--quit`** stops a running copy and exits, without ever
  starting the app itself (see [Stopping it without the tray
  menu](#stopping-it-without-the-tray-menu)).
- Launching it with **`--monitor-off`** does the same and then switches the
  display off (see [Switching the display off
  too](#switching-the-display-off-too)).
- Only a **single instance** runs at a time: a named-mutex guard makes a second
  launch exit silently (no window). Before it exits it pokes the running copy
  (via a named event) to show an "already running" tray notification, distinct
  from the one shown at first launch.
- **Relaunching restores a missing tray icon**: the running copy re-registers its
  icon when poked, so if the icon is ever gone, simply start the app again.

## System tray

There is no visible window. On launch a tray icon appears (with a short balloon
notification) and the app starts keeping the PC awake immediately. Left- or
right-click the icon for a menu whose only item is **Exit**.

The timer runs on the UI thread via `native-windows-gui`'s `AnimationTimer`;
there is no blocking `sleep` anywhere.

### Why a hidden top-level window

The tray icon and its menu are hosted by a window that is created without the
`VISIBLE` flag and is never shown, so it appears neither on screen nor in the
taskbar. It is deliberately a **top-level** window rather than the message-only
window such an invisible host would normally be.

The reason is Explorer. When it restarts or crashes it rebuilds the notification
area from scratch, dropping every icon in it, and announces this by broadcasting
the registered `TaskbarCreated` message. Each app is expected to hear that and
add its icon back. Windows delivers broadcasts to top-level windows only —
message-only windows are excluded by design — so a message-only host can never
receive the message, and its icon is gone for the rest of the process's life.

That failure is worse than a missing icon, because the two other design choices
here compound it: the process keeps running and keeps the PC awake, but with no
icon there is no way to reach its **Exit** menu, and the single-instance mutex
makes every new launch exit silently instead of bringing the icon back. The app
is then stuck awake and unreachable until it is killed from Task Manager.

So the host window is top-level, and on `TaskbarCreated` the app re-registers
its tray icon.

### The relaunch fallback

The running copy also re-registers its icon whenever a second launch pokes it
through the single-instance named event. That path does not touch the tray, so
it arrives even when the icon is gone.

This covers what `TaskbarCreated` cannot. Nothing retries the *initial*
`NIM_ADD`, and `native-windows-gui` discards its result, so an icon that failed
to register at startup — with the shell already up, and therefore no
`TaskbarCreated` coming — would stay missing with nothing to notice. It also
makes the obvious human reaction to a missing icon, starting the app again, do
the right thing rather than exit silently.

## Stopping it without the tray menu

```sh
NoSleepTillBrooklyn.exe --quit
```

The flag is for scripts and shortcuts: it stops a copy that is already running
and exits. If nothing is running it does nothing at all and exits immediately —
it never starts the app, so it is safe to fire blindly.

Mechanically it is the single-instance path again. The launch sees the named
mutex already taken, signals a second named event (the quit one, separate from
the "already running" banner event), and returns. The running copy's watcher
thread wakes its UI thread, which runs the same shutdown as the tray menu's
**Exit**: release the keep-awake request, stop the timer, remove the tray icon
and end the process. With the mutex free, nothing is signalled and the launch
just returns.

### Switching the display off too

```sh
NoSleepTillBrooklyn.exe --monitor-off
```

Everything `--quit` does, and then the display goes off — whether or not a copy
was running, so this too is safe to fire blindly from a shortcut. Moving the
mouse or pressing a key wakes the display back up; the app stays stopped.

The display is switched off the way the classic "monitor off" shortcut does it,
with no third-party helper: broadcast `WM_SYSCOMMAND` with `SC_MONITORPOWER` and
lParam `2` (`1` would be low power, `-1` powers it back on).

Before that, the launch waits for the copy it just stopped to actually exit. It
must: a live copy taps F15 every minute, and that input would turn the display
straight back on. Since a named kernel object lives only as long as a handle to
it is open, the launch closes its own handle to the single-instance mutex and
then polls the name — once the mutex can no longer be opened, the other process
is gone. The wait gives up after two seconds so a wedged copy cannot make the
shortcut hang.

## Build

Requires the Rust MSVC toolchain (`x86_64-pc-windows-msvc`).

```sh
cargo build --release
```

The output is a single self-contained executable:

```
target/release/NoSleepTillBrooklyn.exe
```

The C runtime is linked **statically** so the exe runs on a clean Windows 11
without the Visual C++ redistributable. This is configured in
`.cargo/config.toml`:

```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

so a plain `cargo build --release` already produces the static binary. The
equivalent explicit invocation is:

```sh
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release --target x86_64-pc-windows-msvc
```

An application manifest (`app.manifest`, embedded by `build.rs`) declares a
dependency on Common-Controls v6, which `native-windows-gui` needs at load
time. The application icon (`icon.ico`, referenced by `icon.rc` and embedded by
`build.rs` via `embed-resource`) serves as both the exe's file icon and the
tray icon. The binary is intentionally **not** packed with UPX or obfuscated —
doing so only increases false-positive antivirus flags on an input-emulating
utility.

## Self-contained

The resulting `NoSleepTillBrooklyn.exe` depends only on DLLs that ship with
Windows (`kernel32`, `user32`, `gdi32`, `comctl32`, `shell32`, `ole32`, …).
There is no dependency on `VCRUNTIME140.dll` or any other redistributable, so
you can copy the single `.exe` to any Windows 11 machine and run it.
