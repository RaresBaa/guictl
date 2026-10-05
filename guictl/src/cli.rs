use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "guictl",
    version,
    about = "Control a desktop GUI from the command line, for AI agents.",
    after_help = "AI agents: run `guictl guide` before anything else. It explains how to use these\ncommands well and what to watch out for (dialogs, focus, waiting, coordinates)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// Indent JSON output instead of printing it compact on one line
    #[arg(long, global = true)]
    pub pretty: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Monitor list, mode, power
    #[command(subcommand)]
    Monitor(MonitorCmd),
    /// Capture the desktop, a monitor, a window, or a region
    Screenshot(ScreenshotArgs),
    /// Mouse move, click, drag, scroll
    #[command(subcommand)]
    Mouse(MouseCmd),
    /// Keyboard text and key input
    #[command(subcommand)]
    Keyboard(KeyboardCmd),
    /// List, focus, move, resize windows
    #[command(subcommand)]
    Window(WindowCmd),
    /// Accessibility tree: find and operate on UI elements
    #[command(subcommand)]
    Element(ElementCmd),
    /// Clipboard text and image
    #[command(subcommand)]
    Clipboard(ClipboardCmd),
    /// Master and per-app volume
    #[command(subcommand)]
    Volume(VolumeCmd),
    /// Audio device listing and selection
    #[command(subcommand)]
    Audio(AudioCmd),
    /// Monitor brightness
    #[command(subcommand)]
    Brightness(BrightnessCmd),
    /// Media playback control
    #[command(subcommand)]
    Media(MediaCmd),
    /// Virtual desktops
    #[command(subcommand)]
    Workspace(WorkspaceCmd),
    /// System power and session
    #[command(subcommand)]
    System(SystemCmd),
    /// Send a desktop notification
    Notify(NotifyArgs),
    /// Open a file or URL with the default handler
    Open(OpenArgs),
    /// Launch an application
    Launch(LaunchArgs),
    /// Report what works in the current session
    Capabilities,
    /// How to use guictl well: read this first if you are an AI agent
    Guide,
    /// Grant the one-time permissions the desktop asks for; run once after installing
    Setup,
    /// Wait for a window, element, or idle screen
    #[command(subcommand)]
    Wait(WaitCmd),
    /// Run several commands from a JSON file or stdin
    Batch(BatchArgs),
    /// Locate text on screen (OCR)
    Ocr(OcrArgs),
}

// ---- shared selectors ----

#[derive(Args, Clone, Default)]
pub struct WindowSelector {
    /// Window id from `window list`
    #[arg(long, value_name = "ID")]
    pub window: Option<String>,
    /// The window that currently has focus
    #[arg(long)]
    pub active: bool,
    /// First window whose title contains this text, ignoring case
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// First window of this application (its process name)
    #[arg(long, value_name = "NAME")]
    pub app: Option<String>,
}

