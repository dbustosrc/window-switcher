# Window Switcher

Window-Switcher offers hotkeys for quickly switching windows on Windows OS:

1. ```Alt+`(Backtick)```: switch between windows of the same app.

![switch-windows](https://github.com/sigoden/window-switcher/assets/4012553/06d387ce-31fd-450b-adf3-01bfcfc4bce3)

2. ```Alt+Tab```: switch between all windows, without grouping them by app. The selected window title is shown below the icons.

![switch-apps](https://github.com/sigoden/window-switcher/assets/4012553/0c74a7ca-3a48-4458-8d2d-b40dc041f067)

**💡 Hold down the `Alt` key and tap the ``` `(Backtick)/Tab ``` key to cycle through windows, Press ```Alt + `(Backtick)/Tab``` and release both keys to switch to the last active window.**

## Installation

1. **Download:** Visit the [Github Release](https://github.com/sigoden/windows-switcher/releases) and download the `windows-switcher.zip` file.
2. **Extract:** Unzip the downloaded file and extract the `window-switcher.exe` to your preferred location.
3. **Launch:** `window-switcher.exe` is a standalone executable, no installation is required, just double-click the file to run it.

For the tech-savvy, here's a one-liner to automate the installation:
```ps1
iwr -useb https://raw.githubusercontent.com/sigoden/window-switcher/main/install.ps1 | iex
```

## Configuration

Window-Switcher offers various customization options to tailor its behavior to your preferences. You can define custom keyboard shortcuts, enable or disable specific features, and fine-tune settings through a configuration file.

To personalize Window-Switcher, you'll need a configuration file named `window-switcher.ini`. This file should be placed in the same directory as the `window-switcher.exe` file. Once you've made changes to the configuration, make sure to restart Window-Switcher so your new settings can take effect.

Here is the default configuration:

```ini
# Whether to show trayicon, yes/no
trayicon = yes 

[switch-windows]

# Hotkey to switch windows
hotkey = alt+`

# List of hotkey conflict apps
# e.g. game1.exe,game2.exe
blacklist =

# Ignore minimal windows
ignore_minimal = no

# Only switch within the current virtual desktops: yes/no/auto
only_current_desktop = auto

[switch-apps]

# Whether to enable switching all windows
enable = yes

# Hotkey to switch all windows
hotkey = alt+tab

# Ignore minimal windows
ignore_minimal = no

# Only switch windows within the current virtual desktops: yes/no/auto
only_current_desktop = auto
```

## Running as Administrator (Optional)

The window-switcher works in standard user mode. But only the window-switcher running in administrator mode can manage applications running in administrator mode.

Windows does not allow a normal-integrity process to intercept keyboard input intended for an elevated window. As a result, when Task Manager or another elevated application is focused, Windows may show its native Alt+Tab selector unless Window-Switcher is also elevated. Supporting this without elevation would require a correctly signed executable installed in a trusted location with `uiAccess`; it cannot be enabled safely by an INI option alone.

**Important:** If you enable the startup option while running in standard user mode, it will launch in standard mode upon system reboot. To ensure startup with admin privileges, launch the window-switcher as administrator first before enabling startup.

When startup is enabled from an elevated Window-Switcher instance, the application uses a Windows Scheduled Task instead of the per-user `Run` registry key. This remains the supported fallback for starting with elevated privileges; it does not make a normal, manually launched instance elevated.

## Performance diagnostics

Normal release builds contain no timing overhead. To enable optional timing for window enumeration, icon resolution and painting, build with:

```ps1
cargo build --release --features perf
```

Configure a log file in `window-switcher.ini` to collect the measurements. The inspection tool can also benchmark repeated enumeration and report p50/p95 latency:

```ps1
cargo run --release -p inspect-windows -- --benchmark 100
```

For an end-to-end benchmark, first close any running Window-Switcher instance. The benchmark launches the supplied executable, creates a stable test window, sends real Alt+Tab input, measures selector visibility and process resources, and closes only the instance it launched:

```ps1
cargo build --release -p benchmark-switcher
cargo run --release -p benchmark-switcher -- .\target\release\window-switcher.exe 500 3
```

The last two arguments are the number of measured open/close cycles and the number of additional Tab presses while Alt remains held.

## Runtime and crash logs

The default configuration writes operational events to `window-switcher.log` beside the executable. Set `[log] level` to `debug` temporarily when investigating behavior; `info` is recommended for normal use.

Fatal startup/runtime errors and Rust panics are also appended to `window-switcher-crash.log` beside the executable, including the version, process ID, thread, source location and a backtrace. This crash log is independent of the INI logger, so configuration and logging initialization failures can still be diagnosed.

While the application is running it maintains `window-switcher-running.marker`. A normal shutdown removes it. If the next launch finds the marker, it records an `unclean previous shutdown` entry in the crash log; this also detects forced or native terminations that cannot execute the Rust panic hook.

## License

Copyright (c) 2023-2025 window-switcher developers.

window-switcher is made available under the terms of the MIT License, at your option.

See the LICENSE files for license details.
