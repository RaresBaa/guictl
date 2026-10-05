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
/// costs a few D-Bus round trips.
const MAX_DEPTH: usize = 20;
const MAX_RESULTS: usize = 2000;
const MAX_VISITED: usize = 5000;

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const ACTION: &str = "org.a11y.atspi.Action";
const TEXT: &str = "org.a11y.atspi.Text";
const EDITABLE_TEXT: &str = "org.a11y.atspi.EditableText";
const VALUE: &str = "org.a11y.atspi.Value";

/// What AT-SPI hands back in place of "no such object".
const NULL_PATH: &str = "/org/a11y/atspi/null";
/// Extents relative to the element's top-level window. Screen coordinates
/// (type 0) aren't available to clients on Wayland.
const COORD_WINDOW: u32 = 1;
/// Bit of `ATSPI_STATE_SHOWING` in the first word of the state set.
const STATE_SHOWING: u32 = 1 << 25;

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
}

/// One object in an application's accessibility tree: the application's name
/// on the accessibility bus plus an object path.
#[derive(Clone)]
struct Node {
    dest: String,
    path: String,
}

struct Bus(Connection);

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

    fn children(&self, node: &Node) -> Vec<Node> {
        self.call::<_, Vec<(String, OwnedObjectPath)>>(node, ACCESSIBLE, "GetChildren", &())
            .unwrap_or_default()
            .into_iter()
            .map(|(dest, path)| Node { dest, path: path.to_string() })
            .collect()
    }

    fn name(&self, node: &Node) -> String {
        self.property(node, ACCESSIBLE, "Name").unwrap_or_default()
    }

    fn role(&self, node: &Node) -> String {
        self.call(node, ACCESSIBLE, "GetRoleName", &()).unwrap_or_default()
    }

    fn rect(&self, node: &Node) -> (i32, i32, i32, i32) {
        self.call(node, COMPONENT, "GetExtents", &(COORD_WINDOW,)).unwrap_or_default()
    }

    fn showing(&self, node: &Node) -> bool {
        self.call::<_, Vec<u32>>(node, ACCESSIBLE, "GetState", &())
            .ok()
            .and_then(|state| state.first().copied())
            .is_some_and(|word| word & STATE_SHOWING != 0)
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
        "could not reach the accessibility bus (is at-spi2-core installed?): {e}"
    ))
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
struct Root {
    bus: Bus,
    node: Node,
    origin: (i32, i32),
}

/// Finds the accessible that stands for a KWin window. The two sides share
/// no id, so the application is matched by pid and its top-level by title,
/// falling back to size and then to whichever one is showing.
fn root(handle: WindowHandle) -> Result<Root, CmdError> {
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

    let size = (target.w, target.h);
    let node = frames
        .iter()
        .find(|f| bus.name(f) == target.title)
        .or_else(|| {
            frames.iter().find(|f| {
                let (_, _, w, h) = bus.rect(f);
                (w, h) == size
            })
        })
        .or_else(|| frames.iter().find(|f| bus.showing(f)))
        .or(frames.first())
        .cloned()
        .ok_or_else(|| {
            CmdError::unsupported(
                "the window's application exposes no accessibility tree (Chromium and Electron \
                 apps need --force-renderer-accessibility; some apps only pick up accessibility \
                 when started after it was turned on)",
            )
        })?;
    Ok(Root { bus, node, origin: (target.x, target.y) })
}

struct Walk<'a> {
    bus: &'a Bus,
    window_id: &'a str,
    role_filter: Option<&'a str>,
    name_filter: Option<String>,
    stop_at_first: bool,
    visited: usize,
    out: Vec<ElementInfo>,
}

