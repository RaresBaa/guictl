use super::dbus;
use super::window::{self, WindowHandle};
use crate::cli::MouseButton;
use crate::output::CmdError;
use serde::Serialize;
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Type};

/// Depth/count caps keep `list`/`find` bounded on apps with huge UI trees,
/// as on Windows. The result cap is higher here because Qt lists every item
/// of every closed menu, which alone runs to several hundred elements.
/// `MAX_VISITED` also bounds a filtered walk that matches little: every node
/// costs a few D-Bus round trips. GTK 4 nests its widgets deeply: every box
/// is a level, and so is every part of a control.
const MAX_DEPTH: usize = 64;
const MAX_RESULTS: usize = 2000;
const MAX_VISITED: usize = 5000;

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const ACTION: &str = "org.a11y.atspi.Action";
const TEXT: &str = "org.a11y.atspi.Text";
const EDITABLE_TEXT: &str = "org.a11y.atspi.EditableText";
const VALUE: &str = "org.a11y.atspi.Value";
const SELECTION: &str = "org.a11y.atspi.Selection";

/// What AT-SPI hands back in place of "no such object".
const NULL_PATH: &str = "/org/a11y/atspi/null";
/// Extents relative to the element's top-level window. Screen coordinates
/// (type 0) aren't available to clients on Wayland.
const COORD_WINDOW: u32 = 1;
/// Bits of `ATSPI_STATE_SHOWING`, `_CHECKED`, `_ENABLED` and `_SENSITIVE` in
/// the first word of the state set, and of `ATSPI_STATE_CHECKABLE` in the
/// second. GTK 4 never sets `_ENABLED`: it only clears `_SENSITIVE` on a
/// greyed out control, where Qt and GTK 3 clear both.
const STATE_SHOWING: u32 = 1 << 25;
const STATE_ACTIVE: u32 = 1 << 1;
const STATE_CHECKED: u32 = 1 << 4;
const STATE_ENABLED: u32 = 1 << 8;
const STATE_SENSITIVE: u32 = 1 << 24;
const STATE_SELECTED: u32 = 1 << 23;
const STATE_EXPANDABLE: u32 = 1 << 9;
const STATE_EXPANDED: u32 = 1 << 10;
const STATE_EDITABLE: u32 = 1 << 7;
const STATE_FOCUSED: u32 = 1 << 12;
const STATE_VISIBLE: u32 = 1 << 30;
const STATE_CHECKABLE: u32 = 1 << (41 - 32);
const STATE_READ_ONLY: u32 = 1 << (43 - 32);
/// `ATSPI_RELATION_CONTROLLER_FOR`: a web combobox's aria-controls.
const RELATION_CONTROLLER_FOR: u32 = 3;
/// Stands in a Firefox element's text for each child, where its text continues.
const EMBEDDED_CHILD: char = '\u{fffc}';
/// Roles whose contents `element list` shows as `value`. Only these are
/// asked: each question is a round trip to the app, and a listing can hold
/// hundreds of elements.
const VALUE_ROLES: &[&str] = &[
    "text", "text box", "entry", "password text", "combo box", "spin button", "spin box", "slider",
    "progress bar",
];
/// Roles that can be on or off. GTK 3 doesn't set `_CHECKABLE` on them.
const CHECK_ROLES: &[&str] = &[
    "check box", "checkbox", "radio button", "toggle button", "check menu item", "radio menu item",
];
/// Values longer than this are cut short in listings; `element text get`
/// reads the whole thing.
const MAX_VALUE_CHARS: i32 = 200;
/// What can be chosen in a list or dropdown.
const ITEM_ROLES: &[&str] =
    &["list item", "menu item", "check menu item", "radio menu item", "tree item", "table cell"];

#[derive(Serialize)]
pub struct ElementInfo {
    pub id: String,
    pub name: String,
    pub role: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// False for elements that exist but aren't drawn, like the items of a
    /// closed menu. Those can be invoked but not clicked.
    pub showing: bool,
    /// The current contents of a field, combo box, spinner or slider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Whether a check box, radio button or toggle is on; absent for
    /// controls that can't be checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Set on a control that is greyed out and ignores input for now.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

/// One object in an application's accessibility tree: the application's name
/// on the accessibility bus plus an object path.
#[derive(Clone)]
pub(super) struct Node {
    dest: String,
    path: String,
}

pub(super) struct Bus(Connection);

impl Bus {
    fn call<B, R>(&self, node: &Node, interface: &str, method: &str, body: &B) -> zbus::Result<R>
    where
        B: Serialize + DynamicType,
        R: serde::de::DeserializeOwned + Type,
    {
        let reply = self.0.call_method(
            Some(node.dest.as_str()),
            node.path.as_str(),
            Some(interface),
            method,
            body,
        )?;
        let body = reply.body();
        body.deserialize()
    }

    fn property<R>(&self, node: &Node, interface: &str, name: &str) -> zbus::Result<R>
    where
        R: TryFrom<OwnedValue>,
        R::Error: Into<zbus::Error>,
    {
        let value: OwnedValue =
            self.call(node, "org.freedesktop.DBus.Properties", "Get", &(interface, name))?;
        R::try_from(value).map_err(Into::into)
    }

    pub(super) fn children(&self, node: &Node) -> Vec<Node> {
        self.call::<_, Vec<(String, OwnedObjectPath)>>(node, ACCESSIBLE, "GetChildren", &())
            .unwrap_or_default()
            .into_iter()
            .map(|(dest, path)| Node { dest, path: path.to_string() })
            .collect()
    }

    pub(super) fn name(&self, node: &Node) -> String {
        self.property(node, ACCESSIBLE, "Name").unwrap_or_default()
    }

    pub(super) fn role(&self, node: &Node) -> String {
        self.call(node, ACCESSIBLE, "GetRoleName", &()).unwrap_or_default()
    }

    fn rect(&self, node: &Node) -> (i32, i32, i32, i32) {
        self.call(node, COMPONENT, "GetExtents", &(COORD_WINDOW,)).unwrap_or_default()
    }

    fn state(&self, node: &Node) -> [u32; 2] {
        let words = self.call::<_, Vec<u32>>(node, ACCESSIBLE, "GetState", &()).unwrap_or_default();
        [words.first().copied().unwrap_or(0), words.get(1).copied().unwrap_or(0)]
    }

    pub(super) fn showing(&self, node: &Node) -> bool {
        is_showing(self.state(node))
    }

    /// Whether the element is on screen and has a place there to click.
    fn drawn(&self, node: &Node) -> bool {
        self.showing(node) && laid_out(self.rect(node))
    }

    fn gecko(&self, node: &Node) -> bool {
        self.toolkit(node) == "Gecko"
    }

