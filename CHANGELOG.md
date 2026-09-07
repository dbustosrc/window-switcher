# Changelog

All notable changes to this independent Window Switcher project are documented here.

## [2.0.0] - 2026-09-06

This is the first stable release of the independent project. It is based on upstream Window Switcher 1.19.0 and substantially changes the Alt+Tab model, rendering path, performance characteristics and diagnostics.

### Classic per-window switching

- Changed `Alt+Tab` to list every independent top-level window instead of grouping windows by application.
- Preserved `Alt+Backtick` for cycling through windows belonging to the same logical application.
- Preserved global window ordering and reverse navigation with `Shift`.
- Kept separate documents and independent application sessions as separate entries even when they share a process.
- Added safe validation immediately before activation so windows closed during a selector session are skipped.
- Removed the blanket exclusion of topmost windows.

### Modal dialogs

- Collapsed a disabled owner and its active blocking modal dialog into one selector entry.
- Used the dialog title when available so modal entries remain distinguishable from independent application windows.
- Kept the owner application's icon while activating the dialog that can actually receive input.
- Avoided grouping unrelated windows by validating the root-owner relationship, enabled popup and process identity.

### Interface and rendering

- Added the selected window title below the icon strip, centered with ellipsis when necessary.
- Replaced aliased text rendering with GDI+ `AntiAliasGridFit`.
- Changed the title font to reusable `Segoe UI Semibold` for improved legibility.
- Added per-monitor DPI awareness v2 and DPI-scaled layout resources.
- Protected layout calculations against zero or negative dimensions when many windows are open.
- Retained the simple horizontal selector while allowing icons to shrink safely when required.
- Cached theme information and refreshed it when Windows reports a relevant settings change.

### Window detection and compatibility

- Switched the small-window filter to current visual bounds, with safe fallbacks, so maximized windows are not excluded because of a stale restore size.
- Preserved legitimate minimized windows unless `ignore_minimal` is enabled.
- Improved handling of cloaked windows and Windows virtual desktops.
- Added `auto` virtual-desktop behavior that follows the Windows Alt+Tab setting.
- Preserved delegated `ApplicationFrameHost`, browser profile and PWA handling.
- Documented why an elevated Window Switcher instance is required to intercept shortcuts over elevated applications such as Task Manager.

### Responsiveness and resource usage

- Rendered the static icon strip once per Alt+Tab session and updated only the selection indicator and title while cycling.
- Reused the session DC, bitmap and GDI+ resources, releasing them when the selector closes.
- Added fast provisional icons and moved slower icon resolution off the interface thread.
- Reduced synchronous icon timeouts and avoided unnecessary retry delays.
- Coalesced rapid `Tab` input so only the latest selection needs to be painted.
- Replaced lock-based keyboard-hook state with atomics and early returns for unrelated keys.
- Delivered hook actions through posted window messages to keep the callback short.
- Added bounded caches for process metadata, AUMID data and active application icons.
- Protected process-cache entries against PID reuse by recording process creation time.
- Reused window vectors and reduced repeated string cloning and intermediate collections.
- Replaced linear owner-window searches with a map built during enumeration.
- Replaced the sleeping tray-icon retry thread with a single Windows timer.
- Removed unused dependencies, unsafe global state and obsolete message paths.

### Reliability and diagnostics

- Added RAII cleanup for process handles, icons, GDI objects, bitmaps, device contexts and GDI+ resources.
- Added operational logging with configurable `off`, `error`, `warn`, `info`, `debug` and `trace` levels.
- Added a crash log independent of INI logger initialization, including version, PID, thread, source location and backtrace.
- Added a running-session marker to detect crashes and forced native termination on the next launch.
- Added normal-shutdown logging and single-instance protection.
- Configured elevated Scheduled Task startup to continue running when switching to battery power and removed its execution time limit.
- Preserved user configuration when updating through `install.ps1`.

### Development and validation

- Added optional zero-cost-disabled performance spans behind the `perf` feature.
- Added an enumeration benchmark reporting first-run, p50, p95 and maximum latency plus handle/GDI deltas.
- Added an end-to-end Alt+Tab benchmark that measures repeated selector sessions and resource stability.
- Added tests for configuration parsing, index wrapping, layout safety, modal relationships, window geometry, shutdown markers and GDI object stability.
- Validated 500 repeated selector sessions without growth in GDI or USER objects.
- Validated a prolonged 1,500-cycle run without linear private-memory growth.

### Packaging

- Added stable release packages for Windows x64, Windows x86 and Windows ARM64.
- Included `window-switcher.exe`, `window-switcher.ini`, `README.md` and `LICENSE` in every archive.
- Added SHA-256 checksum files for every downloadable archive.

### Known limitations

- A standard-integrity instance cannot replace Alt+Tab while an elevated application has focus; run Window Switcher as administrator for that scenario.
- Very rapid Alt+Tab input has rarely allowed the native Windows selector to appear above the custom selector. The hook remains active and the condition has not been reproduced consistently.

[2.0.0]: https://github.com/dbustosrc/window-switcher/releases/tag/v2.0.0
