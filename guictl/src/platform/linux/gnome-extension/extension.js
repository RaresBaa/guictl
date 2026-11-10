// guictl's GNOME Shell extension. GNOME gives programs outside the shell no
// way to list or arrange windows, take screenshots without a dialog, or own
// the clipboard after they exit, so guictl asks the shell to do it through a
// D-Bus object this extension exports. It answers only the executables named
// in `allowed-executables` next to this file, which `guictl setup` writes.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// Bumped whenever the methods change, so guictl can tell an old copy apart.
const VERSION = 1;
const OBJECT_PATH = '/io/github/guictl/Shell';
const INTERFACE = `<node>
  <interface name="io.github.guictl.Shell">
    <method name="Call">
      <arg type="s" direction="in" name="method"/>
      <arg type="s" direction="in" name="args"/>
      <arg type="s" direction="out" name="result"/>
    </method>
  </interface>
</node>`;

class NotFound extends Error {}

function now() {
    return global.display.get_current_time_roundtrip();
}

function windowId(w) {
    return String(w.get_id());
}

function allWindows() {
    const windows = global.get_window_actors().map(a => a.meta_window).filter(w => w);
    return global.display.sort_windows_by_stacking(windows);
}

function findWindow(id) {
    const w = allWindows().find(w => windowId(w) === String(id));
    if (!w)
        throw new NotFound(`no window with id ${id}`);
    return w;
}

function isMaximized(w) {
    // Mutter 49 replaced get_maximized() with is_maximized().
    if (w.is_maximized)
        return w.is_maximized();
    return w.get_maximized() === Meta.MaximizeFlags.BOTH;
}

function maximize(w) {
    if (w.is_maximized)
        w.maximize();
    else
        w.maximize(Meta.MaximizeFlags.BOTH);
}

function unmaximize(w) {
    if (w.is_maximized)
        w.unmaximize();
    else
        w.unmaximize(Meta.MaximizeFlags.BOTH);
}

const POPUP_TYPES = [
    Meta.WindowType.DROPDOWN_MENU, Meta.WindowType.POPUP_MENU, Meta.WindowType.MENU,
    Meta.WindowType.COMBO, Meta.WindowType.TOOLTIP,
];

function isPopup(w) {
    return POPUP_TYPES.includes(w.get_window_type());
}

function describe(w, z) {
    const f = w.get_frame_rect();
    const b = w.get_buffer_rect();
    const type = w.get_window_type();
    const index = w.get_monitor();
    const monitor = index >= 0 ? global.display.get_monitor_geometry(index) : null;
    const parent = w.get_transient_for();
    const workspace = w.get_workspace();
    return {
        id: windowId(w),
        title: w.get_title() ?? '',
        class: w.get_wm_class() ?? w.get_gtk_application_id() ?? '',
        pid: w.get_pid(),
        x: f.x, y: f.y, w: f.width, h: f.height,
        bx: b.x, by: b.y, bw: b.width, bh: b.height,
        state: w.minimized ? 'minimized'
            : w.is_fullscreen() ? 'fullscreen'
            : isMaximized(w) ? 'maximized'
            : 'normal',
        monitor_x: monitor ? monitor.x : null,
        monitor_y: monitor ? monitor.y : null,
        z,
        // Panels, the desktop and popups are windows too; only normal
        // windows and dialogs are worth listing.
        listed: [Meta.WindowType.NORMAL, Meta.WindowType.DIALOG, Meta.WindowType.MODAL_DIALOG]
            .includes(type) && !w.is_skip_taskbar(),
        active: w === global.display.focus_window,
        owner: parent ? windowId(parent) : '',
        modal: type === Meta.WindowType.MODAL_DIALOG,
        popup: isPopup(w),
        workspace: workspace && !w.is_on_all_workspaces() ? workspace.index() + 1 : 0,
    };
}

function windows() {
    return allWindows().map((w, z) => describe(w, z));
}

/// The window and its open menus and dropdowns, lowest first. A submenu
/// belongs to the menu it opened from, so the walk goes up through popups.
function withPopups(w) {
    const popups = [];
    allWindows().forEach((p, z) => {
        if (p === w || !isPopup(p))
            return;
        let up = p;
        while (up && up !== w && isPopup(up))
            up = up.get_transient_for();
        const ownerless = !up && p.get_pid() === w.get_pid() && w === global.display.focus_window;
        if (up === w || ownerless)
            popups.push(describe(p, z));
    });
    return {window: describe(w, 0), popups};
}

