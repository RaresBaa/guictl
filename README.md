# guictl

Control a desktop GUI from the command line, for AI agents. Every command is a
one-off that prints a JSON result and exits; there is no server to run.

Supported platforms: Windows 10 1809+ / Windows 11, and Linux with KDE Plasma 6
(Wayland session only). Run `guictl --help` for the commands. What is missing or imperfect is listed
under [Not implemented and known gaps](#not-implemented-and-known-gaps).

## What it can do

- **See the screen:** take a screenshot of the desktop, a monitor, a window or
  a region, optionally with a coordinate grid; find text on screen with OCR.
- **Mouse and keyboard:** move, click, drag and scroll; type text, press keys
  and key combinations.
- **Windows:** list, focus, move, resize, minimize, maximize, close, keep on
  top, and send to another monitor or virtual desktop.
- **UI elements:** list a window's buttons, fields and menu items through the
  accessibility tree, then click or activate them and read or set their text.
- **Waiting:** wait for a window or an element to appear, or for the screen to
  stop changing.
- **Clipboard:** read and write text and images.
- **Apps and files:** launch an application; open a file or URL with its
  default handler.
- **Monitors:** list them, change resolution, refresh rate, rotation, position
  and the primary monitor, switch them off and on, set the brightness.
- **Sound and media:** master and per-app volume and mute; list audio devices
  and pick the default; play, pause, skip and read what is playing.
- **Virtual desktops:** list them and switch between them.
- **System:** lock the screen, sleep, keep the computer awake, show a
  notification.
- **Batches:** run several commands in one call from a JSON file or stdin.

Not all of it works on both platforms yet; see
[Not implemented and known gaps](#not-implemented-and-known-gaps).

## Building

```
cargo build --release
```

The binary is `target/release/guictl`. Building needs a Rust toolchain (1.87 or
newer) and nothing else: on Linux guictl links no system libraries, it talks to
the desktop over D-Bus and through a few command-line tools at run time.

With Nix, `nix build` gives `result/bin/guictl`, and `nix run github:RaresBaa/guictl`
runs it without a checkout. That build brings its own `tesseract` (English),
`wl-clipboard` and `pactl`. Its path changes with every update, so run
`guictl setup` again after one.

On Arch, `makepkg -si` in [packaging/arch/](packaging/arch/) builds and installs a
package with the run-time tools as dependencies. It fetches the source from
GitHub, not from the checkout.

```
cargo test
```

runs the unit tests. They cover the logic that needs no desktop (argument
parsing, ids, regions, monitor modes, OCR text matching) and run anywhere.

On an operating system other than Windows or Linux guictl still builds, and
every command that touches the desktop fails with an `unsupported` error
(exit code 10). `RUSTFLAGS="--cfg guictl_stub" cargo check` compiles that
fallback on a supported system.

## Pointing an agent at it

Tell the agent that `guictl` is available and to run `guictl guide` first.
That prints a short plain-text briefing: how to check that an action worked,
what to do when a "save changes?" or other dialog appears, how to find controls
and wait for things, and notes specific to the platform it is running on.
`guictl --help` ends with the same pointer, so an agent that starts there finds
it too.

## Linux requirements

guictl only supports **KDE Plasma 6**. Other desktops are out of scope: most
commands go through KWin and Plasma services that don't exist elsewhere.

### Runtime dependencies

A normal Plasma install already has everything except OCR and clipboard
images. Package names are Arch's.

On Arch, the two things to add are:

```
sudo pacman -S tesseract tesseract-data-eng wl-clipboard
```

`ocr` uses English when its language pack is installed and otherwise falls back
to whichever pack is.

### One-time setup

Run this once after installing:

```
guictl setup
```

It takes care of the two permissions Plasma asks for. Both are checks made by
Plasma, and guictl doesn't try to get around either: you grant them by running
this command and by accepting the dialog.

**Screenshots** (`screenshot`, `ocr`, `wait idle`). KWin only gives screenshots
to executables that declare the screenshot interface in a `.desktop` file, so
`setup` writes `~/.local/share/applications/guictl.desktop` pointing at the
guictl binary and refreshes Plasma's application cache.

**Mouse and keyboard input** (`mouse`, `keyboard`). `setup` makes Plasma show a
"Remote control" dialog asking whether to let guictl control input. Accept it
once; guictl stores the permission token in
`~/.local/state/guictl/remote-desktop-token`.

## Not implemented and known gaps

`guictl capabilities` reports what is and isn't implemented on the machine it
runs on. Commands that aren't implemented fail with an `unsupported` error
(exit code 10).

| Platform | Area | Gap |
|---|---|---|
| Both | User takeover | While an agent drives the desktop, show an on-screen indicator and stop as soon as the user does anything. |
| Windows | `workspace`, `window send --workspace` | **Not implemented.** |
| Windows | `monitor set --enabled` | **Not implemented.** |
| Windows | `monitor power` | **Not implemented.** |
| Windows | Programs running as administrator | A guictl that isn't itself elevated can't act on their windows, and Windows blocks it silently, so commands such as `window state restore` still return `ok`. |
| Windows | `brightness` | Works over DDC/CI only. |
| Windows | `audio device set-default` / `enable` / `disable` | Use the undocumented `IPolicyConfig` interface (CLSID `CPolicyConfigClient`, IID `f8679f50-850a-41cf-9c72-430f290290c8`). |
| Linux | Desktops | Only KDE Plasma 6 in a Wayland session. The X11 session and other desktops are not supported. |
| Linux | `keyboard hold`, `mouse press` | A key or button can't stay held between separate commands. Put the hold and the matching release in one `batch`. |
| Linux | `keyboard type` | Relies on KWin to turn each character into key presses; a character the current keyboard layout can't produce fails with an error. |
| Linux | `window move` / `resize` / `state` | Return before the window has its new geometry, so reading it back at once can show the old values. |
| Linux | `monitor set --position` | Can't be negative: KScreen keeps the desktop's top-left corner at 0,0. |
| Linux | `monitor power` | Works only while a single monitor is attached: `kscreen-doctor --dpms` switches every monitor. |
| Linux | `audio device enable` / `disable` | Switch the sound card's profile, since PulseAudio has no per-device switch. Disabling a device disables every input and output on its card. |
| Linux | `element` | Switches on the session's accessibility support the first time it is used, and it stays on until logout. Apps built on Chromium or Electron (VS Code, Chrome) expose nothing unless started with `--force-renderer-accessibility`. |
| Linux | `element` rectangles | Correct for elements in the window itself, but not for those in an open popup menu, which is a separate surface. |
| Linux | `element click` | Refuses elements that aren't drawn (items of closed menus, scrolled-away rows; they have `"showing": false`), since it would hit whatever is drawn there. `element invoke` works on them. |
| Linux | `element text set` | Some fields report success and change nothing (Firefox address bar). guictl reads the text back and, if it didn't take, focuses the field and types instead. |
| Linux | `clipboard set --image` | `wl-copy` leaves a small process behind that serves the image until something else is copied. |
| Both | `element click` | Doesn't bring the window to the front first; run `window focus` before it. |
| Both | `window close` | Only asks the window to close. If the app shows a "save changes?" dialog instead, the result has `"closed": false` and lists the dialog under `new_windows`. |
| Both | `wait idle` | Never settles while anything in view animates, even a small spinner; narrow it with `--region`, `--window` or `--monitor`. It takes three identical captures 300 ms apart, so an animation slower than that can slip through. |

## License

MIT, see [LICENSE](LICENSE).

The application icon (`guictl/assets/icon.ico`, `icon.svg`) is adapted from
Lucide's `square-mouse-pointer`, ISC licensed:
Copyright (c) 2026 Lucide Contributors (https://lucide.dev).
