// tobii gaze switcher: while the Alt+Tab switcher is open, select the tile
// under the user's gaze, as if the pointer entered it. Reads tobiid's gaze
// stream over a Unix socket (async Gio). Free of resource:// imports:
// extension.js passes the shell objects in, so tests run it under plain gjs.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {
    subscribeFrame, parseFrames, filterGaze, gazeToPixel, hitTest, visibleTiles, STREAM_GAZE,
} from './gazelib.js';

const EMA_ALPHA = 0.4;        // smoothing while fixating (0..1; higher = snappier)
const JUMP_PX = 48;           // a move past this is a saccade: follow at once
const HYSTERESIS_PX = 24;     // border deadband before switching tiles
const RECONNECT_MS = 1000;    // backoff between reconnect attempts

export class GazeSwitcher {
    // uiGroup: the actor switcher popups add themselves to (Main.uiGroup).
    // isSwitcher(actor): whether an added actor is a switcher popup.
    // monitor(): the monitor the popups show on, {x, y, width, height}.
    constructor({ uiGroup, isSwitcher, monitor }) {
        this._uiGroup = uiGroup;
        this._isSwitcher = isSwitcher;
        this._monitor = monitor;
        this._cancellable = new Gio.Cancellable();
        this._conn = null;
        this._istream = null;
        this._buf = new Uint8Array(0);
        this._gaze = null;        // filtered gaze point, pixels {x, y}
        this._reconnectId = 0;
        this._popup = null;       // open switcher popup, if any
        this._prevId = null;      // previously selected item index
        uiGroup.connectObject(
            'child-added', (_, actor) => this._onChildAdded(actor), this);
        this._connect();
    }

    _socketPath() {
        const r = GLib.getenv('XDG_RUNTIME_DIR');
        return r ? `${r}/tobiid.sock` : '/tmp/tobiid.sock';
    }

    _connect() {
        const client = new Gio.SocketClient();
        const addr = new Gio.UnixSocketAddress({ path: this._socketPath() });
        client.connect_async(addr, this._cancellable, (c, res) => {
            let conn;
            try {
                conn = c.connect_finish(res);
            } catch (e) {
                if (!this._cancellable.is_cancelled()) this._scheduleReconnect();
                return;
            }
            this._conn = conn;
            this._istream = conn.get_input_stream();
            try {
                conn.get_output_stream().write_all(
                    subscribeFrame(STREAM_GAZE), this._cancellable);
            } catch (e) {
                this._reconnect();
                return;
            }
            this._buf = new Uint8Array(0);
            this._readChunk();
        });
    }

    _scheduleReconnect() {
        if (this._cancellable.is_cancelled() || this._reconnectId) return;
        this._reconnectId = GLib.timeout_add(
            GLib.PRIORITY_DEFAULT, RECONNECT_MS, () => {
                this._reconnectId = 0;
                if (this._cancellable.is_cancelled()) return GLib.SOURCE_REMOVE;
                this._connect();
                return GLib.SOURCE_REMOVE;
            });
    }

    _readChunk() {
        if (!this._istream || this._cancellable.is_cancelled()) return;
        this._istream.read_bytes_async(
            4096, GLib.PRIORITY_DEFAULT, this._cancellable, (s, res) => {
                let bytes;
                try {
                    bytes = s.read_bytes_finish(res);
                } catch (e) {
                    if (!this._cancellable.is_cancelled()) this._reconnect();
                    return;
                }
                const arr = bytes.get_data();
                if (!arr || arr.length === 0) {
                    if (!this._cancellable.is_cancelled()) this._reconnect();
                    return;
                }
                const merged = new Uint8Array(this._buf.length + arr.length);
                merged.set(this._buf, 0);
                merged.set(arr, this._buf.length);
                const { gaze, rest } = parseFrames(merged);
                this._buf = Uint8Array.from(rest);
                for (const g of gaze) this._onGaze(g);
                this._readChunk();
            });
    }

    _reconnect() {
        try { this._conn?.close(null); } catch (e) {}
        this._conn = null;
        this._istream = null;
        this._buf = new Uint8Array(0);
        this._scheduleReconnect();
    }

    _onGaze(g) {
        if (!g.valid) return;
        const { px, py } = gazeToPixel(g.x, g.y, this._monitor());
        this._gaze = filterGaze(this._gaze, { x: px, y: py }, EMA_ALPHA, JUMP_PX);
        if (this._popup) this._maybeSelect(this._popup);
    }

    _onChildAdded(actor) {
        if (!this._isSwitcher(actor)) return;
        this._popup = actor;
        this._prevId = null;
        actor.connectObject('destroy', () => {
            if (this._popup === actor) this._popup = null;
        }, this);
    }

    _rectOf(actor) {
        const [x0, y0] = actor.get_transformed_position();
        const [w, h] = actor.get_transformed_size();
        return { x0, y0, w, h };
    }

    _maybeSelect(popup) {
        const list = popup._switcherList;
        if (!Array.isArray(list?._items) || !list._scrollView) return;
        // Not before the popup shows (150 ms in; a quick Alt+Tab must keep
        // its pick) nor once it is accepted and fading out.
        if (!popup._haveModal || popup.opacity < 255) return;
        const tiles = visibleTiles(
            list._items.map(item => this._rectOf(item)),
            this._rectOf(list._scrollView));
        if (tiles.length === 0) return;
        const i = hitTest(tiles, this._gaze.x, this._gaze.y, this._prevId, HYSTERESIS_PX);
        if (i === null || i === this._prevId) return;
        if (i !== popup._selectedIndex) {
            try {
                popup._itemEnteredHandler(i);
            } catch (e) {
                console.log(`#tobii-gaze select failed: ${e}`);
            }
        }
        this._prevId = i;
    }

    destroy() {
        this._cancellable.cancel();
        this._uiGroup.disconnectObject(this);
        this._popup?.disconnectObject(this);
        this._popup = null;
        if (this._reconnectId) {
            GLib.source_remove(this._reconnectId);
            this._reconnectId = 0;
        }
        try { this._conn?.close(null); } catch (e) {}
        this._conn = null;
        this._istream = null;
        this._buf = null;
    }
}