    /// The elements a web combobox's aria-controls points at.
    fn controlled(&self, node: &Node) -> Vec<Node> {
        self.call::<_, Vec<(u32, Vec<(String, OwnedObjectPath)>)>>(node, ACCESSIBLE, "GetRelationSet", &())
            .unwrap_or_default()
            .into_iter()
            .filter(|(kind, _)| *kind == RELATION_CONTROLLER_FOR)
            .flat_map(|(_, targets)| targets)
            .map(|(dest, path)| Node { dest, path: path.to_string() })
            .collect()
    }

    /// The item a Firefox `<select>` has chosen, which it marks; the select
    /// itself is named after its label.
    fn selected_item(&self, node: &Node, depth: usize) -> Option<String> {
        if depth == 0 {
            return None;
        }
        self.children(node).iter().find_map(|child| {
            if ITEM_ROLES.contains(&self.role(child).as_str()) && self.state(child)[0] & STATE_SELECTED != 0 {
                return Some(self.name(child));
            }
            self.selected_item(child, depth - 1)
        })
    }

    /// The text an unnamed element holds. A page's paragraphs and sections
    /// have nothing else to go by.
    fn own_text(&self, node: &Node) -> Option<String> {
        if !self.implements(node, TEXT) {
            return None;
        }
        let text = self.value(node)?.replace(EMBEDDED_CHILD, " ");
        Some(text.trim().to_string()).filter(|text| !text.is_empty())
    }

    /// An element's whole text, with its children's text where Firefox only
    /// leaves a placeholder for them.
    fn full_text(&self, node: &Node, budget: &mut usize) -> Option<String> {
        if !self.implements(node, TEXT) {
            return None;
        }
        let text: String = self.call(node, TEXT, "GetText", &(0i32, -1i32)).ok()?;
        if !text.contains(EMBEDDED_CHILD) {
            return Some(text);
        }
        let children = self.children(node);
        let mut children = children.iter();
        let mut out = String::new();
        for c in text.chars() {
            if c != EMBEDDED_CHILD {
                out.push(c);
                continue;
            }
            let Some(child) = children.next() else { continue };
            if *budget == 0 {
                continue;
            }
            *budget -= 1;
            let role = self.role(child);
            let inline = ["link", "static"].contains(&role.as_str());
            // A frame has no text of its own, only the document inside it.
            let inner = if role == "internal frame" { self.children(child).into_iter().next() } else { None };
            let piece = self
                .full_text(inner.as_ref().unwrap_or(child), budget)
                .or_else(|| self.value(child))
                .unwrap_or_else(|| self.name(child));
            let gap = if inline { " " } else { "\n" };
            out.push_str(gap);
            out.push_str(&piece);
            out.push_str(gap);
        }
        // Nesting leaves empty lines behind.
        let lines: Vec<&str> = out.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        Some(lines.join("\n"))
    }

    /// The start of a field's text, or a slider's number; None for elements
    /// without a Text or Value interface.
    fn value(&self, node: &Node) -> Option<String> {
        if self.implements(node, TEXT) {
            // Firefox answers with nothing when asked for more than there is.
            let count = self.property::<i32>(node, TEXT, "CharacterCount").unwrap_or(MAX_VALUE_CHARS + 1);
            let text: String = self.call(node, TEXT, "GetText", &(0i32, count.min(MAX_VALUE_CHARS + 1))).ok()?;
            return Some(match text.char_indices().nth(MAX_VALUE_CHARS as usize) {
                Some((cut, _)) => format!("{}…", &text[..cut]),
                None => text,
            });
        }
        if self.implements(node, VALUE) {
            return self.property::<f64>(node, VALUE, "CurrentValue").ok().map(|v| v.to_string());
        }
        // Toolkits other than Firefox name a dropdown after its current item.
        if self.role(node) == "combo box" {
            if self.gecko(node) {
                return self.selected_item(node, 3);
            }
            return Some(self.name(node)).filter(|name| !name.is_empty());
        }
        None
    }

    /// The toolkit that made the element's application, like "Qt" or "GTK".
    fn toolkit(&self, node: &Node) -> String {
        let app = Node { dest: node.dest.clone(), path: "/org/a11y/atspi/accessible/root".into() };
        self.property(&app, "org.a11y.atspi.Application", "ToolkitName").unwrap_or_default()
    }

    fn implements(&self, node: &Node, interface: &str) -> bool {
        self.call::<_, Vec<String>>(node, ACCESSIBLE, "GetInterfaces", &())
            .is_ok_and(|all| all.iter().any(|i| i == interface))
    }

    fn pid(&self, dest: &str) -> Option<u32> {
        self.0
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetConnectionUnixProcessID",
                &(dest,),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()
    }

    /// The applications registered on the accessibility bus that belong to `pid`.
    fn apps(&self, pid: u32) -> Vec<Node> {
        let registry = Node {
            dest: "org.a11y.atspi.Registry".into(),
            path: "/org/a11y/atspi/accessible/root".into(),
        };
        self.children(&registry)
            .into_iter()
            .filter(|app| self.pid(&app.dest) == Some(pid))
            .collect()
    }
}

fn no_bus(e: impl std::fmt::Display) -> CmdError {
    CmdError::unsupported(format!(
        "could not reach the accessibility bus (is at-spi2-core installed? On NixOS, set services.gnome.at-spi2-core.enable = true): {e}"
    ))
}

/// Whether anything provides the accessibility bus, asked without starting it.
pub(super) fn bus_exists() -> bool {
    let Ok(session) = dbus::session() else {
        return false;
    };
    ["ListNames", "ListActivatableNames"].iter().any(|method| {
        session
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                *method,
                &(),
            )
            .and_then(|reply| reply.body().deserialize::<Vec<String>>())
            .is_ok_and(|names| names.iter().any(|name| name == "org.a11y.Bus"))
    })
}

/// Connects to the accessibility bus, which is separate from the session
/// bus. Toolkits only build their trees while the session's accessibility
/// flag is on, so this turns it on; the returned bool says whether it had to,
/// in which case running apps need a moment to show up. The flag stays on
/// until the session ends.
fn connect() -> Result<(Bus, bool), CmdError> {
    const SERVICE: &str = "org.a11y.Bus";
    const PATH: &str = "/org/a11y/bus";
    const STATUS: &str = "org.a11y.Status";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

    let session = dbus::session()?;
    let enabled = session
        .call_method(Some(SERVICE), PATH, Some(PROPERTIES), "Get", &(STATUS, "IsEnabled"))
        .and_then(|reply| reply.body().deserialize::<OwnedValue>())
        .map_err(no_bus)?;
    let was_enabled = bool::try_from(enabled).unwrap_or(false);
    if !was_enabled {
        session
            .call_method(
                Some(SERVICE),
                PATH,
                Some(PROPERTIES),
                "Set",
                &(STATUS, "IsEnabled", zbus::zvariant::Value::from(true)),
            )
            .map_err(no_bus)?;
    }
    let address: String = session
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetAddress", &())
        .and_then(|reply| reply.body().deserialize())
        .map_err(no_bus)?;
    let conn = zbus::blocking::connection::Builder::address(address.as_str())
        .and_then(|builder| builder.build())
        .map_err(no_bus)?;
    Ok((Bus(conn), !was_enabled))
}

