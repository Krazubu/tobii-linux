// GazeSwitcher against fake shell actors: which popup item gaze selects, when.
import system from 'system';
import { GazeSwitcher } from '../gaze.js';

let failures = 0;
function ok(cond, msg) {
    print(`${cond ? 'ok' : 'NOT OK'} - ${msg}`);
    if (!cond) failures++;
}

// The slice of the shell's GObject.Object.connectObject these tests need.
class FakeActor {
    constructor(x0 = 0, y0 = 0, w = 0, h = 0) {
        this._rect = [x0, y0, w, h];
        this._handlers = [];
    }
    get_transformed_position() { return this._rect.slice(0, 2); }
    get_transformed_size() { return this._rect.slice(2); }
    connectObject(...args) {
        const owner = args.pop();
        for (let i = 0; i < args.length; i += 2)
            this._handlers.push({ sig: args[i], fn: args[i + 1], owner });
    }
    disconnectObject(owner) {
        this._handlers = this._handlers.filter(h => h.owner !== owner);
    }
    emit(sig, ...a) {
        for (const h of [...this._handlers]) if (h.sig === sig) h.fn(this, ...a);
    }
}

// A shown switcher popup: four 200x200 items at y 620, the scroll view
// showing x 1000..1600 only (item 0 scrolled out), item 1 preselected.
class FakePopup extends FakeActor {
    constructor() {
        super();
        this.isSwitcher = true;
        this._haveModal = true;
        this.opacity = 255;
        this._selectedIndex = 1;
        this.entered = [];
        this._switcherList = {
            _items: [800, 1000, 1200, 1400].map(x => new FakeActor(x, 620, 200, 200)),
            _scrollView: new FakeActor(1000, 600, 600, 240),
        };
    }
    _itemEnteredHandler(i) {
        this.entered.push(i);
        this._selectedIndex = i;
    }
}

const MONITOR = { x: 0, y: 0, width: 2560, height: 1440 };
GazeSwitcher.prototype._connect = () => {}; // no tobiid in tests
const uiGroup = new FakeActor();
const gs = new GazeSwitcher({
    uiGroup, isSwitcher: a => a.isSwitcher === true, monitor: () => MONITOR,
});
// One fresh (unsmoothed) gaze sample at pixel (x, y).
function look(x, y) {
    gs._gaze = null;
    glance(x, y);
}
// The next gaze sample at pixel (x, y), filtered as usual.
function glance(x, y) {
    gs._onGaze({ valid: true, x: x / MONITOR.width, y: y / MONITOR.height });
}

uiGroup.emit('child-added', new FakeActor());
ok(gs._popup === null, 'non-switcher actor ignored');

const p = new FakePopup();
p.opacity = 0;
uiGroup.emit('child-added', p);
ok(gs._popup === p, 'popup tracked once added');
look(1300, 720);
ok(p.entered.length === 0, 'no selection while the popup is still hidden');

p.opacity = 255;
look(1300, 720);
ok(p.entered.join() === '2', 'gaze on item 2 selects it');
for (let k = 0; k < 5; k++) look(1300, 720);
ok(p.entered.join() === '2', 'steady gaze does not reselect');

p._selectedIndex = 3; // Tab pressed
look(1300, 720);
ok(p._selectedIndex === 3, 'keyboard choice kept while gaze stays put');

look(900, 720);
ok(p.entered.join() === '2', 'scrolled-out item 0 not selectable');
look(1100, 720);
ok(p.entered.join() === '2,1', 'gaze on item 1 selects it');
look(1210, 720);
ok(p.entered.join() === '2,1', 'hysteresis near the 1|2 border keeps 1');
gs._onGaze({ valid: false, x: -1, y: -1 });
ok(p.entered.join() === '2,1', 'invalid sample ignored');

// Looking from item 1 to item 2: the first sample there switches, rather
// than smoothing crawling over the border for a few samples.
look(1100, 720);
p._selectedIndex = 1;
glance(1300, 720);
ok(p.entered.join() === '2,1,2', 'saccade to the next item selects it on the first sample');
glance(1290, 720);
glance(1310, 720);
ok(p.entered.join() === '2,1,2', 'fixation jitter keeps the item');

p._haveModal = false; // accepted, fading out
look(1500, 720);
ok(p.entered.join() === '2,1,2', 'no selection once accepted');

p.emit('destroy');
ok(gs._popup === null, 'popup forgotten once destroyed');

const p2 = new FakePopup();
uiGroup.emit('child-added', p2);
look(1100, 720);
ok(p2.entered.length === 0, 'new popup: preselected item 1 not re-entered');
look(1300, 720);
ok(p2.entered.join() === '2', 'new popup: gaze selects without a stale previous');

gs.destroy();
ok(uiGroup._handlers.length === 0 && p2._handlers.length === 0,
    'destroy disconnects every handler');

print(failures === 0 ? 'ALL PASS' : `FAILED ${failures}`);
if (failures > 0) system.exit(1);