#[derive(Args, Clone, Default)]
pub struct MonitorSelector {
    /// Monitor id from `monitor list`
    #[arg(long, value_name = "ID")]
    pub monitor: Option<String>,
    /// The primary monitor
    #[arg(long)]
    pub primary: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum WindowState {
    Minimize,
    Maximize,
    Restore,
    Fullscreen,
}

// ---- monitor ----

#[derive(Subcommand)]
pub enum MonitorCmd {
    /// List monitors with their id, position, size and refresh rate
    List,
    /// Change a monitor's mode, rotation, position or role
    Set {
        #[command(flatten)]
        selector: MonitorSelector,
        /// New resolution in physical pixels, e.g. 1920x1080
        #[arg(long, value_name = "WxH")]
        resolution: Option<String>,
        /// New refresh rate in Hz, rounded, e.g. 60
        #[arg(long, value_name = "HZ")]
        refresh_rate: Option<u32>,
        /// 0, 90, 180 or 270; 90 puts the top of the picture on the screen's left edge
        #[arg(long, value_name = "DEGREES")]
        rotation: Option<u32>,
        /// Top-left corner of the monitor on the desktop
        #[arg(long, value_name = "X,Y", allow_hyphen_values = true)]
        position: Option<String>,
        /// Make this the primary monitor
        #[arg(long)]
        make_primary: bool,
        /// Attach the monitor to the desktop or detach it
        #[arg(long)]
        enabled: Option<OnOff>,
    },
    /// Switch a monitor's panel on or off
    Power {
        /// Monitor id from `monitor list`
        #[arg(long)]
        id: String,
        state: OnOff,
    },
}

// ---- screenshot ----

#[derive(Args)]
pub struct ScreenshotArgs {
    /// Capture only this monitor (id from `monitor list`); default is the whole desktop
    #[arg(long, value_name = "ID")]
    pub monitor: Option<String>,
    /// Capture only this window (id from `window list`)
    #[arg(long, value_name = "ID")]
    pub window: Option<String>,
    /// Capture only this rectangle, in desktop pixels
    #[arg(long, value_name = "X,Y,W,H", allow_hyphen_values = true)]
    pub region: Option<String>,
    /// Resize the image, e.g. 0.5 for half size; divide positions read off it by this
    #[arg(long, value_name = "FACTOR")]
    pub scale: Option<f32>,
    /// Draw the mouse pointer into the image
    #[arg(long)]
    pub cursor: bool,
    /// Overlay gridlines labelled with desktop pixel coordinates
    #[arg(long)]
    pub grid: bool,
    /// Required. A file path for the PNG (prints path, width, height), or `-` for raw PNG on stdout
    #[arg(long, short = 'o', value_name = "PATH")]
    pub output: Option<String>,
}

// ---- mouse ----

#[derive(Subcommand)]
pub enum MouseCmd {
    /// Move the pointer; coordinates are desktop pixels unless a window or --monitor is given
    Move {
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
        #[command(flatten)]
        window: WindowSelector,
        /// Measure X Y from this monitor's top-left corner (id from `monitor list`)
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
        /// Travel there over this many milliseconds instead of jumping
        #[arg(long, value_name = "MS")]
        duration: Option<u64>,
    },
    /// Click at X Y, or where the pointer is when they are left out
    Click {
        #[arg(allow_negative_numbers = true)]
        x: Option<i32>,
        #[arg(allow_negative_numbers = true)]
        y: Option<i32>,
        #[arg(long, default_value = "left")]
        button: MouseButton,
        /// Double-click
        #[arg(long)]
        double: bool,
        #[command(flatten)]
        window: WindowSelector,
        /// Measure X Y from this monitor's top-left corner (id from `monitor list`)
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
    },
    /// Press a button and keep it down, after moving to X Y if given; pair with `mouse release` in one `batch`
    Press {
        #[arg(allow_negative_numbers = true)]
        x: Option<i32>,
        #[arg(allow_negative_numbers = true)]
        y: Option<i32>,
        #[arg(long)]
        button: MouseButton,
        #[command(flatten)]
        window: WindowSelector,
        /// Measure X Y from this monitor's top-left corner (id from `monitor list`)
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
    },
    /// Let go of a button held by `mouse press`
    Release {
        #[arg(long)]
        button: MouseButton,
    },
    /// Press at X1 Y1, move to X2 Y2 and release
    Drag {
        #[arg(allow_negative_numbers = true)]
        x1: i32,
        #[arg(allow_negative_numbers = true)]
        y1: i32,
        #[arg(allow_negative_numbers = true)]
        x2: i32,
        #[arg(allow_negative_numbers = true)]
        y2: i32,
        #[arg(long, default_value = "left")]
        button: MouseButton,
        /// Spend this many milliseconds on the move; some drag targets ignore an instant jump
        #[arg(long, value_name = "MS")]
        duration: Option<u64>,
        #[command(flatten)]
        window: WindowSelector,
        /// Measure the coordinates from this monitor's top-left corner (id from `monitor list`)
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
    },
    /// Turn the mouse wheel at the pointer's position
    Scroll {
        /// Number of wheel notches
        #[arg(long, default_value_t = 1)]
        amount: i32,
        #[arg(long, default_value = "down")]
        direction: ScrollDirection,
    },
    /// Print the pointer's position in desktop pixels
    Position,
}

// ---- keyboard ----

#[derive(Subcommand)]
pub enum KeyboardCmd {
    /// Type text into whatever has keyboard focus
    Type {
        /// The text to type
        text: String,
    },
    /// Press and release one key
    Key {
        /// A character, or a name: enter, tab, escape, space, backspace, delete, up, down, left, right, home, end, pageup, pagedown, f1-f12
        key: String,
        /// Press it this many times
        #[arg(long, default_value_t = 1)]
        repeat: u32,
    },
    /// Press a key combination
    Hotkey {
        /// Keys joined by +, modifiers first: ctrl, shift, alt, win. E.g. ctrl+shift+s
        combo: String,
    },
    /// Press a key and keep it down; pair with `keyboard release` in one `batch`
    Hold {
        /// Key name as for `keyboard key`, or a modifier: ctrl, shift, alt, win
        key: String,
    },
    /// Let go of a key held by `keyboard hold`
    Release {
        /// Key name as for `keyboard hold`
        key: String,
    },
}

// ---- window ----

#[derive(Subcommand)]
pub enum WindowCmd {
    /// List windows front to back, with id, title, app, pid, position and size
    List {
        /// Only windows of this application (its process name)
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
    },
    /// Print the window that has focus
    Active,
    /// Bring a window to the front and give it keyboard focus
    Focus {
        #[command(flatten)]
        selector: WindowSelector,
    },
    /// Move a window's top-left corner to X Y in desktop pixels
    Move {
        #[command(flatten)]
        selector: WindowSelector,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    /// Resize a window to W by H pixels
    Resize {
        #[command(flatten)]
        selector: WindowSelector,
        w: i32,
        h: i32,
    },
    /// Minimize, maximize, restore or fullscreen a window
    State {
        #[command(flatten)]
        selector: WindowSelector,
        state: WindowState,
    },
    /// Ask a window to close; the result says whether it did ("closed") and lists any dialog it opened instead ("new_windows")
    Close {
        #[command(flatten)]
        selector: WindowSelector,
    },
    /// Keep a window above all others, or stop doing so
    Pin {
        #[command(flatten)]
        selector: WindowSelector,
        state: OnOff,
    },
    /// Move a window to another monitor or workspace; give exactly one of the two
    Send {
        #[command(flatten)]
        selector: WindowSelector,
        /// Target monitor (id from `monitor list`)
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
        /// Target workspace (id from `workspace list`)
        #[arg(long, value_name = "ID")]
        workspace: Option<String>,
    },
}

// ---- element ----

#[derive(Subcommand)]
pub enum ElementCmd {
    /// List a window's UI elements with id, role, name and rectangle
    List {
        #[command(flatten)]
        selector: WindowSelector,
        /// Only elements with this role, as `element list` prints it (e.g. button)
        #[arg(long)]
        role: Option<String>,
    },
    /// Print the first element matching --name and/or --role
    Find {
        #[command(flatten)]
        selector: WindowSelector,
        /// Text the element's name contains, ignoring case
        #[arg(long, value_name = "TEXT")]
        name: Option<String>,
        /// The element's role, as `element list` prints it (e.g. button)
        #[arg(long)]
        role: Option<String>,
    },
    /// Click the middle of an element with the mouse; focus its window first
    Click {
        /// Element id from `element list` or `element find`
        #[arg(long)]
        id: String,
    },
    /// Activate an element (press a button, run a menu item) without using the mouse
    Invoke {
        /// Element id from `element list` or `element find`
        #[arg(long)]
        id: String,
    },
    /// Read or replace an element's text
    #[command(subcommand)]
    Text(ElementTextCmd),
}

#[derive(Subcommand)]
pub enum ElementTextCmd {
    /// Print an element's text or value
    Get {
        /// Element id from `element list` or `element find`
        #[arg(long)]
        id: String,
    },
    /// Replace an element's text
    Set {
        /// Element id from `element list` or `element find`
        #[arg(long)]
        id: String,
        /// The new text
        text: String,
    },
}

// ---- clipboard ----

#[derive(Subcommand)]
pub enum ClipboardCmd {
    /// Print the clipboard's text, or save its image with --image --output
    Get {
        /// Read an image instead of text
        #[arg(long)]
        image: bool,
        /// Where to write the image; required with --image
        #[arg(long, value_name = "PATH")]
        output: Option<String>,
    },
    /// Put text or an image on the clipboard; give one of the two
    Set {
        /// The text to copy
        text: Option<String>,
        /// Copy this image file instead of text
        #[arg(long, value_name = "PATH")]
        image: Option<String>,
    },
}

// ---- volume / audio ----

#[derive(Subcommand)]
pub enum VolumeCmd {
    /// Print the volume, 0-100
    Get {
        /// One application's volume instead of the master volume
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
    },
    /// Set the volume
    Set {
        /// One application's volume instead of the master volume
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// 0-100
        level: u32,
    },
    /// Mute or unmute
    Mute {
        /// One application instead of the master output
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        state: OnOff,
    },
}

#[derive(Subcommand)]
pub enum AudioCmd {
    /// List, select, enable or disable audio devices
    #[command(subcommand)]
    Device(AudioDeviceCmd),
}

#[derive(Subcommand)]
pub enum AudioDeviceCmd {
    /// List input and output devices with id, direction and state
    List,
    /// Make a device the default for its direction
    SetDefault {
        /// Device id from `audio device list`
        #[arg(long)]
        id: String,
    },
    /// Enable a disabled device
    Enable {
        /// Device id from `audio device list`
        #[arg(long)]
        id: String,
    },
    /// Disable a device
    Disable {
        /// Device id from `audio device list`
        #[arg(long)]
        id: String,
    },
}

// ---- brightness ----

#[derive(Subcommand)]
pub enum BrightnessCmd {
    /// Print a monitor's brightness, 0-100
    Get {
        /// Monitor id from `monitor list`
        #[arg(long, value_name = "ID")]
        monitor: String,
    },
    /// Set a monitor's brightness
    Set {
        /// Monitor id from `monitor list`
        #[arg(long, value_name = "ID")]
        monitor: String,
        /// 0-100
        level: u32,
    },
}

// ---- media ----

#[derive(Subcommand)]
pub enum MediaCmd {
    /// Start playback; does nothing if already playing
    Play,
    /// Pause playback; does nothing if already paused
    Pause,
    /// Skip to the next track
    Next,
    /// Go back to the previous track
    Previous,
    /// Print the current track's title, artist, album, status and player
    NowPlaying,
}

// ---- workspace ----

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// List virtual desktops
    List,
    /// Switch to a virtual desktop
    Switch {
        /// Workspace id from `workspace list`, or its position counting from 1
        id: String,
    },
}

// ---- system ----

#[derive(Subcommand)]
pub enum SystemCmd {
    /// Lock the screen
    Lock,
    /// Put the computer to sleep
    Sleep,
    /// Keep the computer from sleeping; `on --duration <ms>` blocks for that long, then lets go
    KeepAwake {
        state: OnOff,
        /// How long to stay awake, in milliseconds; required with `on`
        #[arg(long, value_name = "MS")]
        duration: Option<u64>,
    },
}

#[derive(Args)]
pub struct NotifyArgs {
    /// The notification's heading
    #[arg(long)]
    pub title: String,
    /// The notification's text
    #[arg(long)]
    pub body: String,
}

#[derive(Args)]
pub struct OpenArgs {
    /// A file path or URL
    pub target: String,
}

#[derive(Args)]
pub struct LaunchArgs {
    /// The program to start; prints its pid
    pub program: String,
    /// Arguments for the program, after `--`
    #[arg(last = true)]
    pub args: Vec<String>,
    /// Wait for the program's window and print it too
    #[arg(long)]
    pub wait_window: bool,
    /// How long --wait-window waits, in milliseconds [default: 5000]
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

// ---- agent-specific ----

#[derive(Subcommand)]
pub enum WaitCmd {
    /// Wait until a window matching --title and/or --app exists, and print it
    Window {
        /// Text the window's title contains, ignoring case
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        /// The window's application (its process name)
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// Give up after this many milliseconds, with exit code 5 [default: 5000]
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
    },
    /// Wait until a window has an element matching --name and/or --role, and print it
    Element {
        #[command(flatten)]
        selector: WindowSelector,
        /// Text the element's name contains, ignoring case
        #[arg(long, value_name = "TEXT")]
        name: Option<String>,
        /// The element's role, as `element list` prints it (e.g. button)
        #[arg(long)]
        role: Option<String>,
        /// Give up after this many milliseconds, with exit code 5 [default: 5000]
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
    },
    /// Wait until the screen stops changing; anything animating in view keeps it waiting, so narrow the area
    Idle {
        /// Watch only this monitor (id from `monitor list`); default is the whole desktop
        #[arg(long, value_name = "ID")]
        monitor: Option<String>,
        /// Watch only this window (id from `window list`)
        #[arg(long, value_name = "ID")]
        window: Option<String>,
        /// Watch only this rectangle, in desktop pixels
        #[arg(long, value_name = "X,Y,W,H", allow_hyphen_values = true)]
        region: Option<String>,
        /// Give up after this many milliseconds, with exit code 5 [default: 5000]
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
    },
}

#[derive(Args)]
pub struct BatchArgs {
    /// Read the commands from this file instead of stdin. The input is a JSON array of argument lists, e.g. [["window","focus","--app","kwrite"],["keyboard","type","hello"]]
    #[arg(long, value_name = "PATH")]
    pub file: Option<String>,
}

#[derive(Args)]
pub struct OcrArgs {
    /// Text to look for, ignoring case; prints its rectangle in desktop pixels
    pub text: String,
    /// Search only this monitor (id from `monitor list`); default is the whole desktop
    #[arg(long, value_name = "ID")]
    pub monitor: Option<String>,
    /// Search only this window (id from `window list`)
    #[arg(long, value_name = "ID")]
    pub window: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("guictl").chain(args.iter().copied()))
    }

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    /// `--help` is the only command reference, so nothing may be left bare.
    /// Plain coordinates and on/off-style choices explain themselves.
    #[test]
    fn everything_has_help_text() {
        const SELF_EVIDENT: &[&str] = &["x", "y", "x1", "y1", "x2", "y2", "w", "h", "state", "button", "direction"];
        fn check(cmd: &clap::Command, path: &str) {
            for arg in cmd.get_arguments() {
                let id = arg.get_id().as_str();
                assert!(arg.get_help().is_some() || SELF_EVIDENT.contains(&id), "{path}: {id} has no help");
            }
            for sub in cmd.get_subcommands() {
                let path = format!("{path} {}", sub.get_name());
                assert!(sub.get_about().is_some(), "{path} has no description");
                check(sub, &path);
            }
        }
        check(&Cli::command(), "guictl");
    }