/// A window's accessible object and where its contents start on screen.
pub(super) struct Root {
    pub bus: Bus,
    pub node: Node,
    origin: (i32, i32),
}

/// Picks a window's accessible among those of its application. Going by the
/// wrong one would act on another window, so two that nothing tells apart are
/// an error. Only the active one can be recognised without a title or a size
/// to go by; there's no position on Wayland.
fn pick_frame(
    bus: &Bus,
    frames: &[Node],
    handle: WindowHandle,
    target: &window::ClientArea,
) -> Result<Node, CmdError> {
    // Qt pads the separator in its accessible title ("Doc  — KWrite") but
    // not in the window caption, so compare the words only.
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = words(&target.title);
    let same_size = |f: &&Node| {
        let (_, _, w, h) = bus.rect(f);
        (w, h) == (target.w, target.h) || (w, h) == (target.frame_w, target.frame_h)
    };
    let titled: Vec<&Node> = frames.iter().filter(|f| words(&bus.name(f)) == title).collect();
    let sized: Vec<&Node> = titled.iter().filter(|f| same_size(f)).copied().collect();
    let pool = if !sized.is_empty() {
        sized
    } else if !titled.is_empty() {
        titled
    } else {
        // The app's own idea of the title differs, as it may while it changes.
        let by_size: Vec<&Node> = frames.iter().filter(same_size).collect();
        if !by_size.is_empty() { by_size } else { frames.iter().filter(|f| bus.showing(f)).collect() }
    };
    let pool = if pool.is_empty() { frames.iter().collect() } else { pool };
    let Some(first) = pool.first() else {
        return Err(CmdError::unsupported(
            "the window's application exposes no accessibility tree (Chromium and Electron \
             apps need --force-renderer-accessibility; some apps only pick up accessibility \
             when started after it was turned on)",
        ));
    };
    if pool.len() == 1 {
        return Ok((*first).clone());
    }
    let is_active = window::active().is_ok_and(|w| w.id == window::format_id(handle));
    let active = is_active.then(|| pool.iter().find(|f| bus.state(f)[0] & STATE_ACTIVE != 0)).flatten();
    active.map(|f| (*f).clone()).ok_or_else(|| {
        CmdError::invalid_args(
            "another window of this application has the same title and size, and there's no telling \
             them apart unless this one has focus; focus it first with `window focus`",
        )
    })
}

/// Finds the accessible that stands for a KWin window. The two sides share
/// no id, so the application is matched by pid and its top-level by title,
/// falling back to size and then to whichever one is showing.
pub(super) fn root(handle: WindowHandle) -> Result<Root, CmdError> {
    let target = window::client_area(handle)?;
    let (bus, just_enabled) = connect()?;

    let deadline = Instant::now() + Duration::from_secs(if just_enabled { 3 } else { 0 });
    let frames = loop {
        let frames: Vec<Node> = bus
            .apps(target.pid)
            .iter()
            .flat_map(|app| bus.children(app))
            .collect();
        if !frames.is_empty() || Instant::now() >= deadline {
            break frames;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let node = pick_frame(&bus, &frames, handle, &target)?;
    // A toolkit that draws its own shadow may measure from the visible frame
    // (GTK 4) or from the surface around it (GTK 3). Its top-level says which:
    // one the size of the frame gives away where the frame is. Otherwise, as
    // under a KWin title bar, it measures from the surface.
    let (x, y, w, h) = bus.rect(&node);
    let origin = if (w, h) == (target.frame_w, target.frame_h) {
        (target.frame_x - x, target.frame_y - y)
    } else {
        (target.x, target.y)
    };
    Ok(Root { bus, node, origin })
}

/// Whether a rectangle is a real place. Elements that aren't laid out report
/// a size of -1 (Firefox) or a position billions of pixels away (GTK 3).
/// Firefox keeps the items of a closed menu "showing", but not "visible".
fn is_showing(state: [u32; 2]) -> bool {
    state[0] & STATE_SHOWING != 0 && state[0] & STATE_VISIBLE != 0
}

fn laid_out((x, y, w, h): (i32, i32, i32, i32)) -> bool {
    const FAR: i32 = 1 << 24;
    w > 0 && h > 0 && (-FAR..FAR).contains(&x) && (-FAR..FAR).contains(&y)
}

fn disabled(state: [u32; 2]) -> bool {
    state[0] & (STATE_ENABLED | STATE_SENSITIVE) == 0
}

fn checkable(state: [u32; 2], role: &str) -> bool {
    state[1] & STATE_CHECKABLE != 0 || CHECK_ROLES.contains(&role)
}

struct Walk<'a> {
    bus: &'a Bus,
    window_id: &'a str,
    role_filter: Option<&'a str>,
    name_filter: Option<String>,
    /// Stop at the first match whose whole name is the wanted text.
    stop_at_exact: bool,
    found_exact: bool,
    visited: usize,
    out: Vec<ElementInfo>,
}

impl Walk<'_> {
    fn done(&self) -> bool {
        self.out.len() >= MAX_RESULTS
            || self.visited >= MAX_VISITED
            || (self.stop_at_exact && self.found_exact)
    }

    fn collect(&mut self, parent: &Node, path: &mut Vec<usize>, origin: (i32, i32)) {
        if path.len() > MAX_DEPTH {
            return;
        }
        for (index, child) in self.bus.children(parent).iter().enumerate() {
            if self.done() {
                return;
            }
            self.visited += 1;
            path.push(index);
            let role = self.bus.role(child);
            let name = self.bus.name(child);
            let is_value_role = VALUE_ROLES.contains(&role.as_str());
            let role_ok = self.role_filter.is_none_or(|r| role.eq_ignore_ascii_case(r));
            let text = if role_ok && name.is_empty() && !is_value_role { self.bus.own_text(child) } else { None };
            let name_ok = self.name_filter.as_ref().is_none_or(|n| {
                name.to_lowercase().contains(n) || text.as_ref().is_some_and(|t| t.to_lowercase().contains(n))
            });
            if role_ok && name_ok {
                // A field's label goes by the same name as the field.
                self.found_exact |= self.name_filter.as_ref().is_none_or(|n| name.to_lowercase() == *n && role != "label");
                let rect = self.bus.rect(child);
                let placed = laid_out(rect);
                let (x, y, w, h) = if placed { (origin.0 + rect.0, origin.1 + rect.1, rect.2, rect.3) } else { (0, 0, 0, 0) };
                let id = format!(
                    "{}/{}",
                    self.window_id,
                    path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
                );
                let state = self.bus.state(child);
                let showing = placed && is_showing(state);
                let checked = checkable(state, &role).then_some(state[0] & STATE_CHECKED != 0);
                let value = if is_value_role { self.bus.value(child) } else { text };
                self.out.push(ElementInfo {
                    id,
                    name,
                    role,
                    x,
                    y,
                    w,
                    h,
                    showing,
                    value,
                    checked,
                    disabled: disabled(state),
                });
            }
            self.collect(child, path, origin);
            path.pop();
        }
    }
}

