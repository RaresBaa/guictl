use super::keyboard::tap;
use crate::output::CmdError;
use serde::Serialize;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};

fn current_session() -> Option<windows::Media::Control::GlobalSystemMediaTransportControlsSession> {
    SessionManager::RequestAsync().ok()?.get().ok()?.GetCurrentSession().ok()
}

/// Asks the current media session to play, so that `play` on something
/// already playing changes nothing. Without a session there's only the
/// hardware play/pause key, which toggles.
pub fn play() -> Result<(), CmdError> {
    match current_session() {
        Some(session) => session
            .TryPlayAsync()
            .and_then(|op| op.get())
            .map(|_| ())
            .map_err(|e| CmdError::os(format!("could not start playback: {e}"))),
        None => tap(VK_MEDIA_PLAY_PAUSE),
    }
}

pub fn pause() -> Result<(), CmdError> {
    match current_session() {
        Some(session) => session
            .TryPauseAsync()
            .and_then(|op| op.get())
            .map(|_| ())
            .map_err(|e| CmdError::os(format!("could not pause playback: {e}"))),
        None => tap(VK_MEDIA_PLAY_PAUSE),
    }
}

pub fn next() -> Result<(), CmdError> {
    tap(VK_MEDIA_NEXT_TRACK)
}

pub fn previous() -> Result<(), CmdError> {
    tap(VK_MEDIA_PREV_TRACK)
}

#[derive(Serialize)]
pub struct NowPlaying {
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub status: &'static str,
}

/// Uses the same session-manager WinRT API (`Windows.Media.Control`) that
/// Windows' own volume mixer and media overlay read from - unlike the media
/// keys above, this is a public, documented interface. "Current session" is
/// whichever app Windows currently considers the active media source, which
/// may not be the most recently launched one.
pub fn now_playing() -> Result<NowPlaying, CmdError> {
    let manager = SessionManager::RequestAsync()
        .map_err(|e| CmdError::os(format!("could not request the media session manager: {e}")))?
        .get()
        .map_err(|e| CmdError::os(format!("could not request the media session manager: {e}")))?;

    let session = manager
        .GetCurrentSession()
        .map_err(|_| CmdError::not_found("no active media session"))?;

    let props = session
        .TryGetMediaPropertiesAsync()
        .map_err(|e| CmdError::os(format!("could not read media properties: {e}")))?
        .get()
        .map_err(|e| CmdError::os(format!("could not read media properties: {e}")))?;

    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().ok().map(|s| s.to_string()).filter(|s| !s.is_empty());

    let status = session
        .GetPlaybackInfo()
        .ok()
        .and_then(|info| info.PlaybackStatus().ok())
        .map(playback_status_str)
        .unwrap_or("unknown");

    let app = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();

    Ok(NowPlaying { app, title, artist, album, status })
}

fn playback_status_str(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "playing",
        PlaybackStatus::Paused => "paused",
        PlaybackStatus::Stopped => "stopped",
        PlaybackStatus::Changing => "changing",
        PlaybackStatus::Opened => "opened",
        PlaybackStatus::Closed => "closed",
        _ => "unknown",
    }
}
