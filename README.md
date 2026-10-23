# GestureLite for Windows

A lightweight native Windows 11 x64 utility for mouse gestures and keyboard shortcuts. The existing application is distributed unchanged as a portable ZIP. The separate Mac project is [GestureLite-Mac](https://github.com/Leon2323232323232323232323/GestureLite-Mac).

## Download and run

Get **GestureLite-Windows_1.0.10.0_x64-portable.zip** from [Releases](https://github.com/Leon2323232323232323232323/GestureLite-Windows/releases). Download the portable ZIP, rather than GitHub's automatically generated source-code ZIP.

1. Extract the entire ZIP into a permanent folder, then open GestureLite.exe. No installer or administrator privileges are required. The C runtime is statically linked.
2. Close any other running GestureLite instance first. Portable and packaged editions use the same single-instance guard and configuration directory.
3. The native Settings window opens. Choose a trigger, edit directional shortcuts and the optional Click action, then choose **Save and Apply**. Closing Settings leaves the tray utility running. Click the tray icon to reopen Settings; right-click it to disable gestures or exit.

This executable is **unsigned**. Smart App Control or organizational application-control policies may block it; SmartScreen may also warn. A checksum confirms download integrity but does not establish publisher trust. Do not disable security protections or install this project's local development certificates. If your policy blocks it, use an approved signed distribution when available. The Microsoft Store submission has not been published or altered by this GitHub release.

## Gestures and click actions

Hold the configured trigger and move past its distance threshold to perform one directional shortcut immediately. Release before starting another gesture. There are no repeat gestures or click timers. A release without a completed gesture runs the assigned click shortcut, otherwise the configured ordinary tap behavior applies. A completed gesture suppresses that click.

Back/Forward mouse buttons, additional mouse triggers and F13-F24 mappings are supported. Keyboard triggers supplied through G HUB must provide a held key-down/key-up pair. Enter custom shortcuts such as `CTRL+SHIFT+S` or `WIN+CTRL+RIGHT` in the editable shortcut fields. Existing configuration is preserved. On a new profile the current application's default threshold is 18 pixels; it can be adjusted in Settings.

Configuration and operational logs are at `%LOCALAPPDATA%\GestureLite`. The bundled `config.example.json` is documentation; it does not overwrite your saved settings. Open Configuration from the tray to edit the actual file. Do not run packaged and portable copies simultaneously.

To launch quietly later, create a shortcut to GestureLite.exe with `--tray`. To start at sign-in, place that shortcut in your own Windows Startup folder (`Win+R`, `shell:startup`). Startup is optional and is not installed automatically by this ZIP. Remove the shortcut to undo it. Quit before moving/deleting the portable folder. User settings remain until you remove their directory manually.

## Validation and development

The existing source retains WH_MOUSE_LL, WH_KEYBOARD_LL, SendInput, single-gesture-per-press behavior, replay filtering, release cancellation and modifier cleanup. Existing tests cover those engine/planner regressions; passing tests do not guarantee every physical mouse or target application behaves correctly. No live input is injected by the release workflow.

The distribution version is **1.0.10.0**, matching the current Windows development package. Cargo's internal crate version remains 0.1.0. This portable download is independent of the preserved first Microsoft Store package, version 1.0.0.0.

Build with Windows x64, Visual Studio C++ Build Tools/Windows SDK, PowerShell 7 and Rust **1.90.0** with rustfmt/clippy. Run `scripts/package-github.ps1` to check formatting, run Clippy with warnings denied, run the existing tests, build with the lockfile, and verify the portable ZIP. The toolchain is pinned to the existing release rather than changing application code to satisfy newer style lints.

The initial source archive is an upload bootstrap snapshot. After import, CI uses the browsable source tree; source at the release tag identifies the exact GitHub build. No Store MSIX, personal settings, runtime logs, test certificates or private keys are distributed. Dependency license texts are in THIRD_PARTY_NOTICES.txt. No project open-source license has been selected; source visibility does not itself grant an open-source license.