fn search(
    handle: WindowHandle,
    role: Option<&str>,
    name: Option<&str>,
    stop_at_exact: bool,
) -> Result<Vec<ElementInfo>, CmdError> {
    let root = root(handle)?;
    let window_id = window::format_id(handle);
    let mut walk = Walk {
        bus: &root.bus,
        window_id: &window_id,
        role_filter: role,
        name_filter: name.map(str::to_lowercase),
        stop_at_exact,
        found_exact: false,
        visited: 0,
        out: Vec::new(),
    };
    walk.collect(&root.node, &mut Vec::new(), root.origin);
    Ok(walk.out)
}

pub fn list(window: WindowHandle, role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
    search(window, role, None, false)
}

/// How well an element answers to a name, lowest first: a whole name beats
/// one that merely contains it, so "Discard" finds the Discard button rather
/// than a label above it asking whether to discard. Between equals a control
/// beats a label, since Firefox gives a field's label the field's name.
fn rank(e: &ElementInfo, wanted: Option<&str>) -> u8 {
    let exact = wanted.is_none_or(|n| e.name.to_lowercase() == n);
    u8::from(!exact) * 2 + u8::from(e.role == "label")
}

fn best_match(found: Vec<ElementInfo>, name: Option<&str>) -> Result<ElementInfo, CmdError> {
    let wanted = name.map(str::to_lowercase);
    found
        .into_iter()
        .min_by_key(|e| rank(e, wanted.as_deref()))
        .ok_or_else(|| CmdError::not_found("no matching element"))
}

pub fn find(
    window: WindowHandle,
    name: Option<&str>,
    role: Option<&str>,
) -> Result<ElementInfo, CmdError> {
    best_match(search(window, role, name, true)?, name)
}

/// Every match, in the order `find` prefers them.
pub fn find_all(
    window: WindowHandle,
    name: Option<&str>,
    role: Option<&str>,
) -> Result<Vec<ElementInfo>, CmdError> {
    let wanted = name.map(str::to_lowercase);
    let mut found = search(window, role, name, false)?;
    found.sort_by_key(|e| rank(e, wanted.as_deref()));
    Ok(found)
}

struct Element {
    id: String,
    bus: Bus,
    node: Node,
    /// What the id's indices count from: the window's top element.
    top: Node,
    window: WindowHandle,
    origin: (i32, i32),
}

fn resolve(id: &str) -> Result<Element, CmdError> {
    let mut parts = id.split('/');
    let window_part = parts.next().ok_or_else(|| CmdError::invalid_args("invalid element id"))?;
    let window = window::parse_id(window_part)?;
    let indices = parts
        .map(|part| part.parse::<i32>().map_err(|_| CmdError::invalid_args("invalid element id")))
        .collect::<Result<Vec<_>, _>>()?;

    let Root { bus, mut node, origin } = root(window)?;
    let top = node.clone();
    for index in indices {
        let not_found = || CmdError::not_found("element no longer exists; re-run element list/find");
        let (dest, path): (String, OwnedObjectPath) = bus
            .call(&node, ACCESSIBLE, "GetChildAtIndex", &(index,))
            .map_err(|_| not_found())?;
        if path.as_str() == NULL_PATH {
            return Err(not_found());
        }
        node = Node { dest, path: path.to_string() };
    }
    Ok(Element { id: id.to_string(), bus, node, top, window, origin })
}

fn click_at(bus: &Bus, node: &Node, window: WindowHandle, origin: (i32, i32)) -> Result<(), CmdError> {
    // Closed menus and scrolled-away rows still report a rectangle,
    // which would send the click to whatever is drawn there instead.
    if !bus.drawn(node) {
        return Err(CmdError::not_found(
            "element is not showing on screen, so it can't be clicked; try element invoke",
        ));
    }
    // The click goes to whatever is drawn at that spot.
    window::bring_forward(window)?;
    let (x, y, w, h) = bus.rect(node);
    super::mouse::click(MouseButton::Left, false, Some((origin.0 + x + w / 2, origin.1 + y + h / 2)))
}

impl Element {
    fn click(&self) -> Result<(), CmdError> {
        click_at(&self.bus, &self.node, self.window, self.origin)
    }

    /// Toolkits accept actions on a greyed out control and do nothing.
    fn check_enabled(&self) -> Result<(), CmdError> {
        if disabled(self.bus.state(&self.node)) {
            return Err(CmdError::os(
                "the element is greyed out and ignores input for now; `wait element --enabled` waits until it doesn't",
            ));
        }
        Ok(())
    }

    /// Gives the element keyboard focus, or says it didn't take it: keys sent
    /// anyway would land in whatever had focus before.
    fn take_focus(&self) -> Result<(), CmdError> {
        // An application can only move keyboard focus inside its own window.
        window::focus(self.window)?;
        let _ = self.bus.call::<_, bool>(&self.node, COMPONENT, "GrabFocus", &());
        if settle(|| self.bus.state(&self.node)[0] & STATE_FOCUSED != 0) {
            Ok(())
        } else {
            Err(CmdError::os("the element did not take keyboard focus, so nothing was typed"))
        }
    }
}

pub fn rect(id: &str) -> Result<(i32, i32, i32, i32), CmdError> {
    let el = resolve(id)?;
    if !el.bus.drawn(&el.node) {
        return Err(CmdError::not_found("element is not showing on screen"));
    }
    let (x, y, w, h) = el.bus.rect(&el.node);
    Ok((el.origin.0 + x, el.origin.1 + y, w, h))
}

/// The smallest element drawn at a point, in the topmost window there.
/// Wayland gives clients no way to hit-test the screen, so this goes by the
/// rectangles the window and its elements report.
pub fn at(x: i32, y: i32) -> Result<ElementInfo, CmdError> {
    let inside = |rx: i32, ry: i32, rw: i32, rh: i32| x >= rx && y >= ry && x < rx + rw && y < ry + rh;
    let top = window::list(None)?
        .into_iter()
        .find(|w| w.state != "minimized" && inside(w.x, w.y, w.w, w.h))
        .ok_or_else(|| CmdError::not_found(format!("no window at {x},{y}")))?;
    search(window::parse_id(&top.id)?, None, None, false)?
        .into_iter()
        .rev()
        .filter(|e| e.showing && inside(e.x, e.y, e.w, e.h))
        .min_by_key(|e| e.w as i64 * e.h as i64)
        .ok_or_else(|| CmdError::not_found(format!("no element of the window at {x},{y} is there")))
}

