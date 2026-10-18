import system from 'system';
import {
    parseFrames, frameWithLen, subscribeFrame, ema, gazeToPixel, hitTest,
    visibleTiles, filterGaze,
    TAG_GAZE, TAG_SUBSCRIBE, TAG_SUBSCRIBED, STREAM_GAZE,
} from '../gazelib.js';

let failures = 0;
function ok(cond, msg) {
    print(`${cond ? 'ok' : 'NOT OK'} - ${msg}`);
    if (!cond) failures++;
}
const approx = (a, b, eps = 1e-4) => Math.abs(a - b) < eps;

function gazeFrame(tsUs, valid, x, y) {
    const body = new Uint8Array(18);
    const dv = new DataView(body.buffer);
    body[0] = TAG_GAZE;
    dv.setBigInt64(1, BigInt(tsUs), true);
    body[9] = valid ? 1 : 0;
    dv.setFloat32(10, x, true);
    dv.setFloat32(14, y, true);
    return frameWithLen(body);
}
function cat(...arrs) {
    const out = new Uint8Array(arrs.reduce((n, a) => n + a.length, 0));
    let o = 0;
    for (const a of arrs) { out.set(a, o); o += a.length; }
    return out;
}

// subscribe frame layout
{
    const f = subscribeFrame(STREAM_GAZE);
    ok(f.length === 6, 'subscribe frame is 6 bytes');
    ok(f[4] === TAG_SUBSCRIBE && f[5] === STREAM_GAZE, 'subscribe body bytes');
}
// one gaze frame
{
    const { gaze, rest } = parseFrames(gazeFrame(123, true, 0.25, 0.75));
    ok(gaze.length === 1, 'one gaze frame parsed');
    ok(gaze[0].valid && approx(gaze[0].x, 0.25) && approx(gaze[0].y, 0.75), 'gaze decoded');
    ok(rest.length === 0, 'no leftover');
}
// two frames + partial third held back as rest
{
    const full = cat(gazeFrame(1, true, 0.1, 0.2), gazeFrame(2, false, 0.3, 0.4));
    const partial = gazeFrame(3, true, 0.5, 0.6).subarray(0, 10);
    const { gaze, rest } = parseFrames(cat(full, partial));
    ok(gaze.length === 2, 'two complete frames parsed');
    ok(rest.length === 10, 'partial retained as rest');
}
// ema
ok(ema(null, 5, 0.5) === 5, 'ema seeds first value');
ok(approx(ema(0, 10, 0.5), 5), 'ema halfway');
// gazeToPixel
{
    const p = gazeToPixel(0.5, 0.5, { x: 0, y: 0, width: 1000, height: 800 });
    ok(approx(p.px, 500) && approx(p.py, 400), 'gazeToPixel center');
}
// hitTest + hysteresis
{
    const tiles = [
        { x0: 0, y0: 0, w: 100, h: 100, id: 'a' },
        { x0: 100, y0: 0, w: 100, h: 100, id: 'b' },
    ];
    ok(hitTest(tiles, 50, 50, null, 10) === 'a', 'hits a');
    ok(hitTest(tiles, 150, 50, 'a', 10) === 'b', 'moves to b when inside');
    ok(hitTest(tiles, 105, 50, 'a', 10) === 'a', 'hysteresis keeps a near border');
    ok(hitTest(tiles, 300, 50, 'b', 10) === 'b', 'outside keeps previous');
}
// SUBSCRIBED ack decodes for ok and busy
{
    const okFrame = frameWithLen(Uint8Array.from([TAG_SUBSCRIBED, 1]));
    const busyFrame = frameWithLen(Uint8Array.from([TAG_SUBSCRIBED, 0]));
    ok(parseFrames(okFrame).subscribedOk === true, 'subscribed ok=true decoded');
    ok(parseFrames(busyFrame).subscribedOk === false, 'subscribed ok=false decoded');
}
// flush adjacent tiles: the shared edge pixel belongs to the right tile only
{
    const tiles = [
        { x0: 0, y0: 0, w: 100, h: 100, id: 'a' },
        { x0: 100, y0: 0, w: 100, h: 100, id: 'b' },
    ];
    ok(hitTest(tiles, 100, 50, null, 10) === 'b', 'shared edge x=100 belongs to b');
}
// visibleTiles: popup items clipped to the scroll view, id = item index
{
    const clip = { x0: 100, y0: 0, w: 300, h: 100 };
    const tiles = visibleTiles([
        { x0: 0, y0: 0, w: 100, h: 100 },    // scrolled out to the left
        { x0: 50, y0: 0, w: 100, h: 100 },   // half visible
        { x0: 150, y0: 0, w: 100, h: 100 },  // fully visible
        { x0: 400, y0: 0, w: 100, h: 100 },  // scrolled out to the right
    ], clip);
    ok(tiles.length === 2, 'items outside the clip dropped');
    ok(tiles[0].id === 1 && tiles[0].x0 === 100 && tiles[0].w === 50,
        'half-visible item clipped, keeps its index');
    ok(tiles[1].id === 2 && tiles[1].x0 === 150 && tiles[1].w === 100 &&
        tiles[1].y0 === 0 && tiles[1].h === 100, 'visible item unchanged');
    ok(hitTest(tiles, 60, 50, null, 10) === null, 'scrolled-out area hits nothing');
}
// filterGaze: EMA while fixating, restart at the sample on a saccade
{
    const p = filterGaze(null, { x: 10, y: 20 }, 0.5, 48);
    ok(p.x === 10 && p.y === 20, 'filterGaze seeds with the first sample');
    const f = filterGaze({ x: 0, y: 0 }, { x: 20, y: 0 }, 0.5, 48);
    ok(approx(f.x, 10) && approx(f.y, 0), 'small move is smoothed');
    const n = filterGaze({ x: 0, y: 0 }, { x: 30, y: 30 }, 0.5, 48);
    ok(approx(n.x, 15) && approx(n.y, 15), 'move just under the threshold (42 px) is smoothed');
    const j = filterGaze({ x: 0, y: 0 }, { x: 40, y: 30 }, 0.5, 48);
    ok(j.x === 40 && j.y === 30, 'move past the threshold (50 px) jumps to the sample');
}

print(failures === 0 ? 'ALL PASS' : `FAILED ${failures}`);
if (failures > 0) system.exit(1);