    #[test]
    fn negative_coordinates_need_no_separator() {
        assert!(matches!(
            parse(&["mouse", "move", "-100", "50"]).unwrap().command,
            Command::Mouse(MouseCmd::Move { x: -100, y: 50, .. })
        ));
        assert!(matches!(
            parse(&["mouse", "drag", "-5", "-6", "7", "8"]).unwrap().command,
            Command::Mouse(MouseCmd::Drag { x1: -5, y1: -6, x2: 7, y2: 8, .. })
        ));
        match parse(&["screenshot", "--region", "-1920,0,800,600"]).unwrap().command {
            Command::Screenshot(args) => assert_eq!(args.region.as_deref(), Some("-1920,0,800,600")),
            _ => panic!("not a screenshot"),
        }
    }

    #[test]
    fn mouse_commands_take_a_window_or_monitor() {
        match parse(&["mouse", "click", "5", "6", "--app", "dolphin", "--double"]).unwrap().command {
            Command::Mouse(MouseCmd::Click { x, y, double, window, monitor, .. }) => {
                assert_eq!((x, y, double), (Some(5), Some(6), true));
                assert_eq!(window.app.as_deref(), Some("dolphin"));
                assert!(monitor.is_none());
            }
            _ => panic!("not a click"),
        }
        assert!(parse(&["mouse", "click"]).is_ok());
        assert!(parse(&["mouse", "move", "1", "2", "--monitor", "DP-1", "--duration", "300"]).is_ok());
    }

    #[test]
    fn play_and_pause_are_separate_commands() {
        assert!(matches!(parse(&["media", "play"]).unwrap().command, Command::Media(MediaCmd::Play)));
        assert!(matches!(parse(&["media", "pause"]).unwrap().command, Command::Media(MediaCmd::Pause)));
    }

    #[test]
    fn pretty_is_accepted_anywhere() {
        assert!(parse(&["--pretty", "window", "list"]).unwrap().pretty);
        assert!(parse(&["window", "list", "--pretty"]).unwrap().pretty);
        assert!(!parse(&["window", "list"]).unwrap().pretty);
    }

    #[test]
    fn bad_input_is_rejected() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["nonsense"]).is_err());
        assert!(parse(&["mouse", "move", "1"]).is_err());
        assert!(parse(&["mouse", "click", "--button", "fourth"]).is_err());
        assert!(parse(&["monitor", "power", "--id", "DP-1", "maybe"]).is_err());
    }
}