pub fn click(id: &str) -> Result<(), CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    el.click()
}

/// Runs the element's own activating AT-SPI action, which presses the
/// control without moving the mouse. Ok(false) when it has none.
pub(super) fn activate(bus: &Bus, node: &Node) -> Result<bool, CmdError> {
    // In order of preference. Qt also lists SetFocus and the like, which
    // wouldn't activate anything. Firefox names the action after what it does
    // right now, and "clickancestor" on what sits inside a clickable element.
    const ACTIVATING: &[&str] = &[
        "press", "click", "activate", "invoke", "toggle", "showmenu", "jump", "open", "check",
        "uncheck", "select", "switch", "expand", "collapse", "clickancestor",
    ];
    run_action(bus, node, ACTIVATING)
}

/// Runs the first of the `wanted` actions the element has. Ok(false) when it
/// has none of them.
fn run_action(bus: &Bus, node: &Node, wanted: &[&str]) -> Result<bool, CmdError> {
    // One at a time: Firefox answers GetActions with ";;" for every name.
    let count: i32 = bus.property(node, ACTION, "NActions").unwrap_or(0);
    // Qt calls one "Show Menu".
    let names: Vec<String> = (0..count)
        .map(|index| bus.call::<_, String>(node, ACTION, "GetName", &(index,)).unwrap_or_default())
        .map(|name| name.to_lowercase().replace(' ', ""))
        .collect();
    match wanted.iter().find_map(|wanted| names.iter().position(|n| n == wanted)) {
        Some(index) => {
            bus.call::<_, bool>(node, ACTION, "DoAction", &(index as i32,)).map_err(dbus::err)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Prefers one of the element's own AT-SPI actions; falls back to a
/// synthesized click at its center for controls that have none.
pub fn invoke(id: &str) -> Result<(), CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    if activate(&el.bus, &el.node)? {
        return Ok(());
    }
    el.click()
}

/// Apps may apply a change a moment after accepting it, so polls for up to
/// half a second before believing it didn't happen.
fn settle(done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    done()
}

pub fn check(id: &str, on: bool) -> Result<(), CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    if !checkable(el.bus.state(&el.node), &el.bus.role(&el.node)) {
        return Err(CmdError::unsupported("this element doesn't report a checked state"));
    }
    let is_on = || el.bus.state(&el.node)[0] & STATE_CHECKED != 0;
    if is_on() == on {
        return Ok(());
    }
    if !activate(&el.bus, &el.node)? {
        el.click()?;
    }
    if settle(|| is_on() == on) {
        Ok(())
    } else {
        Err(CmdError::os(
            "the control did not switch; a radio button can't be turned off directly, so turn on another one instead",
        ))
    }
}

/// GTK leaves list rows unnamed and puts their text in a label inside.
fn item_name(bus: &Bus, node: &Node) -> String {
    fn label_in(bus: &Bus, node: &Node, depth: usize) -> Option<String> {
        if depth == 0 {
            return None;
        }
        bus.children(node).iter().find_map(|child| {
            let name = bus.name(child);
            if bus.role(child) == "label" && !name.is_empty() {
                return Some(name);
            }
            label_in(bus, child, depth - 1)
        })
    }
    let name = bus.name(node);
    if name.is_empty() {
        return label_in(bus, node, 4).unwrap_or_default();
    }
    // Firefox counts the bullet of a list item as part of its name.
    name.trim_start_matches(['•', '◦', '▪', ' ']).to_string()
}

/// The item under `node` with that name, its parent and its place in the
/// parent. An exact name wins over one that merely contains the text.
fn find_item(bus: &Bus, node: &Node, wanted: &str, depth: usize) -> Option<(bool, Node, usize, Node)> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut partial = None;
    for (index, child) in bus.children(node).into_iter().enumerate() {
        if ITEM_ROLES.contains(&bus.role(&child).as_str()) {
            let name = item_name(bus, &child).to_lowercase();
            if name == wanted {
                return Some((true, node.clone(), index, child));
            }
            if partial.is_none() && name.contains(wanted) {
                partial = Some((false, node.clone(), index, child.clone()));
            }
        }
        match find_item(bus, &child, wanted, depth + 1) {
            Some(found) if found.0 => return Some(found),
            Some(found) if partial.is_none() => partial = Some(found),
            _ => {}
        }
    }
    partial
}

/// How many presses of the down key lead from the first item under `node` to
/// `target`, across any groups of items; the keyboard skips greyed out ones.
fn keyboard_steps(bus: &Bus, node: &Node, target: &Node) -> Option<u32> {
    fn walk(bus: &Bus, node: &Node, target: &Node, steps: &mut u32) -> bool {
        for child in bus.children(node) {
            if child.dest == target.dest && child.path == target.path {
                return true;
            }
            if ITEM_ROLES.contains(&bus.role(&child).as_str()) {
                *steps += u32::from(!disabled(bus.state(&child)));
            } else if walk(bus, &child, target, steps) {
                return true;
            }
        }
        false
    }
    let mut steps = 0;
    walk(bus, node, target, &mut steps).then_some(steps)
}

/// Through the parent's Selection interface, or else the item's own action.
/// In Firefox the action comes first: Selection changes a page's list without
/// telling the page.
/// Qt's Selection only highlights a row without making it the current one,
/// which is what its apps act on, so a row showing there gets clicked instead,
/// which also replaces any other selection.
fn select_child(el: &Element, parent: &Node, index: usize, item: &Node) -> Result<(), CmdError> {
    let bus = &el.bus;
    if disabled(bus.state(item)) {
        return Err(CmdError::os("this item is greyed out"));
    }
    if bus.toolkit(item) == "Qt" && bus.showing(item) {
        return click_at(bus, item, el.window, el.origin);
    }
    if bus.gecko(item) {
        if !activate(bus, item)? {
            return Err(CmdError::unsupported("this element can't be selected"));
        }
        return if settle(|| bus.state(item)[0] & STATE_SELECTED != 0) {
            Ok(())
        } else {
            Err(CmdError::os("the item was pressed but did not become selected"))
        };
    }
    if bus.implements(parent, SELECTION)
        && bus.call::<_, bool>(parent, SELECTION, "SelectChild", &(index as i32,)).unwrap_or(false)
        && settle(|| bus.state(item)[0] & STATE_SELECTED != 0)
    {
        return Ok(());
    }
    if activate(bus, item)? {
        return Ok(());
    }
    Err(CmdError::unsupported("this element can't be selected"))
}

