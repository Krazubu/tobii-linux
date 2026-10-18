// Pure, gi-free helpers for the tobii gaze switcher. Imported by both
// gaze.js (inside GNOME Shell) and the standalone gjs test/probe scripts.
// Keep this file free of gi:// / resource:// imports so it runs under `gjs -m`.

// tobiid IPC constants — mirror crates/tobii-ipc/src/lib.rs, whose table also
// lists the retired numbers that no new constant may take.
export const TAG_SUBSCRIBE = 0x01;
export const TAG_SUBSCRIBED = 0x10;
export const TAG_GAZE = 0x21;
export const STREAM_GAZE = 0x02;

// Prefix a frame body with its u32 little-endian length.
export function frameWithLen(body) {
    const out = new Uint8Array(4 + body.length);
    new DataView(out.buffer).setUint32(0, body.length, true);
    out.set(body, 4);
    return out;
}

// A complete SUBSCRIBE frame (length-prefixed) for the given stream bitmask.
export function subscribeFrame(streams = STREAM_GAZE) {
    return frameWithLen(Uint8Array.from([TAG_SUBSCRIBE, streams]));
}

// Parse all complete frames in `buf`. Returns decoded gaze samples, the last
// SUBSCRIBED ack seen (or null), and the unconsumed tail bytes (`rest`).
// A sample's `tsUs` is when it was taken, in host CLOCK_MONOTONIC
// microseconds: comparable with GLib.get_monotonic_time().
export function parseFrames(buf) {
    const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    const gaze = [];
    let subscribedOk = null;
    let off = 0;
    while (buf.length - off >= 4) {
        const len = dv.getUint32(off, true);
        if (buf.length - off - 4 < len)
            break; // body not fully arrived yet
        const b = off + 4;
        const tag = buf[b];
        if (tag === TAG_GAZE && len >= 18) {
            gaze.push({
                tsUs: Number(dv.getBigInt64(b + 1, true)),
                valid: buf[b + 9] !== 0,
                x: dv.getFloat32(b + 10, true),
                y: dv.getFloat32(b + 14, true),
            });
        } else if (tag === TAG_SUBSCRIBED && len >= 2) {
            subscribedOk = buf[b + 1] !== 0;
        }
        off = b + len;
    }
    return { gaze, subscribedOk, rest: buf.subarray(off) };
}

// Exponential moving average for a scalar; seeds with `cur` on first call.
export function ema(prev, cur, alpha) {
    if (prev === null || prev === undefined)
        return cur;
    return prev + alpha * (cur - prev);
}

// One step of the gaze filter over points `{x, y}`: smooth while fixating
// (EMA), follow a saccade at once. A sample farther than `jump` from the
// filtered point restarts the filter there, instead of the EMA crawling over
// for a few samples.
export function filterGaze(prev, cur, alpha, jump) {
    if (!prev || Math.hypot(cur.x - prev.x, cur.y - prev.y) > jump)
        return { x: cur.x, y: cur.y };
    return { x: ema(prev.x, cur.x, alpha), y: ema(prev.y, cur.y, alpha) };
}

// Map gaze (gx,gy in 0..1) to an absolute pixel on `monitor`.
export function gazeToPixel(gx, gy, monitor) {
    return {
        px: monitor.x + gx * monitor.width,
        py: monitor.y + gy * monitor.height,
    };
}

// Tiles of a switcher popup: each item rect `{x0,y0,w,h}` cut to `clip` (the
// popup's visible scroll area), with its item index as id. Items scrolled out
// of view are dropped, so gaze on empty screen next to the popup selects none.
export function visibleTiles(rects, clip) {
    const tiles = [];
    rects.forEach((r, i) => {
        const x0 = Math.max(r.x0, clip.x0);
        const y0 = Math.max(r.y0, clip.y0);
        const x1 = Math.min(r.x0 + r.w, clip.x0 + clip.w);
        const y1 = Math.min(r.y0 + r.h, clip.y0 + clip.h);
        if (x1 > x0 && y1 > y0)
            tiles.push({ x0, y0, w: x1 - x0, h: y1 - y0, id: i });
    });
    return tiles;
}

// Find the id of the tile under (px,py). Border hysteresis: when crossing into
// a new tile, require being inside it by `margin` before switching; while
// outside every tile, keep the previous id. `tiles`: [{x0,y0,w,h,id}].
export function hitTest(tiles, px, py, prevId, margin) {
    let hit = null;
    for (const t of tiles) {
        if (px >= t.x0 && px < t.x0 + t.w && py >= t.y0 && py < t.y0 + t.h) {
            hit = t.id;
            break;
        }
    }
    if (hit !== null && hit !== prevId && prevId !== null && prevId !== undefined) {
        const t = tiles.find(t => t.id === hit);
        const inset = px >= t.x0 + margin && px <= t.x0 + t.w - margin &&
                      py >= t.y0 + margin && py <= t.y0 + t.h - margin;
        if (!inset)
            return prevId; // not clearly inside the new tile yet
    }
    return hit !== null ? hit : (prevId ?? null);
}
