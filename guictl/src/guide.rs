//! `guictl guide`: what an agent should know before driving a desktop with
//! this tool - the things `--help` can't say because they're about how the
//! commands behave together, not what their flags are.

use crate::platform::os;

const COMMON: &str = r#"GUICTL GUIDE FOR AI AGENTS

guictl controls this computer's desktop: windows, mouse, keyboard, screen.
Each command prints one JSON object on stdout. Errors go to stderr as
{"error":{"code","message"}} with a non-zero exit code:
  2 invalid arguments   3 target not found   4 permission denied
  5 wait timed out      10 not supported here
`guictl <command> --help` shows flags. `guictl capabilities` lists what works
on this machine.

LOOK BEFORE AND AFTER YOU ACT
- "ok" means the request was delivered, not that it had the effect you wanted.
  After anything that matters, check: `window list`, `window active`,
  `element find`, or a screenshot.
- `window close` only asks the window to close, and its result tells you what
  happened: "closed": true, or "closed": false with the dialog the app put up
  instead (usually "save changes?") under "new_windows". Find that dialog's
  buttons with `element list --window <its id>` and press one with
  `element invoke`. Do not discard someone's work unless you were told to.
- Other dialogs (file pickers, confirmations, permission prompts) also appear
  as new windows and take keyboard focus. If typing or clicking seems to do
  nothing, run `window list` and look for one.
- Mouse and keyboard input goes to whatever is in front, not to the window
  you have in mind. Run `window focus` first, then click or type. That
  includes `element click`, which is a real mouse click at the element's
  position and does not bring its window forward.

FINDING THINGS
- Prefer `element find --name <text>` / `element list --role <role>` over
  guessing coordinates from a screenshot: you get exact rectangles, and
  `element invoke` presses a control without moving the mouse.
- Element ids are a path through the window's UI tree. They break when the
  UI changes (a tab opens, a dialog appears). On "not found", find it again.
- Not every app exposes elements. When `element` returns nothing useful, use
  `ocr "<text>"` to locate text on screen, or a screenshot. `ocr` matches
  text within one line, so search for a few words, not a whole paragraph.
- Window selectors work on every window command: --window <id>, --active,
  --title <substring>, --app <name>. The first match wins, so prefer
  --window <id> from `window list` when several windows could match.

COORDINATES AND SCREENSHOTS
- Coordinates are real desktop pixels, the same ones `window list`,
  `monitor list` and `element` report.
- `screenshot --scale 0.5` returns a smaller picture. Divide a position you
  read off it by the scale before clicking.
- A position read off a `screenshot --window <id>` image can be clicked
  directly with `mouse click X Y --window <id>`; --monitor works the same.
- `mouse move` and `mouse drag` take --duration <ms> to travel instead of
  jumping, which some drag targets need.
- `screenshot --window <id>` or `--region X,Y,W,H` keeps images small; add
  `--grid` for labelled gridlines if you find positions hard to judge.

WAITING
- Don't sleep and hope. `wait window --title <text>` waits for a window to
  appear, `wait element` for a control, `wait idle` for the screen to stop
  changing. All take --timeout in milliseconds and exit with code 5 when it
  runs out.
- `wait idle` never settles while anything animates (a spinner, a video, a
  blinking cursor). Narrow it with --window or --region.
- `launch <program> --wait-window` starts an app and waits for its window.

SEVERAL STEPS AT ONCE
- `guictl batch` reads a JSON array of argument lists on stdin and runs them
  in order in one call:
    [["window","focus","--app","kwrite"],["keyboard","type","hello"]]
  Every step runs even if an earlier one failed; check "all_ok".

KEYS
- `keyboard type "<text>"` types text. `keyboard key enter` presses a named
  key (enter, tab, escape, space, backspace, delete, up/down/left/right, home,
  end, pageup, pagedown, f1-f12). `keyboard hotkey ctrl+shift+s` presses a
  combination.

BE CAREFUL
- A person may be using this computer. `system sleep`, `system lock`,
  `monitor power`, `monitor set`, `window close` and `clipboard set` all
  affect them directly; use them only when the task calls for it.
"#;

pub fn text() -> String {
    format!("{COMMON}\n{}", os::GUIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_has_the_common_and_platform_parts() {
        let guide = text();
        assert!(guide.starts_with("GUICTL GUIDE FOR AI AGENTS"));
        assert!(guide.contains("ON THIS SYSTEM"));
    }

    #[test]
    fn guide_fits_a_terminal() {
        for line in text().lines() {
            assert!(line.chars().count() <= 80, "longer than 80 columns: {line}");
        }
    }
}