/// Whether this is Firefox's own `<select>` dropdown rather than a web page's
/// imitation of one, which is a text field.
fn native_select(bus: &Bus, node: &Node) -> bool {
    bus.role(node) == "combo box" && bus.gecko(node) && !bus.implements(node, TEXT)
}

/// Whether a dropdown's list is showing. Its button, in GTK 4, always is.
fn dropdown_is_open(bus: &Bus, node: &Node) -> bool {
    bus.children(node)
        .iter()
        .any(|child| ["list", "list box", "menu"].contains(&bus.role(child).as_str()) && bus.showing(child))
}

/// Presses a dropdown, or the button inside it in GTK 4, where it has no
/// action of its own.
fn press_dropdown(bus: &Bus, node: &Node) -> Result<bool, CmdError> {
    fn button_in(bus: &Bus, node: &Node, depth: usize) -> Option<Node> {
        if depth == 0 {
            return None;
        }
        bus.children(node).into_iter().find_map(|child| {
            if bus.role(&child) == "toggle button" {
                return Some(child);
            }
            button_in(bus, &child, depth - 1)
        })
    }
    if activate(bus, node)? {
        return Ok(true);
    }
    match button_in(bus, node, 3) {
        Some(button) => activate(bus, &button),
        None => Ok(false),
    }
}

/// The list of a Firefox `<select>` that is open in this window. The `<select>`
/// itself shows no sign of it: Firefox makes a second dropdown for the list,
/// directly under the window, and removes it on closing, which moves the ids of
/// the elements after it by one.
fn open_select_list(el: &Element) -> Option<Node> {
    let bus = &el.bus;
    bus.children(&el.top).into_iter().find(|child| bus.role(child) == "combo box" && dropdown_is_open(bus, child))
}

/// Whether this element is itself the list of an open `<select>`.
fn is_open_select_list(el: &Element) -> bool {
    open_select_list(el).is_some_and(|list| list.dest == el.node.dest && list.path == el.node.path)
}

/// Opens or closes a Firefox `<select>`. Only one list can be open at a time,
/// and it may be another `<select>`'s, so opening starts by closing it.
fn set_select_open(el: &Element, open: bool) -> Result<bool, CmdError> {
    let is_open = || open_select_list(el).is_some();
    if if open { is_open_select_list(el) } else { !is_open() } {
        return Ok(true);
    }
    window::bring_forward(el.window)?;
    if is_open() {
        super::keyboard::key("escape", 1)?;
        if !settle(|| !is_open()) {
            return Ok(false);
        }
    }
    Ok(!open || (press_dropdown(&el.bus, &el.node)? && settle(is_open)))
}

/// Opens or closes a dropdown. Its list takes keyboard focus away from
/// whichever window has it, and only opens over the active window.
fn set_dropdown_open(el: &Element, open: bool) -> Result<bool, CmdError> {
    if native_select(&el.bus, &el.node) {
        return set_select_open(el, open);
    }
    let is_open = || dropdown_is_open(&el.bus, &el.node);
    if is_open() == open {
        return Ok(true);
    }
    window::bring_forward(el.window)?;
    if !press_dropdown(&el.bus, &el.node)? {
        return Ok(false);
    }
    if settle(|| is_open() == open) {
        return Ok(true);
    }
    if !open {
        return Ok(false);
    }
    // Qt's list won't open in a window that hasn't had a key press yet, which
    // an accessibility action doesn't count as, and shift does nothing else.
    super::keyboard::key("shift", 1)?;
    Ok(press_dropdown(&el.bus, &el.node)? && settle(is_open))
}

/// Walks an open list to an item and takes it. Firefox only tells a page once,
/// on taking it, as long as its `<select>` really is open: on a closed one each
/// key is a change of its own.
fn take_with_keys(bus: &Bus, list: &Node, item: &Node) -> Result<(), CmdError> {
    let steps = keyboard_steps(bus, list, item).unwrap_or(0);
    super::keyboard::key("home", 1)?;
    if steps > 0 {
        super::keyboard::key("down", steps)?;
    }
    super::keyboard::key("return", 1)
}

/// Selecting through a dropdown's list reports success in Qt and GTK 3 and
/// changes nothing, so what counts here is the dropdown's own text afterwards.
/// GTK 3's items can be pressed while it's closed; Qt's and GTK 4's can only be
/// reached with the keyboard once it's open.
fn select_in_dropdown(el: &Element, wanted: &str) -> Result<(), CmdError> {
    let bus = &el.bus;
    let lower = wanted.to_lowercase();
    // An item from the list of an open <select>, which goes when it closes, so
    // that is all there is to check afterwards.
    if is_open_select_list(el) {
        let (_, _, _, item) = find_item(bus, &el.node, &lower, 0)
            .ok_or_else(|| CmdError::not_found(format!("no item \"{wanted}\" here")))?;
        if disabled(bus.state(&item)) {
            return Err(CmdError::os("this item is greyed out"));
        }
        window::bring_forward(el.window)?;
        take_with_keys(bus, &el.node, &item)?;
        return if settle(|| open_select_list(el).is_none()) {
            Ok(())
        } else {
            Err(CmdError::os(format!("the dropdown did not change to \"{wanted}\"")))
        };
    }
    let current = || bus.value(&el.node).unwrap_or_default().to_lowercase();
    if current() == lower {
        return Ok(());
    }
    if bus.gecko(&el.node) && !native_select(bus, &el.node) {
        return select_in_page_combobox(el, wanted);
    }
    if let Some((_, _, _, item)) = find_item(bus, &el.node, &lower, 0).filter(|_| !native_select(bus, &el.node)) {
        let name = item_name(bus, &item).to_lowercase();
        if run_action(bus, &item, &["click", "press", "activate"])? && settle(|| current() == name) {
            return Ok(());
        }
    }
    if !set_dropdown_open(el, true)? {
        return Err(CmdError::unsupported("this dropdown can't be opened"));
    }
    let start = Instant::now();
    let mut found = find_item(bus, &el.node, &lower, 0);
    while found.is_none() && start.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(100));
        found = find_item(bus, &el.node, &lower, 0);
    }
    let Some((_, _, _, item)) = found else {
        super::keyboard::key("escape", 1)?;
        return Err(CmdError::not_found(format!("no item \"{wanted}\" here")));
    };
    // The keyboard skips it, so the steps below would land on another item.
    if disabled(bus.state(&item)) {
        super::keyboard::key("escape", 1)?;
        return Err(CmdError::os("this item is greyed out"));
    }
    let name = item_name(bus, &item).to_lowercase();
    take_with_keys(bus, &el.node, &item)?;
    if settle(|| current() == name) {
        return Ok(());
    }
    if dropdown_is_open(bus, &el.node) && !native_select(bus, &el.node) {
        super::keyboard::key("escape", 1)?;
    }
    Err(CmdError::os(format!("the dropdown did not change to \"{wanted}\"")))
}

