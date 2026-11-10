---
name: guictl
description: See and control this computer's desktop GUI with the guictl command-line tool - take screenshots, read text on screen (OCR), click, type, press keys, manage windows, press buttons and fill fields in other applications, pick menu items, handle dialogs, change volume, monitors and the clipboard. Use when a task needs a desktop application that has no command-line or API alternative, when asked to look at the screen, or when asked to operate a GUI program on Windows, KDE Plasma or GNOME (Wayland).
---

# guictl

guictl drives the desktop of the computer it runs on. Every command is a
one-off that prints one JSON object and exits.

## Before the first command

1. Find the binary. Try `guictl --version`. If that fails, look for
   `guictl.exe` (Windows) or `guictl` (Linux) where the user said they put
   it, and run it by its full path. If you still can't find it, ask the user
   rather than searching the whole disk.
2. Run `guictl guide` once per session and read all of it. It is the manual
   for agents: how to check that an action worked, how to deal with dialogs
   and popups, how to find controls, how to wait instead of sleeping, and
   notes for this platform. It is kept in step with the binary, so trust it
   over anything you remember about guictl.
3. If a command fails with code 4 (`permission_denied`) on Linux, the user
   may need to run `guictl setup` once; tell them. On GNOME, a command that
   fails saying guictl's GNOME Shell extension is not running needs the
   same `guictl setup`, and then the user has to log out and back in once.

## The rules that matter most

- `"ok": true` means the request was delivered, not that it worked. Look
  again after every step that matters.
- Prefer `element find` / `element invoke` over clicking coordinates read
  off a screenshot.
- Use `wait ...` commands, not sleeps.
- Never discard someone's work, close windows you didn't open, or kill a
  process without being asked. A person may be using this computer.
- Close the windows and dialogs you opened when you are done, unless the
  user still needs them.

`guictl <command> --help` lists a command's flags; `guictl capabilities`
shows what works on this machine.
