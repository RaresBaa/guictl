//! `menu`: walks a window's menu bar down to an item through AT-SPI and
//! activates it. Qt keeps the items of closed menus in the tree, so most
//! items are reached without opening anything; a menu that only fills its
//! items when it opens is opened on the way.

use super::element::{self, Bus, Node};
use super::window::WindowHandle;
use crate::output::CmdError;
use std::time::{Duration, Instant};

/// How long to wait for an opened menu's items to appear.
const LEVEL_TIMEOUT: Duration = Duration::from_millis(2000);
/// Bounds the search for the menu bar in a large tree.
const MAX_SEARCHED: usize = 3000;

const ITEM_ROLES: &[&str] = &["menu item", "menu", "check menu item", "radio menu item"];

/// Lowercase, without the & that marks an access key, a trailing "..." or
/// the shortcut some apps append after a tab.
fn normalize(name: &str) -> String {
    let name = name.split('\t').next().unwrap_or_default().replace('&', "");
    name.trim().trim_end_matches("...").trim_end_matches('…').trim().to_lowercase()
}

/// The menu bars under `root`, breadth first so the window's own comes
/// before any embedded one.
fn menu_bars(bus: &Bus, root: &Node) -> Vec<Node> {
    let mut queue = std::collections::VecDeque::from([root.clone()]);
    let mut bars = Vec::new();
    let mut searched = 0;
    while let Some(node) = queue.pop_front() {
        searched += 1;
        if searched > MAX_SEARCHED {
            break;
        }
        for child in bus.children(&node) {
            if bus.role(&child) == "menu bar" {
                bars.push(child);
            } else {
                queue.push_back(child);
            }
        }
    }
    bars
}

/// The items directly in the menu under `parent`, looking through the popup
/// container between an item and its submenu but not into further submenus.
fn items(bus: &Bus, parent: &Node) -> Vec<(Node, String)> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from(bus.children(parent));
    while let Some(node) = queue.pop_front() {
        let role = bus.role(&node);
        if ITEM_ROLES.contains(&role.as_str()) {
            let name = normalize(&bus.name(&node));
            out.push((node, name));
        } else if role != "separator" {
            queue.extend(bus.children(&node));
        }
    }
    out
}

/// An exact name before one that only starts with the wanted text.
fn pick(items: Vec<(Node, String)>, wanted: &str) -> Option<Node> {
    let exact = items.iter().position(|(_, name)| name == wanted);
    let prefix = || items.iter().position(|(_, name)| !wanted.is_empty() && name.starts_with(wanted));
    let index = exact.or_else(prefix)?;
    items.into_iter().nth(index).map(|(node, _)| node)
}

/// Opens the menus along `chain`, outermost first: a submenu only pops up
/// from an open menu.
fn open(bus: &Bus, chain: &[Node]) -> Result<(), CmdError> {
    for item in chain {
        if !element::activate(bus, item)? {
            return Err(CmdError::unsupported("a menu on the way has no action to open it"));
        }
        // Wait for the popup before opening the next level from it.
        let start = Instant::now();
        while !bus.children(item).iter().any(|c| bus.showing(c)) && start.elapsed() < LEVEL_TIMEOUT / 4 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

fn any_menu_open(bus: &Bus, bars: &[Node]) -> bool {
    bars.iter().flat_map(|bar| bus.children(bar)).any(|item| bus.children(&item).iter().any(|c| bus.showing(c)))
}

pub fn select(window: WindowHandle, path: &[&str]) -> Result<(), CmdError> {
    let root = element::root(window)?;
    let bus = &root.bus;
    let bars = menu_bars(bus, &root.node);
    if bars.is_empty() {
        return Err(CmdError::not_found(
            "the window has no menu bar; find the item with `element find` and `element invoke` it",
        ));
    }
    // Some apps (KWrite among them) run an item's action on whichever of
    // their windows is active, and a popup only shows for the active window.
    super::window::bring_forward(window)?;
    let mut opened = 0;
    let result = walk(bus, &bars, path, &mut opened);
    // Running an item through AT-SPI leaves the menus opened on the way
    // open, and closing them leaves Qt's menu bar in keyboard mode, where
    // letters typed next would pick menus. Escape closes one menu at a time,
    // and one more ends keyboard mode. It goes to whatever has focus, so
    // only while that's this window, and only while a menu is open: an
    // item that opened a dialog shouldn't have it closed.
    let ours = || super::window::active().is_ok_and(|w| w.id == super::window::format_id(window));
    if opened > 0 && ours() {
        let mut closed = false;
        for _ in 0..=opened {
            if !any_menu_open(bus, &bars) {
                break;
            }
            let _ = super::keyboard::key("escape", 1);
            closed = true;
            std::thread::sleep(Duration::from_millis(100));
        }
        if closed {
            let _ = super::keyboard::key("escape", 1);
        }
    }
    result
}

fn walk(bus: &Bus, bars: &[Node], path: &[&str], opened: &mut usize) -> Result<(), CmdError> {
    let mut chain: Vec<Node> = Vec::new();
    for (depth, name) in path.iter().enumerate() {
        let wanted = normalize(name);
        let candidates = || match chain.last() {
            None => bars.iter().flat_map(|bar| items(bus, bar)).collect(),
            Some(parent) => items(bus, parent),
        };
        let mut found = pick(candidates(), &wanted);
        // Some menus only fill in their items when they open.
        if found.is_none() && !chain.is_empty() {
            // Firefox closes a menu that is pressed again.
            open(bus, &chain[*opened..])?;
            *opened = chain.len();
            let start = Instant::now();
            while found.is_none() && start.elapsed() < LEVEL_TIMEOUT {
                std::thread::sleep(Duration::from_millis(100));
                found = pick(candidates(), &wanted);
            }
        }
        let Some(item) = found else {
            let at = path[..depth].join(">");
            let place = if at.is_empty() { "the menu bar".to_string() } else { format!("the menu {at}") };
            return Err(CmdError::not_found(format!("no item \"{name}\" in {place}")));
        };
        chain.push(item);
    }
    let item = chain.last().expect("the path is not empty");
    if element::activate(bus, item)? {
        Ok(())
    } else {
        Err(CmdError::unsupported(format!("the menu item \"{}\" has no action to run", path[path.len() - 1])))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn names_compare_without_decoration() {
        assert_eq!(normalize("Save &As..."), "save as");
        assert_eq!(normalize("&Zoom In\tCtrl+Plus"), "zoom in");
        assert_eq!(normalize(" Print… "), "print");
    }
}
