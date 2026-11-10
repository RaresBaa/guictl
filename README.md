# guictl

[![Latest release](https://img.shields.io/github/v/release/RaresBaa/guictl)](https://github.com/RaresBaa/guictl/releases/latest)
[![crates.io](https://img.shields.io/crates/v/guictl)](https://crates.io/crates/guictl)
[![License: MIT](https://img.shields.io/github/license/RaresBaa/guictl)](LICENSE)

Control a desktop GUI from the command line, for AI agents. Every command is a
one-off that prints a JSON result and exits; there is no server to run, unless
you want one: `guictl mcp start` serves the same commands over
[MCP](#as-an-mcp-server).

Supported platforms: Windows 10 1809+ / Windows 11, and Linux with KDE Plasma 6
or GNOME 45+ (Wayland sessions only). macOS is not supported. Run
`guictl --help` for the commands.

## A quick look

Closing a Notepad window that has unsaved text, on Windows (output shortened
where it says `…`):

```
$ guictl window list --app notepad
[{"app":"notepad.exe","id":"0xf131e","pid":13600,"state":"normal","title":"Untitled - Notepad","x":2272,"y":285,"w":1321,"h":707,…}]

$ guictl element find --window 0xf131e --role edit
{"id":"0xf131e/0","name":"Text Editor","role":"edit","value":"","x":2280,"y":336,"w":1305,"h":625}

$ guictl element text set --id 0xf131e/0 "hello"
{"ok":true}

$ guictl window close --window 0xf131e
{"closed":false,"new_windows":[{"app":"notepad.exe","id":"0x1315a4","owner":"0xf131e","title":"Notepad",…}],"ok":true}

$ guictl element list --window 0x1315a4 --role button
[{"id":"0x1315a4/2","name":"Save",…},{"id":"0x1315a4/3","name":"Don't Save",…},{"id":"0x1315a4/4","name":"Cancel",…},…]

$ guictl element invoke --id 0x1315a4/3
{"ok":true}

$ guictl window focus --title "Untitled - Notepad"
{"error":{"code":"not_found","message":"no matching window"}}
```

Results go to stdout and errors to stderr, and the exit code says what kind of
failure it was: 2 invalid arguments, 3 not found, 4 permission denied, 5 wait
timed out, 6 stopped because someone used the computer, 10 not supported here.

### Why not just screenshots and clicks?

An agent can drive a desktop from screenshots alone, guessing where to click.
guictl gives it more to go on:

- **Exact targets.** Controls come from the accessibility tree with their
  rectangles, names, values and check states. `element invoke` presses one
  without moving the mouse, whatever window is in front.
- **Results that say what happened.** `window close` reports whether the window
  closed or which "save changes?" dialog appeared instead. `launch` lists every
  window a program opened, splash screens and prompts included. `window list`
  marks dialogs, the windows they block, and frozen windows.
- **Waiting instead of polling.** Wait for a window, an element, a value or the
  screen to settle, rather than taking a screenshot every second.

Screenshots and OCR are still there for apps that expose nothing.

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
  keep one process running and feed it commands line by line. A batch stops at
  the first step that fails, and optionally as soon as a person touches the
  mouse or keyboard.
- **MCP server:** run guictl in the background as a local MCP server, so an
  MCP client such as Claude Code can call its commands as tools and get
  screenshots back as images.
- **Administrator windows (Windows):** add `--elevate` to run a command as
  administrator, after the person answers the prompt.

### By platform

| Area | Windows | KDE Plasma | GNOME |
|---|---|---|---|
| Screenshots, OCR | ✓ | ✓ | ✓ |
| Mouse and keyboard | ✓ | ✓ | ✓ |
| Windows | ✓ | ✓ | ✓, without "not responding" |
| UI elements, menus | ✓ | ✓ | ✓ |
| Clipboard | ✓ | ✓ | ✓ |
| Launch, open | ✓ | ✓ | ✓ |
| Monitors | ✓; power and brightness need DDC/CI | ✓; power only with one monitor | Changes aren't saved; power only with one monitor |
| Sound and media | ✓ | ✓ | ✓ |
| Virtual desktops | Windows 10 only | ✓ | ✓; ids are positions |
| System | ✓ | ✓, without idle time | ✓ |
| Batches | ✓ | ✓; stopping on user input needs the `input` group | ✓; stopping on user input needs the `input` group |
| MCP server | ✓ | ✓ | ✓ |
| `--elevate` | ✓ | — | — |

The details are under
[Not implemented and known gaps](#not-implemented-and-known-gaps).

## Installing

### From a release

Download the archive for your platform from the
[latest release](https://github.com/RaresBaa/guictl/releases/latest):

| Platform | File |
|---|---|
| Windows (x86-64) | `guictl-<version>-x86_64-windows.zip` |
| Linux (x86-64) | `guictl-<version>-x86_64-linux.tar.gz` |

Unpack it anywhere. It holds the `guictl` binary, this README, the license and
the [agent skill](#as-a-skill); there is nothing else to install. `SHA256SUMS`
in the same release lets you check the download.

Then either tell your agent where it is, for example *"guictl is at
`C:\Tools\guictl\guictl.exe`; run `guictl guide` first"*, or run

```
guictl setup
```

once, so you can type plain `guictl`. On Windows, `setup` adds the folder
holding `guictl.exe` to your user PATH (no administrator needed; running it
again does nothing). Terminals and agents started after that find it; ones
already open need a restart. On Linux, `setup` grants the
[one-time permissions](#one-time-setup) the desktop asks for (on GNOME it also
installs guictl's Shell extension), and you put the binary on your PATH
yourself, e.g. in `~/.local/bin`. Linux also needs a few
[run-time tools](#runtime-dependencies) for OCR and clipboard images.

To update, download the new release and unpack it over the old one. Stop the
MCP server first (`guictl mcp stop`) if it is running, since Windows won't
replace a running program.

### From crates.io

With a Rust toolchain (1.87 or newer):

```
cargo install guictl
```

This builds guictl and puts it in `~/.cargo/bin`, which rustup already adds to
your PATH, so on Windows there is no need for `guictl setup`. On Linux, still
run `guictl setup` once for the [one-time permissions](#one-time-setup), and
add the [run-time tools](#runtime-dependencies) for OCR and clipboard images.
The [agent skill](#as-a-skill) isn't part of the crate; copy it from this
repository or a release archive.

To update, run `cargo install guictl` again, after `guictl mcp stop` if the MCP
server is running. On Linux, run `guictl setup` again too: on GNOME it updates
the Shell extension.

### With Nix

`nix build` gives `result/bin/guictl`, and `nix run github:RaresBaa/guictl`
runs it without a checkout. That build brings its own `tesseract` (English),
`wl-clipboard` and `pactl`. Its path changes with every update, so run
`guictl setup` again after one.

On NixOS, use the flake's module instead of installing the package alone:

```nix
# inputs.guictl.url = "github:RaresBaa/guictl";
imports = [ inputs.guictl.nixosModules.default ];
```

It installs guictl and sets `services.gnome.at-spi2-core.enable = true`, which
`element` and `menu` need and NixOS leaves off under Plasma (GNOME has it on
already). Log in again after rebuilding, and restart any app that is still
missing from `element list`.

To add a user to the `input` group for `batch --stop-on-user-input`, set
`programs.guictl.inputUsers = [ "alice" ];`. Read
[what that allows](#stopping-when-someone-uses-the-computer) first.

### On Arch

`makepkg -si` in [packaging/arch/](packaging/arch/) builds and installs a
package with the run-time tools as dependencies. It fetches the source from
GitHub, not from the checkout.

## Pointing an agent at it

Tell the agent that `guictl` is available and to run `guictl guide` first.
That prints a short plain-text briefing: how to check that an action worked,
what to do when a "save changes?" or other dialog appears, how to find controls
and wait for things, and notes specific to the platform it is running on.
`guictl --help` ends with the same pointer, so an agent that starts there finds
it too.

### As a skill

[skills/guictl/SKILL.md](skills/guictl/SKILL.md) is an
[Agent Skill](https://agentskills.io) that does this for you: the agent picks
guictl up by itself whenever a task needs the desktop. It is also in every
release archive. For Claude Code, copy the `guictl` folder into
`~/.claude/skills/` (every project) or `.claude/skills/` in one project. Other
agents that read skills take the same folder in their own skills directory.

The skill only says when to use guictl and to read `guictl guide` first, so it
stays correct when guictl is updated; the guide itself ships in the binary.

### As an MCP server

For a client that speaks the [Model Context Protocol](https://modelcontextprotocol.io)
rather than running commands, guictl can serve itself over MCP's HTTP transport:

```
guictl mcp start     # starts it in the background, prints its URL, returns
guictl mcp status    # running or not, and where
guictl mcp stop      # stops it
```

The URL is `http://127.0.0.1:47800/mcp`; `--port` on `start` picks another
port. Register it with the client, e.g. for Claude Code:

```
claude mcp add --transport http guictl http://127.0.0.1:47800/mcp
```

Add `--scope user` to have it in every project rather than just the current
one. The client connects when it starts, so start the server first; it keeps
running until you stop it, but nothing starts it again after you log out or
restart.

The server has two tools. `guictl` runs one command, given as its argument
list (`["window", "list"]`), and returns the same JSON the command prints;
a `screenshot` without `--output` comes back as an image. A failed command
comes back as a tool error holding `{"error":{"code","message"}}`.
`guictl_batch` runs several in order, like `guictl batch`, and stops at the
first one that fails. The guide is sent to the client as the server's
instructions, so the agent doesn't need to run `guictl guide` itself.
Commands run one at a time.

It only listens on 127.0.0.1 and refuses requests from web pages, but any
program on the computer can use it while it runs, the same as it could run
guictl itself. `guictl mcp serve` runs it in the foreground instead, for
debugging; the background server's output goes to `guictl-mcp.log` in the
temp directory (`$XDG_RUNTIME_DIR` on Linux). `--elevate` isn't available
through the server: to drive administrator windows on Windows, start the
server from an administrator terminal.

## Safety

An agent using guictl can do anything you could do at the keyboard: close
windows, type into any program, delete files through Explorer. Things to
know before you let one loose:

- **There is no on-screen indicator** that an agent is driving the desktop
  (GNOME shows its own remote-control icon in the top bar while input is sent).
- **guictl runs as you**, with your rights. It doesn't ask before acting; the
  agent's own permission settings are what stand between it and your desktop.
  On Windows it can reach administrator windows only through `--elevate`, which
  shows the system prompt every time and needs a person to answer it.
- **Taking over:** with `batch --stop-on-user-input`, a batch stops as soon as
  you move the mouse or press a key, and exits with code 6. Outside a batch,
  nothing stops a command that is already running. On Linux this flag needs
  the [`input` group](#stopping-when-someone-uses-the-computer).
- **Permissions on Linux** go through the desktop's own dialogs, which a person
  has to accept; guictl doesn't try to get around any of them.
- **The MCP server** listens only on 127.0.0.1 and refuses requests from web
  pages, but any program on the computer can use it while it runs. Stop it
  with `guictl mcp stop` when you're done.
- **The guide tells agents** not to discard unsaved work, not to end processes
  they didn't start, and to answer dialogs rather than leave them open. That is
  advice for the agent, not something guictl enforces.

## Linux requirements

guictl supports **KDE Plasma 6** and **GNOME 45 or newer**, in a Wayland
session. Other desktops are out of scope: most commands go through the
compositor (KWin, or GNOME Shell), and Wayland gives ordinary programs no
common way to manage windows or capture the screen.

### Runtime dependencies

A normal Plasma or GNOME install already has everything except OCR, and on
Plasma clipboard images. NixOS also lacks the accessibility service under
Plasma; see [With Nix](#with-nix) for its module. Package names are Arch's.

On Arch, the things to add are:

```
sudo pacman -S tesseract tesseract-data-eng
sudo pacman -S wl-clipboard   # Plasma only, for clipboard images
```

`ocr` uses English when its language pack is installed and otherwise falls back
to whichever pack is.

### One-time setup

Run this once after installing:

```
guictl setup
```

What it does depends on the desktop. Every permission it involves is a check
the desktop makes, and guictl doesn't try to get around any: you grant them by
running this command and by accepting the dialog.

#### On KDE Plasma

It takes care of the two permissions Plasma asks for and switches off one
notification.

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

#### On GNOME

GNOME Shell lets no outside program list or move windows, switch workspaces,
take a screenshot without a dialog, or keep something on the clipboard after
it exits. guictl comes with a small GNOME Shell extension that does these
things when guictl asks.

**The extension** (`window`, `workspace`, `screenshot`, `ocr`, `wait`,
`clipboard`, `element`, `menu`, and `mouse` for the pointer position). `setup`
writes it to `~/.local/share/gnome-shell/extensions/guictl@guictl.github.io`
and adds it to GNOME's enabled extensions. **GNOME Shell loads a new extension
only at login, so log out and back in once** after the first `setup`, and after
a guictl update that changes the extension (commands then say so). The
extension answers only the executables listed in its `allowed-executables`
file, which `setup` fills with the path of the guictl that ran it. Run `setup`
again if the binary moves. To remove the extension, delete that folder or turn
it off in the Extensions app.

**Mouse and keyboard input** (`mouse`, `keyboard`). `setup` makes GNOME show its
"Remote Desktop" dialog asking whether to allow input control. Accept it once;
guictl stores the permission token in
`~/.local/state/guictl/remote-desktop-token`. While a command runs, GNOME shows
its remote-control indicator in the top bar.

`monitor set` on GNOME applies until a monitor is plugged in or out or you log
in again. GNOME asks a person to confirm a lasting change and undoes it when
nobody answers, so guictl doesn't make one.

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
fail with an `unsupported` error (exit code 10). `guictl guide` lists the
smaller quirks of each platform, such as apps that need a flag before they
expose their controls.

| Platform | Area | Gap |
|---|---|---|
| Both | User takeover | No on-screen indicator while an agent drives the desktop. Stopping when the user does something only works inside a `batch` with `--stop-on-user-input`. |
| Windows | `workspace`, `window send --workspace` | Windows 10 only. They go through Explorer's undocumented interfaces, which change between builds, and Windows 11 isn't supported yet. |
| Windows | `monitor power`, `brightness` | Need DDC/CI, so they don't work on most laptop panels or on monitors with DDC/CI switched off. |
| Linux | `monitor power` | Works only while a single monitor is attached: Plasma and GNOME both switch every monitor at once. |
| Linux (GNOME) | `monitor set` | Not saved: lasts until a monitor is plugged in or out or the next login. |
| Linux (GNOME) | `window list` | Never reports `not_responding`. |
| Linux (GNOME) | `workspace` | GNOME gives workspaces no ids, so guictl uses their position, which changes when GNOME removes an empty workspace before it (it does so by itself, unless dynamic workspaces are turned off). |
| Linux (Plasma) | `system status` | No `idle_ms`: Plasma doesn't tell programs how long the user has been idle. |

## Building

```
cargo build --release
```

The binary is `target/release/guictl`. Building needs a Rust toolchain (1.87 or
newer) and nothing else: on Linux guictl links no system libraries, it talks to
the desktop over D-Bus and through a few command-line tools at run time.

```
cargo test
```

runs the unit tests. They cover the logic that needs no desktop (argument
parsing, ids, regions, monitor modes, OCR text matching) and run anywhere.

On an operating system other than Windows or Linux guictl still builds, and
every command that touches the desktop fails with an `unsupported` error
(exit code 10). `RUSTFLAGS="--cfg guictl_stub" cargo check` compiles that
fallback on a supported system.

## License

MIT, see [LICENSE](LICENSE).

The application icon (`guictl/assets/icon.ico`, `icon.svg`) is adapted from
Lucide's `square-mouse-pointer`, ISC licensed:
Copyright (c) 2026 Lucide Contributors (https://lucide.dev).
