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
- Only a **single instance** runs at a time: a named-mutex guard makes a second
  launch exit silently (no window). Before it exits it pokes the running copy
  (via a named event) to show an "already running" tray notification, distinct
  from the one shown at first launch.

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