/// The lists under `node`, in the order the page has them, looking at no more
/// than `budget` elements: a whole page can take seconds to walk.
fn lists_under(bus: &Bus, node: &Node, depth: usize, budget: &mut usize, out: &mut Vec<Node>) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in bus.children(node) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        if ["list box", "list", "menu"].contains(&bus.role(&child).as_str()) {
            out.push(child);
        } else {
            lists_under(bus, &child, depth + 1, budget, out);
        }
    }
}

/// The web page an element is in.
fn page_of(el: &Element) -> Option<Node> {
    let mut id = el.id.as_str();
    while let Some((above, _)) = id.rsplit_once('/') {
        let up = resolve(above).ok()?;
        if up.bus.role(&up.node) == "document web" {
            return Some(up.node);
        }
        id = above;
    }
    None
}

/// A web page's combobox: a text field whose list is usually elsewhere in the
/// page, tied to it by aria-controls, and often made only when it opens. One
/// without that tie gets the list that came with opening it, since the page
/// may have other lists with the same items. Many show their list, or the item
/// wanted, only once something is typed.
fn select_in_page_combobox(el: &Element, wanted: &str) -> Result<(), CmdError> {
    let bus = &el.bus;
    let lower = wanted.to_lowercase();
    let lists = |page: &Node| {
        let mut out = Vec::new();
        lists_under(bus, page, 0, &mut { MAX_VISITED }, &mut out);
        out
    };
    let page = if bus.controlled(&el.node).is_empty() { page_of(el) } else { None };
    let before: Vec<Node> = match &page {
        Some(page) if bus.state(&el.node)[0] & STATE_EXPANDED == 0 => {
            lists(page).into_iter().filter(|list| laid_out(bus.rect(list))).collect()
        }
        _ => Vec::new(),
    };
    let in_list = || {
        std::iter::once(el.node.clone())
            .chain(bus.controlled(&el.node))
            .find_map(|list| find_item(bus, &list, &lower, 0))
            .or_else(|| {
                let page = page.as_ref()?;
                lists(page)
                    .iter()
                    .filter(|list| !before.iter().any(|old| old.dest == list.dest && old.path == list.path))
                    .find_map(|list| find_item(bus, list, &lower, 0))
            })
    };
    let wait_for_item = || {
        let start = Instant::now();
        let mut found = in_list();
        while found.is_none() && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(100));
            found = in_list();
        }
        found
    };
    let opened = set_expanded(el, true);
    let mut found = if opened.is_ok() { wait_for_item() } else { None };
    let mut typed_over = None;
    if found.is_none() && bus.state(&el.node)[0] & STATE_EDITABLE != 0 {
        typed_over = bus.value(&el.node);
        set_text(el, wanted)?;
        found = wait_for_item();
    }
    let Some((_, _, _, item)) = found else {
        match typed_over {
            Some(text) => {
                let _ = set_text(el, &text);
            }
            None => opened?,
        }
        let _ = set_expanded(el, false);
        return Err(CmdError::not_found(format!("no item \"{wanted}\" here")));
    };
    let name = item_name(bus, &item).to_lowercase();
    let title = || root(el.window).map(|frame| frame.bus.name(&frame.node)).unwrap_or_default();
    let title_before = title();
    // An item wrapping a link takes its own action and does nothing with it.
    let link = bus.children(&item).into_iter().find(|child| bus.role(child) == "link");
    if !activate(bus, link.as_ref().unwrap_or(&item))? {
        return Err(CmdError::unsupported("this item has no action to choose it"));
    }
    // A search box may go to another page instead of keeping the item's text,
    // which can take a while, and the list may stay behind in the old one.
    let start = Instant::now();
    let chosen = || {
        let now = title();
        bus.value(&el.node).unwrap_or_default().to_lowercase() == name
            || !bus.showing(&item)
            || (!now.is_empty() && now != title_before)
    };
    let mut done = chosen();
    while !done && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
        done = chosen();
    }
    if done {
        Ok(())
    } else {
        Err(CmdError::os(format!("the field did not change to \"{wanted}\"")))
    }
}

pub fn select(id: &str, item: Option<&str>) -> Result<(), CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    let Some(wanted) = item else {
        let (parent_id, index) =
            id.rsplit_once('/').ok_or_else(|| CmdError::invalid_args("this element id has no parent to select in"))?;
        let index = index.parse().map_err(|_| CmdError::invalid_args("invalid element id"))?;
        // An item of a dropdown can only be chosen through the dropdown.
        let mut ancestor = parent_id;
        for _ in 0..3 {
            let above = resolve(ancestor)?;
            if above.bus.role(&above.node) == "combo box" {
                above.check_enabled()?;
                return select_in_dropdown(&above, &item_name(&el.bus, &el.node));
            }
            match ancestor.rsplit_once('/') {
                Some((next, _)) => ancestor = next,
                None => break,
            }
        }
        let parent = resolve(parent_id)?;
        return select_child(&parent, &parent.node, index, &el.node);
    };
    if el.bus.role(&el.node) == "combo box" {
        return select_in_dropdown(&el, wanted);
    }
    let (_, parent, index, item) = find_item(&el.bus, &el.node, &wanted.to_lowercase(), 0)
        .ok_or_else(|| CmdError::not_found(format!("no item \"{wanted}\" here")))?;
    select_child(&el, &parent, index, &item)
}

pub fn expand(id: &str, open: bool) -> Result<(), CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    let is_page_combobox = el.bus.gecko(&el.node) && !native_select(&el.bus, &el.node);
    if el.bus.role(&el.node) == "combo box" && !is_page_combobox {
        return if set_dropdown_open(&el, open)? {
            Ok(())
        } else {
            Err(CmdError::os(format!("the dropdown did not {}", if open { "expand" } else { "collapse" })))
        };
    }
    set_expanded(&el, open)
}

