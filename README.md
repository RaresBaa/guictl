# guictl

Control a desktop GUI from the command line, for AI agents. Every command is a
one-off that prints a JSON result and exits; there is no server to run.

Supported platforms: Windows 10 1809+ / Windows 11, and Linux with KDE Plasma 6
(Wayland session only). Run `guictl --help` for the commands. What is missing or imperfect is listed
under [Not implemented and known gaps](#not-implemented-and-known-gaps).

## What it can do

- **See the screen:** take a screenshot of the desktop, a monitor, a window or
  a region, optionally with a coordinate grid; find or read text on screen with
  OCR.
- **Mouse and keyboard:** move, click, drag and scroll; type text, press keys
  and key combinations.
- **Windows:** list, focus, move, resize, minimize, maximize, close, keep on
  top, and send to another monitor or virtual desktop; see that one has frozen
  and end its process.
- **UI elements:** list a window's buttons, fields and menu items through the
  accessibility tree, with field values and check states, then click or
  activate them, read or set their text, set check boxes, dropdowns, tree
  nodes and sliders; pick an item from nested menus by its path, e.g.
  `"View>Zoom>Zoom In"`.
- **Dialogs and popups:** see which window a dialog belongs to and which
  windows a dialog is blocking; `launch` reports every window a program opens
  at start (splash screens, tips, licence prompts), not just the first.
- **Waiting:** wait for a window, a window an action just opened, or an element
  to appear or go away, for an element to become enabled or show a value, or
  for the screen to change or stop changing.
- **Clipboard:** read and write text and images.
- **Apps and files:** launch an application; open a file or URL with its
  default handler.
- **Monitors:** list them, change resolution, refresh rate, rotation, position
  and the primary monitor, switch them off and on, set the brightness.
- **Sound and media:** master and per-app volume and mute; list audio devices
  and pick the default; play, pause, skip and read what is playing.
- **Virtual desktops:** list them and switch between them.
- **System:** lock the screen, sleep, keep the computer awake, show a
  notification, see whether the screen is locked.
- **Batches:** run several commands in one call from a JSON file or stdin, or
  keep one process running and feed it commands line by line; optionally stop
  as soon as a person touches the mouse or keyboard.
- **Administrator windows (Windows):** add `--elevate` to run a command as
  administrator, after the person answers the prompt.

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

On NixOS, use the flake's module instead of installing the package alone:

```nix
# inputs.guictl.url = "github:RaresBaa/guictl";
imports = [ inputs.guictl.nixosModules.default ];
```

It installs guictl and sets `services.gnome.at-spi2-core.enable = true`, which
`element` and `menu` need and NixOS leaves off under Plasma. Log in again after
rebuilding, and restart any app that is still missing from `element list`.

To add a user to the `input` group for `batch --stop-on-user-input`, set
`programs.guictl.inputUsers = [ "alice" ];`. Read
[what that allows](#stopping-when-someone-uses-the-computer) first.

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
images. NixOS also lacks the accessibility service; see
[Building](#building) for its module. Package names are Arch's.

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

It takes care of the two permissions Plasma asks for and switches off one
notification. Both permissions are checks made by Plasma, and guictl doesn't
try to get around either: you grant them by running this command and by
accepting the dialog.

**Screenshots** (`screenshot`, `ocr`, `wait idle`). KWin only gives screenshots
to executables that declare the screenshot interface in a `.desktop` file, so
`setup` writes `~/.local/share/applications/guictl.desktop` pointing at the
guictl binary and refreshes Plasma's application cache.

**Mouse and keyboard input** (`mouse`, `keyboard`). `setup` makes Plasma show a
"Remote control" dialog asking whether to allow input control. The dialog names
the program guictl was started from, such as your terminal or the agent's host,
not guictl itself, because Plasma identifies the caller by the application it
was launched from. Accept it once; guictl stores the permission token in
`~/.local/state/guictl/remote-desktop-token`. Starting guictl from a different
program may bring the dialog back.

**The "Remote Control Started" popup.** Every `mouse` or `keyboard` command is
its own remote-control session, and Plasma announces each new session with a
popup, so an agent at work piles them up. A popup doesn't go away while the
pointer is over it. `setup` turns this popup off: it is the same as unticking
"Show a message in a popup" for "Started Remote Desktop" under System Settings >
Notifications > Applications > KDE Portal Integration > Configure Events, and
that is where to turn it back on. It is one Plasma setting, so the popup goes
away for every program that takes remote control, not just guictl. The consent
dialog above is not affected.

### Stopping when someone uses the computer

`batch --stop-on-user-input` is optional and needs one more permission, which
`setup` can't give because it takes root:

```
sudo usermod -aG input $USER
```

Then log in again. Without it, the flag fails with an error that says this, and
everything else works as usual.

**Being in the `input` group lets every program you run read everything you
type, passwords included,** so join only if you want this flag. guictl itself
only notes that a key was pressed or the pointer moved, not which key, and
doesn't stop the input reaching your apps. Its own mouse and keyboard input
doesn't reach these devices, so it doesn't stop itself.

## Not implemented and known gaps

`guictl capabilities` reports what is and isn't implemented on the machine it
runs on, and under `unavailable` what is implemented but can't run there, such
as `element` without an accessibility bus. Commands that aren't implemented
fail with an `unsupported` error (exit code 10).

| Platform | Area | Gap |
|---|---|---|
| Both | User takeover | No on-screen indicator while an agent drives the desktop. Stopping when the user does something only works inside a `batch` with `--stop-on-user-input`. |
| Windows | `workspace`, `window send --workspace` | Windows 10 only. They go through Explorer's undocumented interfaces, which change between builds, and Windows 11 isn't supported yet. |
| Linux | `monitor power` | Works only while a single monitor is attached: `kscreen-doctor --dpms` switches every monitor. |

## License

MIT, see [LICENSE](LICENSE).

The application icon (`guictl/assets/icon.ico`, `icon.svg`) is adapted from
Lucide's `square-mouse-pointer`, ISC licensed:
Copyright (c) 2026 Lucide Contributors (https://lucide.dev).
