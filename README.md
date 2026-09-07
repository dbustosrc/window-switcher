# Window Switcher

Window Switcher is a lightweight, classic-style Alt+Tab replacement for Windows. It switches individual windows instead of grouping them by application and is designed for low latency and minimal resource usage.

## Features

- `Alt+Tab` cycles through every independent top-level window in Windows' global window order.
- `Alt+\`` (Backtick) cycles through windows that belong to the same application.
- The selected window title is displayed below the icon.
- Independent documents or sessions remain separate entries, even when they belong to the same process. For example, two Excel workbooks or two SAP GUI sessions appear separately.
- A blocking modal dialog and its disabled owner are represented by one entry. The selector displays the dialog title when available and activates the dialog when selected.
- Minimized windows and windows on other virtual desktops can be included or excluded through configuration.
- Elevated applications are supported when Window Switcher is also running as administrator.

Hold `Alt` and press `Tab` or Backtick repeatedly to move through the available windows. Release `Alt` to activate the selected window. Add `Shift` to cycle in reverse.

## Installation

There is not yet a packaged release for this independent version. Build it from source using the Rust MSVC toolchain:

```powershell
git clone https://github.com/dbustosrc/window-switcher.git
cd window-switcher
cargo build --release
```

Copy both required runtime files to the same destination directory:

```powershell
$destination = "$env:LOCALAPPDATA\Programs\window-switcher"
New-Item -ItemType Directory -Force -Path $destination
Copy-Item .\target\release\window-switcher.exe $destination
Copy-Item .\window-switcher.ini $destination
```

Run `window-switcher.exe` from that directory. The configuration file must remain beside the executable.

## Configuration

Window Switcher reads `window-switcher.ini` from the executable directory. Restart the application after editing it. When the tray icon is enabled, its context menu provides a shortcut for opening the configuration file.

```ini
# Whether to show the tray icon: yes/no
trayicon = yes

[switch-windows]

# Switch between windows of the same application.
# Multiple hotkeys can be separated with ||.
hotkey = alt+`

# Executables for which the same-application shortcut is disabled.
# Example: game1.exe,game2.exe
blacklist =

# Exclude minimized windows: yes/no
ignore_minimal = no

# Restrict switching to the current virtual desktop: yes/no/auto
# auto follows Windows' Alt+Tab virtual-desktop setting.
only_current_desktop = auto

[switch-apps]

# Enable classic switching between all individual windows: yes/no
enable = yes

# Multiple hotkeys can be separated with ||.
hotkey = alt+tab

# Exclude minimized windows: yes/no
ignore_minimal = no

# Override application icons.
# Syntax: app1.exe=icon1.ico,app2.exe=icon2.png
# Paths can be absolute or relative to the executable directory.
override_icons =

# Restrict switching to the current virtual desktop: yes/no/auto
# auto follows Windows' Alt+Tab virtual-desktop setting.
only_current_desktop = auto

[log]

# One of: off, error, warn, info, debug, trace
level = info

# Relative paths are resolved from the executable directory.
# Leave empty to disable the operational log.
path = window-switcher.log
```

Boolean values also accept `true/false`, `on/off`, and `1/0`.

## Running as administrator

A standard-integrity process cannot intercept keyboard input intended for an elevated window. If Task Manager or another elevated application has focus, Windows may show its native Alt+Tab selector unless Window Switcher is also elevated.

To start Window Switcher automatically:

1. Run it at the privilege level you want to use permanently.
2. Right-click the tray icon.
3. Enable **Startup**.

When enabled from an elevated instance, Window Switcher creates a Scheduled Task with the highest available run level. The generated task is configured to continue running when the computer switches to battery power. A standard instance uses the current user's `Run` registry key instead.

Disable an existing elevated startup task before enabling startup from a standard instance, avoiding two competing instances.

## Building and testing

```powershell
cargo build --release
cargo test --workspace
```

The optimized executable is written to `target\release\window-switcher.exe`.

## Performance diagnostics

Release builds contain no performance-timing instrumentation by default. Enable optional measurements for window enumeration, icon resolution and painting with:

```powershell
cargo build --release --features perf
```

Set `[log] level = debug` and configure `[log] path` to collect those measurements. The inspection tool can benchmark repeated enumeration and report latency percentiles:

```powershell
cargo run --release -p inspect-windows -- --benchmark 100
```

For an end-to-end benchmark, first close any running Window Switcher instance. The benchmark launches the supplied executable, creates a test window, sends Alt+Tab input and closes only the instance it launched:

```powershell
cargo build --release -p benchmark-switcher
cargo run --release -p benchmark-switcher -- .\target\release\window-switcher.exe 500 3
```

The last two arguments are the number of measured open/close cycles and the number of additional `Tab` presses while `Alt` remains held.

## Logs and crash diagnostics

When `[log] path` is configured, operational events are appended to that file. `info` is recommended for normal use; use `debug` temporarily when investigating behavior.

Fatal runtime errors and Rust panics are appended to `window-switcher-crash.log` beside the executable. The report includes the version, process ID, thread, source location and a backtrace, and does not depend on the regular INI logger.

While running, the application maintains `window-switcher-running.marker` beside the executable. A normal shutdown removes it. If the next launch finds the marker, it records an unclean previous shutdown in the crash log.

## Project history and license

This project is derived from [sigoden/window-switcher](https://github.com/sigoden/window-switcher) and includes substantial changes to window enumeration, switching behavior, rendering, diagnostics and resource management.

Window Switcher is distributed under the MIT License. See [LICENSE](LICENSE) for the original copyright notice and license terms.