fn set_expanded(el: &Element, open: bool) -> Result<(), CmdError> {
    let what = if open { "expand" } else { "collapse" };
    let role = el.bus.role(&el.node);
    let gecko = el.bus.gecko(&el.node);
    let state = || el.bus.state(&el.node)[0];
    if state() & STATE_EXPANDABLE == 0 {
        return Err(CmdError::unsupported(format!("this element can't {what}")));
    }
    let is_open = || state() & STATE_EXPANDED != 0;
    if is_open() == open {
        return Ok(());
    }
    // A row's "Toggle" in Qt is about whether it's selected, not expanded.
    let row = ["tree item", "table cell"].contains(&role.as_str());
    let actions: &[&str] = if row && !gecko {
        &["expandorcontract"]
    } else if row {
        &[what, "click"]
    } else {
        &["expandorcontract", what, "toggle", "showmenu", "press", "click", "activate", "open", "close"]
    };
    let ran = run_action(&el.bus, &el.node, actions)?;
    if ran && settle(|| is_open() == open) {
        return Ok(());
    }
    if row {
        // The keyboard needs the row to be the current one; a click makes it
        // so. Not in Firefox, where a row's middle may be a child row by now.
        if gecko {
            el.take_focus()?;
        } else {
            el.click()?;
        }
        super::keyboard::key(if open { "right" } else { "left" }, 1)?;
        // Qt replaces a row's object when the tree changes shape, so this one
        // goes on reporting the old state.
        let id = el.id.as_str();
        let now_open = || resolve(id).is_ok_and(|e| e.bus.state(&e.node)[0] & STATE_EXPANDED != 0);
        if settle(|| now_open() == open) {
            return Ok(());
        }
    } else if gecko {
        // A web page's control may only listen for the mouse.
        el.click()?;
        if settle(|| is_open() == open) {
            return Ok(());
        }
    } else if !ran {
        return Err(CmdError::unsupported(format!("this element has no action to {what} it")));
    }
    Err(CmdError::os(format!("the element accepted the {what} but its state did not change")))
}

/// Returns what the control ended up with: a slider may snap to its steps.
pub fn set_value(id: &str, value: f64) -> Result<f64, CmdError> {
    let el = resolve(id)?;
    el.check_enabled()?;
    if !el.bus.implements(&el.node, VALUE) {
        return Err(CmdError::unsupported("this element has no value to set"));
    }
    let bound = |name| el.bus.property::<f64>(&el.node, VALUE, name).map_err(dbus::err);
    let (min, max) = (bound("MinimumValue")?, bound("MaximumValue")?);
    if value < min || value > max {
        return Err(CmdError::invalid_args(format!("{value} is outside this control's range, {min} to {max}")));
    }
    let before = bound("CurrentValue")?;
    // Firefox changes a number field this way without telling the page, so
    // anything on the page that depends on it goes stale. Typing tells it.
    if el.bus.gecko(&el.node) && el.bus.state(&el.node)[0] & STATE_EDITABLE != 0 {
        text_set(id, &value.to_string())?;
        settle(|| bound("CurrentValue").is_ok_and(|now| now == value));
        return bound("CurrentValue");
    }
    el.bus
        .0
        .call_method(
            Some(el.node.dest.as_str()),
            el.node.path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &(VALUE, "CurrentValue", zbus::zvariant::Value::from(value)),
        )
        .map_err(dbus::err)?;
    settle(|| bound("CurrentValue").is_ok_and(|now| now == value));
    let now = bound("CurrentValue")?;
    // Staying put is fine if a step of the control is too big to get closer.
    let step = bound("MinimumIncrement").unwrap_or(0.0);
    if now == before && (value - before).abs() >= step.max(f64::MIN_POSITIVE) {
        return Err(CmdError::os(
            "the control ignored the new value; use the keyboard: click it, then press the arrow keys with `keyboard key`",
        ));
    }
    Ok(now)
}

pub fn text_get(id: &str) -> Result<String, CmdError> {
    let el = resolve(id)?;
    if el.bus.implements(&el.node, TEXT) {
        return el
            .bus
            .full_text(&el.node, &mut { MAX_VISITED })
            .ok_or_else(|| CmdError::os("the element did not give its text"));
    }
    if el.bus.implements(&el.node, VALUE) {
        return el
            .bus
            .property::<f64>(&el.node, VALUE, "CurrentValue")
            .map(|v| v.to_string())
            .map_err(dbus::err);
    }
    Err(CmdError::unsupported(
        "element does not support reading text (no Text or Value interface)",
    ))
}

/// Prefers the EditableText interface; falls back to focusing the element
/// and synthesizing select-all + type, for editors that don't expose it.
pub fn text_set(id: &str, text: &str) -> Result<(), CmdError> {
    set_text(&resolve(id)?, text)
}

fn set_text(el: &Element, text: &str) -> Result<(), CmdError> {
    el.check_enabled()?;
    let state = el.bus.state(&el.node);
    // Firefox lets everything on a page claim to be editable text.
    if state[1] & STATE_READ_ONLY != 0 || (el.bus.gecko(&el.node) && state[0] & STATE_EDITABLE == 0) {
        return Err(CmdError::unsupported("this element's text can't be edited"));
    }
    let read = || el.bus.call::<_, String>(&el.node, TEXT, "GetText", &(0i32, -1i32));
    let before = read();
    if el.bus.implements(&el.node, EDITABLE_TEXT) {
        let _: zbus::Result<bool> =
            el.bus.call(&el.node, EDITABLE_TEXT, "SetTextContents", &(text,));
        // Read it back rather than trusting the reply: Firefox's address
        // bar reports success and changes nothing.
        if read().is_ok_and(|now| now == text) {
            return Ok(());
        }
    }
    el.take_focus()?;
    super::keyboard::hotkey("ctrl+a")?;
    if text.is_empty() {
        super::keyboard::key("delete", 1)?;
    } else {
        super::keyboard::type_text(text)?;
    }
    // A field may reformat what it's given, and a password field only shows
    // dots, but one still holding the old text didn't take it.
    let hidden = el.bus.role(&el.node) == "password text";
    if let Ok(before) = before {
        if !hidden && before != text && !settle(|| read().is_ok_and(|now| now != before)) {
            return Err(CmdError::os("the field did not change; it may not accept typing"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str, role: &str) -> ElementInfo {
        ElementInfo {
            id: String::new(),
            name: name.into(),
            role: role.into(),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            showing: true,
            value: None,
            checked: None,
            disabled: false,
        }
    }

    #[test]
    fn a_control_comes_before_its_label() {
        let found = vec![element("Name", "label"), element("Name", "entry")];
        assert_eq!(best_match(found, Some("name")).unwrap().role, "entry");
    }

    #[test]
    fn a_whole_name_comes_before_a_control_that_contains_it() {
        let found = vec![element("Discard all", "button"), element("Discard", "label")];
        assert_eq!(best_match(found, Some("discard")).unwrap().role, "label");
    }

    #[test]
    fn bogus_rectangles_are_not_places() {
        assert!(laid_out((10, -20, 5, 5)));
        assert!(!laid_out((-1, -1, -1, -1)));
        assert!(!laid_out((-2147481712, 0, 20, 20)));
        assert!(!laid_out((i32::MIN, 0, 20, 20)));
        assert!(!laid_out((0, 0, 0, 20)));
    }
}