function windowAction(a) {
    const w = findWindow(a.id);
    switch (a.action) {
    case 'describe':
        return describe(w, 0);
    case 'popups':
        return withPopups(w);
    case 'focus':
        Main.activateWindow(w);
        return null;
    case 'move': {
        if (isMaximized(w))
            unmaximize(w);
        w.move_frame(true, a.x, a.y);
        return null;
    }
    case 'resize': {
        if (isMaximized(w))
            unmaximize(w);
        const f = w.get_frame_rect();
        w.move_resize_frame(true, f.x, f.y, a.w, a.h);
        return null;
    }
    case 'minimize':
        w.minimize();
        return null;
    case 'maximize':
        w.unminimize();
        if (w.is_fullscreen())
            w.unmake_fullscreen();
        maximize(w);
        return null;
    case 'restore':
        w.unminimize();
        if (w.is_fullscreen())
            w.unmake_fullscreen();
        if (isMaximized(w))
            unmaximize(w);
        return null;
    case 'fullscreen':
        w.unminimize();
        w.make_fullscreen();
        return null;
    case 'close':
        w.delete(now());
        return null;
    case 'pin':
        if (a.on)
            w.make_above();
        else
            w.unmake_above();
        return null;
    case 'send':
        w.change_workspace_by_index(findWorkspace(a.workspace).index(), false);
        return null;
    default:
        throw new Error(`unknown window action ${a.action}`);
    }
}

function workspaceNames() {
    return new Gio.Settings({schema_id: 'org.gnome.desktop.wm.preferences'});
}

function describeWorkspace(ws, i = ws.index()) {
    return {
        id: String(i + 1),
        name: Meta.prefs_get_workspace_name(i),
        index: i + 1,
        current: ws === global.workspace_manager.get_active_workspace(),
    };
}

function findWorkspace(id) {
    const manager = global.workspace_manager;
    const n = Number(id);
    if (!Number.isInteger(n) || n < 1 || n > manager.n_workspaces)
        throw new NotFound(`no workspace with id ${id}`);
    return manager.get_workspace_by_index(n - 1);
}

function workspaces() {
    const manager = global.workspace_manager;
    const out = [];
    for (let i = 0; i < manager.n_workspaces; i++)
        out.push(describeWorkspace(manager.get_workspace_by_index(i)));
    return out;
}

function isEmpty(ws) {
    return ws.list_windows().every(w => w.is_on_all_workspaces() || w.is_skip_taskbar());
}

function createWorkspace(a) {
    const manager = global.workspace_manager;
    const last = manager.get_workspace_by_index(manager.n_workspaces - 1);
    // With dynamic workspaces GNOME keeps one empty workspace at the end and
    // removes any other empty one, so that one is the new workspace.
    const dynamic = new Gio.Settings({schema_id: 'org.gnome.mutter'}).get_boolean('dynamic-workspaces');
    const ws = dynamic && isEmpty(last) ? last : manager.append_new_workspace(false, now());
    const index = dynamic ? settledIndex(ws) : ws.index();
    if (a.name) {
        const settings = workspaceNames();
        const names = settings.get_strv('workspace-names');
        while (names.length <= index)
            names.push('');
        names[index] = a.name;
        settings.set_strv('workspace-names', names);
    }
    return describeWorkspace(ws, index);
}

// GNOME's check of dynamic workspaces runs later, from an idle callback, and
// removes every empty workspace but the active one and the last. Right after
// a window leaves one, more than one can still be empty, so give the index
// this workspace will have once that check has run.
function settledIndex(ws) {
    const manager = global.workspace_manager;
    const active = manager.get_active_workspace();
    let index = 0;
    for (let i = 0; i < ws.index(); i++) {
        const other = manager.get_workspace_by_index(i);
        if (other === active || !isEmpty(other))
            index++;
    }
    return index;
}

function workspaceAction(a) {
    switch (a.action) {
    case 'list':
        return workspaces();
    case 'create':
        return createWorkspace(a);
    case 'remove':
        global.workspace_manager.remove_workspace(findWorkspace(a.id), now());
        return null;
    case 'switch':
        findWorkspace(a.id).activate(now());
        return null;
    default:
        throw new Error(`unknown workspace action ${a.action}`);
    }
}

function outputStream(path) {
    return Gio.File.new_for_path(path).replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
}

/// Starts an async call and resolves with its GAsyncResult, for the matching
/// `_finish` to read.
function finish(start) {
    return new Promise(resolve => start((_source, res) => resolve(res)));
}

