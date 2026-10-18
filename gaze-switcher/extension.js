// tobii gaze switcher: GNOME Shell entry point. See gaze.js.

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as SwitcherPopup from 'resource:///org/gnome/shell/ui/switcherPopup.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

import { GazeSwitcher } from './gaze.js';

export default class TobiiGazeSwitcherExtension extends Extension {
    enable() {
        this._switcher = new GazeSwitcher({
            uiGroup: Main.uiGroup,
            isSwitcher: actor => actor instanceof SwitcherPopup.SwitcherPopup,
            // SwitcherPopup lays itself out on the primary monitor.
            monitor: () => Main.layoutManager.primaryMonitor,
        });
        console.log('#tobii-gaze switcher enabled');
    }

    disable() {
        this._switcher?.destroy();
        this._switcher = null;
        console.log('#tobii-gaze switcher disabled');
    }
}