impl Walk<'_> {
    fn done(&self) -> bool {
        self.out.len() >= MAX_RESULTS
            || self.visited >= MAX_VISITED
            || (self.stop_at_first && !self.out.is_empty())
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
            let role_ok = self.role_filter.is_none_or(|r| role.eq_ignore_ascii_case(r));
            let name_ok =
                self.name_filter.as_ref().is_none_or(|n| name.to_lowercase().contains(n));
            if role_ok && name_ok {
                let (x, y, w, h) = self.bus.rect(child);
                let id = format!(
                    "{}/{}",
                    self.window_id,
                    path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
                );
                let showing = self.bus.showing(child);
                self.out.push(ElementInfo {
                    id,
                    name,
                    role,
                    x: origin.0 + x,
                    y: origin.1 + y,
                    w,
                    h,
                    showing,
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
    stop_at_first: bool,
) -> Result<Vec<ElementInfo>, CmdError> {
    let root = root(handle)?;
    let window_id = window::format_id(handle);
    let mut walk = Walk {
        bus: &root.bus,
        window_id: &window_id,
        role_filter: role,
        name_filter: name.map(str::to_lowercase),
        stop_at_first,
        visited: 0,
        out: Vec::new(),
    };
    walk.collect(&root.node, &mut Vec::new(), root.origin);
    Ok(walk.out)
}

pub fn list(window: WindowHandle, role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
    search(window, role, None, false)
}

pub fn find(
    window: WindowHandle,
    name: Option<&str>,
    role: Option<&str>,
) -> Result<ElementInfo, CmdError> {
    search(window, role, name, true)?
        .into_iter()
        .next()
        .ok_or_else(|| CmdError::not_found("no matching element"))
}

struct Element {
    bus: Bus,
    node: Node,
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
    Ok(Element { bus, node, window, origin })
}

impl Element {
    fn click(&self) -> Result<(), CmdError> {
        // Closed menus and scrolled-away rows still report a rectangle,
        // which would send the click to whatever is drawn there instead.
        if !self.bus.showing(&self.node) {
            return Err(CmdError::not_found(
                "element is not showing on screen, so it can't be clicked; try element invoke",
            ));
        }
        let (x, y, w, h) = self.bus.rect(&self.node);
        super::mouse::click(
            MouseButton::Left,
            false,
            Some((self.origin.0 + x + w / 2, self.origin.1 + y + h / 2)),
        )
    }
}

pub fn click(id: &str) -> Result<(), CmdError> {
    resolve(id)?.click()
}

/// Prefers one of the element's own AT-SPI actions (press the control
/// without moving the mouse); falls back to a synthesized click at its
/// center for controls that have none.
pub fn invoke(id: &str) -> Result<(), CmdError> {
    // In order of preference. Qt also lists SetFocus and the like, which
    // wouldn't activate anything.
    const ACTIVATING: &[&str] =
        &["press", "click", "activate", "invoke", "toggle", "showmenu", "jump", "open"];

    let el = resolve(id)?;
    let actions: Vec<(String, String, String)> =
        el.bus.call(&el.node, ACTION, "GetActions", &()).unwrap_or_default();
    let names: Vec<String> = actions.into_iter().map(|(name, _, _)| name.to_lowercase()).collect();
    let chosen = ACTIVATING.iter().find_map(|wanted| names.iter().position(|n| n == wanted));
    match chosen {
        Some(index) => el
            .bus
            .call::<_, bool>(&el.node, ACTION, "DoAction", &(index as i32,))
            .map(|_| ())
            .map_err(dbus::err),
        None => el.click(),
    }
}

pub fn text_get(id: &str) -> Result<String, CmdError> {
    let el = resolve(id)?;
    if el.bus.implements(&el.node, TEXT) {
        return el.bus.call(&el.node, TEXT, "GetText", &(0i32, -1i32)).map_err(dbus::err);
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
    let el = resolve(id)?;
    if el.bus.implements(&el.node, EDITABLE_TEXT) {
        let _: zbus::Result<bool> =
            el.bus.call(&el.node, EDITABLE_TEXT, "SetTextContents", &(text,));
        // Read it back rather than trusting the reply: Firefox's address
        // bar reports success and changes nothing.
        let now: zbus::Result<String> = el.bus.call(&el.node, TEXT, "GetText", &(0i32, -1i32));
        if now.is_ok_and(|now| now == text) {
            return Ok(());
        }
    }
    // An application can only move keyboard focus inside its own window.
    window::focus(el.window)?;
    let _ = el.bus.call::<_, bool>(&el.node, COMPONENT, "GrabFocus", &());
    super::keyboard::hotkey("ctrl+a")?;
    super::keyboard::type_text(text)
}