async function screenshot(a) {
    const stream = outputStream(a.path);
    try {
        if (a.target === 'window') {
            const w = findWindow(a.id);
            const actor = w.get_compositor_private();
            if (!actor)
                throw new NotFound(`window ${a.id} is not on screen`);
            const content = actor.paint_to_content(null);
            if (!content)
                throw new Error('the window has nothing drawn yet');
            const texture = content.get_texture();
            // Crop to the frame so the image leaves out the shadow and
            // matches the window's frame geometry. The texture is in the
            // window's own pixels, which a scaled display multiplies.
            const scale = actor.width > 0 ? texture.get_width() / actor.width : 1;
            const f = w.get_frame_rect();
            const crop = v => Math.max(0, Math.round(v * scale));
            const res = await finish(cb => Shell.Screenshot.composite_to_stream(
                texture, crop(f.x - actor.x), crop(f.y - actor.y),
                Math.min(crop(f.width), texture.get_width() - crop(f.x - actor.x)),
                Math.min(crop(f.height), texture.get_height() - crop(f.y - actor.y)),
                1, null, 0, 0, 1, stream, cb));
            Shell.Screenshot.composite_to_stream_finish(res);
        } else {
            const shooter = new Shell.Screenshot();
            if (a.target === 'area') {
                const res = await finish(cb => shooter.screenshot_area(a.x, a.y, a.w, a.h, stream, cb));
                shooter.screenshot_area_finish(res);
            } else {
                const res = await finish(cb => shooter.screenshot(!!a.cursor, stream, cb));
                shooter.screenshot_finish(res);
            }
        }
    } finally {
        stream.close(null);
    }
    return null;
}

const CLIPBOARD = St.ClipboardType.CLIPBOARD;
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/bmp', 'image/gif'];

async function clipboard(a) {
    const clip = St.Clipboard.get_default();
    switch (a.action) {
    case 'get_text':
        return await new Promise(resolve => clip.get_text(CLIPBOARD, (_, text) => resolve(text ?? '')));
    case 'set_text':
        clip.set_text(CLIPBOARD, a.text);
        return null;
    case 'get_image': {
        const types = clip.get_mimetypes(CLIPBOARD);
        const mime = IMAGE_TYPES.find(t => types.includes(t));
        if (!mime)
            throw new NotFound('the clipboard does not hold an image');
        // Straight from the selection into the file: St.Clipboard's
        // get_content hands JS a buffer that crashed GNOME Shell 50.
        const selection = global.display.get_selection();
        const stream = outputStream(a.path);
        try {
            const res = await finish(cb => selection.transfer_async(
                Meta.SelectionType.SELECTION_CLIPBOARD, mime, -1, stream, null, cb));
            selection.transfer_finish(res);
        } finally {
            stream.close(null);
        }
        return mime;
    }
    case 'set_image': {
        const [, contents] = GLib.file_get_contents(a.path);
        clip.set_content(CLIPBOARD, 'image/png', new GLib.Bytes(contents));
        return null;
    }
    default:
        throw new Error(`unknown clipboard action ${a.action}`);
    }
}

async function dispatch(method, a) {
    switch (method) {
    case 'version':
        return VERSION;
    case 'windows':
        return windows();
    case 'window':
        return windowAction(a);
    case 'workspace':
        return workspaceAction(a);
    case 'cursor': {
        const [x, y] = global.get_pointer();
        return [x, y];
    }
    case 'screenshot':
        return await screenshot(a);
    case 'clipboard':
        return await clipboard(a);
    default:
        throw new Error(`unknown method ${method}`);
    }
}

function callerPid(sender) {
    return new Promise((resolve, reject) => {
        Gio.DBus.session.call(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [sender]),
            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, -1, null,
            (conn, res) => {
                try {
                    resolve(conn.call_finish(res).deep_unpack()[0]);
                } catch (e) {
                    reject(e);
                }
            });
    });
}

class Service {
    constructor(dir) {
        this._dir = dir;
    }

    _allowed() {
        try {
            const [, bytes] = this._dir.get_child('allowed-executables').load_contents(null);
            return new TextDecoder().decode(bytes).split('\n').map(s => s.trim()).filter(s => s);
        } catch {
            return [];
        }
    }

    async _authorized(sender) {
        const pid = await callerPid(sender);
        let exe;
        try {
            exe = GLib.file_read_link(`/proc/${pid}/exe`);
        } catch {
            return false;
        }
        return this._allowed().includes(exe);
    }

    async CallAsync([method, args], invocation) {
        let out;
        try {
            if (!await this._authorized(invocation.get_sender())) {
                out = {ok: false, error: 'not_authorized: this executable is not allowed to use the guictl extension; run `guictl setup` (again if the binary moved)'};
            } else {
                const value = await dispatch(method, JSON.parse(args || '{}'));
                out = {ok: true, value: value ?? null};
            }
        } catch (e) {
            const prefix = e instanceof NotFound ? 'not_found: ' : '';
            out = {ok: false, error: prefix + (e?.message ?? String(e))};
        }
        invocation.return_value(new GLib.Variant('(s)', [JSON.stringify(out)]));
    }
}

export default class GuictlExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, new Service(this.dir));
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
    }
}
