//! `guictl guide`: what an agent should know before driving a desktop with
//! this tool - the things `--help` can't say because they're about how the
//! commands behave together, not what their flags are.

use crate::platform::os;

const COMMON: &str = r#"GUICTL GUIDE FOR AI AGENTS

guictl controls this computer's desktop: windows, mouse, keyboard, screen.
Each command prints one JSON object on stdout. Errors go to stderr as
{"error":{"code","message"}} with a non-zero exit code:
  2 invalid arguments   3 target not found   4 permission denied
  5 wait timed out      6 stopped for the user   10 not supported here
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
- Other dialogs (file pickers, confirmations, error messages, permission
  prompts) also appear as new windows and take keyboard focus. In `window
  list` a dialog has "owner": the id of the window it belongs to, and that
  window shows "blocked": true while the dialog waits for an answer. If
  typing or clicking seems to do nothing, look for one. Never leave a
  dialog unanswered behind you: read it, then answer or close it.
- A window showing "not_responding": true is frozen or busy, and input does
  nothing for now. Wait a little; it may recover. `window kill` ends its whole
  process, with every window and any unsaved work, so ask the person first
  unless you started that program yourself.
- Mouse and keyboard input goes to whatever is in front, not to the window
  you have in mind. Run `window focus` first, then click or type. `element
  click` and `menu` bring their window forward themselves.

STARTING PROGRAMS
- Many programs open more than their main window: a splash screen, a tip
  of the day, a licence, login, update or "restore session?" prompt.
  `launch <program> --wait-window` waits until no new window has turned up
  for a second and lists them all under "windows"; "window" is only the
  first to appear. Look at every one. Deal with the popups (read them, then
  dismiss or answer them) before working in the main window, which may be
  "blocked" until you do.
- A program started another way (`open`, a double-click) does the same.
  Run `window list --app <name>` once it is up, not just `window active`.

FINDING THINGS
- Prefer `element find --name <text>` / `element list --role <role>` over
  guessing coordinates from a screenshot: you get exact rectangles, and
  `element invoke` presses a control without moving the mouse.
- Element ids are a path through the window's UI tree. They break when the
  UI changes (a tab opens, a dialog appears). On "not found", find it again.
- `element list` shows a field's current text as "value" and whether a
  check box, radio button, tab or toggle is on as "checked". `element text
  get --id` reads a field's whole text.
- To change a control, say what you want rather than clicking it: `element
  check --id <id> on` sets a check box (and does nothing if it's already on,
  where a click would turn it off), `element select --id <dropdown> "<item>"`
  picks from a dropdown or list, `element expand` and `collapse` work on tree
  nodes, and `element value --id <slider> 30` sets a slider or spinner and
  prints where it ended up. If one reports it isn't supported, click instead.
- `menu --window <id> "File>Export>PNG"` opens each menu in turn and picks
  the item; the names ignore case, & and a trailing "...".
- Not every app exposes elements. When `element` returns nothing useful, use
  `ocr "<text>"` to locate text on screen, or a screenshot. `ocr` matches
  text within one line, so search for a few words, not a whole paragraph.
  Without text, `ocr` prints every line it reads; narrow it with --window
  or --region.
- `element at X Y` tells you what is at a point you saw on a screenshot, and
  gives you its id.
- `element find` prints the best match; --all prints them all, and --index N
  picks one, for a row of buttons with the same name.
- Window selectors work on every window command: --window <id>, --active,
  --title <substring>, --app <name>, --pid <pid>. The first match wins, so
  prefer --window <id> from `window list` when several windows could match.

COORDINATES AND SCREENSHOTS
- Coordinates are real desktop pixels, the same ones `window list`,
  `monitor list` and `element` report.
- `screenshot --scale 0.5` returns a smaller picture. Divide a position you
  read off it by the scale before clicking.
- A position read off a `screenshot --window <id>` image can be clicked
  directly with `mouse click X Y --window <id>`; --monitor works the same.
- `mouse move` takes --duration <ms> to travel instead of jumping. `mouse
  drag` always travels (300 ms by default) and holds the button at the end
  (--hold, 100 ms) before letting go. If an older app still misses a drag,
  raise both, or build it from `mouse press`, `mouse move` and `mouse
  release` in one `batch`.
- `screenshot --window <id>`, `--element <id>` or `--region X,Y,W,H` keeps
  images small; add `--grid` for labelled gridlines if you find positions
  hard to judge.

WAITING
- Don't sleep and hope. `wait window --title <text>` waits for a window to
  appear, `wait element` for a control, `wait idle` for the screen to stop
  changing. All take --timeout in milliseconds and exit with code 5 when it
  runs out.
- `wait idle` never settles while anything animates (a spinner, a video, a
  blinking cursor). Narrow it with --window or --region; --stable <ms> sets
  how long the picture must stay unchanged.
- `wait window --gone` and `wait element --gone` wait for something to go
  away, such as a progress dialog. `wait element --enabled` waits for a
  greyed-out control ("disabled": true) to come alive, and --value <text>
  for a field to show that text.
- A text cursor blinks about every 500 ms, so with the default --stable 600
  a window with a focused text field never settles. Pass a --region that
  leaves the cursor out, or --stable 400.
- An animation slower than --stable can look idle, so a success doesn't
  prove the screen is finished. Raise --stable if something changes slowly.
- To catch the dialog an action opens, put the action and `wait window
  --new` (with --app or --pid) in one `batch`. The wait then reports the
  first window that wasn't there before the action, even if it appeared
  before the wait began.
- To see that an action changed the screen, put it and `wait change` (with
  --window or --region) in one `batch`. Run alone, `wait change` only sees
  what happens after it starts. Anything animating counts as a change.
- Some older apps repaint only on the next input, so a screenshot right
  after a click can show the state before it. If what you see doesn't match
  what you did, click an empty spot or move the mouse, then look again.

SEVERAL STEPS AT ONCE
- `guictl batch` reads a JSON array of argument lists on stdin and runs them
  in order in one call:
    [["window","focus","--app","kwrite"],["keyboard","type","hello"]]
  Every step runs even if an earlier one failed; check "all_ok".
- For a long run, `batch --lines` keeps one guictl running: write one
  argument list (or a list of them) per line to its stdin and read one JSON
  answer per line back.
- Add --stop-on-user-input to a long batch. If someone touches the mouse
  or keyboard, the remaining steps are skipped with code "interrupted"
  instead of clicking in the wrong place. Check the screen, and ask before
  you carry on.

KEYS
- `keyboard type "<text>"` types text. `keyboard key enter` presses a named
  key (enter, tab, escape, space, backspace, delete, up/down/left/right, home,
  end, pageup, pagedown, f1-f12, and more in `keyboard key --help`), or any
  character on the keyboard. `keyboard hotkey ctrl+shift+s` presses a
  combination. + joins the keys, so write ctrl+plus for Ctrl and +.

WHEN YOU ARE DONE
- Close the windows you opened once you no longer need them, so the
  desktop is left as you found it. Before closing one, make sure nothing in
  it is lost: save the work if the task was to produce it, or check that
  it needs no saving. If a "save changes?" dialog appears and you aren't sure,
  cancel it and ask.
- Leave a window open when the person still needs it: they asked you to
  open or show it, it holds a result they will want to look at, or they
  were using it before you started. When in doubt, ask.
- Also close the popups and dialogs you caused (an About box, a finished
  progress dialog, an error you have read).
- Never close windows you didn't open unless you were asked to.

BE CAREFUL
- While the screen is locked, mouse and keyboard commands fail with
  "permission_denied". Tell the person and wait; `system status` shows when
  "locked" turns false.
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
