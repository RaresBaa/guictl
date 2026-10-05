use super::dbus;
use crate::output::CmdError;
use serde::Serialize;
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Serialize)]
pub struct NowPlaying {
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub status: &'static str,
}

fn player_proxy<'a>(conn: &Connection, name: &'a str) -> Result<Proxy<'a>, CmdError> {
    Proxy::new(conn, name, PATH, PLAYER).map_err(dbus::err)
}

fn status(conn: &Connection, name: &str) -> Option<String> {
    player_proxy(conn, name).ok()?.get_property::<String>("PlaybackStatus").ok()
}

/// There is no "current session" notion in MPRIS - every player is its own
/// bus name - so this picks the one that's playing, then a paused one, then
/// whatever is left.
fn current_player(conn: &Connection) -> Result<String, CmdError> {
    let names: Vec<String> = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .and_then(|reply| reply.body().deserialize())
        .map_err(dbus::err)?;
    let rank = |name: &String| match status(conn, name).as_deref() {
        Some("Playing") => 0,
        Some("Paused") => 1,
        _ => 2,
    };
    names
        .into_iter()
        .filter(|n| n.starts_with(PREFIX))
        .min_by_key(rank)
        .ok_or_else(|| CmdError::not_found("no media player is running"))
}

fn player_call(method: &str) -> Result<(), CmdError> {
    let conn = dbus::session()?;
    let name = current_player(&conn)?;
    conn.call_method(Some(name.as_str()), PATH, Some(PLAYER), method, &())
        .map(|_| ())
        .map_err(dbus::err)
}

pub fn play() -> Result<(), CmdError> {
    player_call("Play")
}

pub fn pause() -> Result<(), CmdError> {
    player_call("Pause")
}

pub fn next() -> Result<(), CmdError> {
    player_call("Next")
}

pub fn previous() -> Result<(), CmdError> {
    player_call("Previous")
}

fn text(meta: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    <&str>::try_from(&**meta.get(key)?).ok().map(str::to_string)
}

pub fn now_playing() -> Result<NowPlaying, CmdError> {
    let conn = dbus::session()?;
    let name = current_player(&conn)?;
    let proxy = player_proxy(&conn, &name)?;
    let meta: HashMap<String, OwnedValue> =
        proxy.get_property("Metadata").map_err(dbus::err)?;
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    let status = match proxy.get_property::<String>("PlaybackStatus").as_deref() {
        Ok("Playing") => "playing",
        Ok("Paused") => "paused",
        Ok("Stopped") => "stopped",
        _ => "unknown",
    };
    Ok(NowPlaying {
        // The bus name's suffix is the player's name, e.g. `firefox.instance_1_42`.
        app: name[PREFIX.len()..].split('.').next().unwrap_or_default().to_string(),
        title: text(&meta, "xesam:title").unwrap_or_default(),
        artist,
        album: text(&meta, "xesam:album").filter(|a| !a.is_empty()),
        status,
    })
}
