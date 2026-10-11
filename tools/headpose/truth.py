#!/usr/bin/env python3
"""The head pose's angles against laser-measured truth: how far the pose turns for a known turn.

    truth.py --session LOG JSONL FITS [--session ...] [--holds laser_holds.json]
             [--targets laser_targets.json] [--params FITTED.json] [--fitted FITTED.json]
             [--gates] [--holds-table]

Each session is a recording (tools/headpose/README.md, "Laser truth") in which a laser pointer
fixed to the head was aimed at the rings of laser_targets.py on the screen, the head held still on
each for a few seconds. laser_holds.json gives, per session (found by its clock offset and image
count, as gates.json finds them), the holds (seconds after the session's first image, and the
ring), where the screen's centre lies in the display frame T (the display area the runtime was
set up with need not be the screen) and where the beam's line passes the head (up and back from
the device's eyeball centres, mm, in the head's frame).

For each hold, over its images where the DLL's pose and ours are valid: the means of the DLL's
yxz angles, ours, and the midpoint of the device's eyeball centres 0x17/0x18 (T). The beam leaves
that midpoint moved by the beam offset (turned with the DLL's rotation) and ends on the ring; its
direction is the true facing: yaw = atan2(-dx, -dz) (left +), pitch = atan2(dy, |(dx, dz)|) (up
+). At the 20 and 25 deg yaw rings the device's eyeball-centre range runs 2-8 % long (the face's
size in the images, the device's own interocular distance and our PnP agree there on about 1 %),
which puts the beam's origin too far from the screen and under-states the turn; there the eye
midpoint is where a rigid turn of the neck puts it instead (neck_eyes: from the neighbouring '0'
holds' midpoint, about a vertical axis behind it, at the hold's own true yaw), the lever fitted per
session on the 5 and 10 deg rings, where the range holds (neck_lever). The pointer may sit askew
on the head, so only changes of angle count: the slopes, per axis
and session, of a pose's angles on the true ones (ordinary least squares with an intercept), yaw
over the holds of the '0' ring and the L/R rings, pitch over those of '0' and U/D (the four
combined rings are listed, not fitted), with a 95 % bootstrap interval over the holds. Three
poses: the DLL's; the core, ours with every rotation gain 1 (the Stream Engine's scale the other
constants are fitted to); and the shipped one, ours with --params' gains (HeadParams::FITTED,
fitted.json, by default).

The gains: per axis, 1 / the core's slope over every session (the sessions' holds pooled, each
session its own intercept); and each session's slope with the gains fitted on the others (leave
one session out). --fitted writes them into a fitted.json (head_params.rotation_gain, x y z, z 1:
the laser does not see roll) with the fingerprint again and what they were fitted on. --gates
checks the shipped slopes against laser_holds.json's gates; exit 1 when one fails.
"""

import argparse
import datetime
import json
import math
import os
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import common as C  # noqa: E402
import reference as ref  # noqa: E402

HOLDS = os.path.join(HERE, "laser_holds.json")
TARGETS = os.path.join(HERE, "laser_targets.json")
AXES = (("yaw", 1), ("pitch", 0))
BOOT = 2000
# The yaw rings (nominal deg) whose eye midpoint the neck model gives, and those its lever is
# fitted on (module docstring).
NECK_RINGS_DEG = (20, 25)
LEVER_RINGS_DEG = (5, 10)
# The neck levers taken for one (mm); another is a fit gone wrong, and the device's eyes stay.
LEVER_BOUNDS_MM = (50.0, 150.0)


def entry_of(doc, o):
    """laser_holds.json's entry for session o, by clock offset and image count."""
    for s in doc["sessions"]:
        if s["clock_offset_us"] == o["k"] and s.get("images", o["n"]) == o["n"]:
            return s
    C.fail(
        f"{o['paths']['log']}: no session of the holds file has clock offset {o['k']} us and "
        f"{o['n']} images"
    )


def on_axis(label, axis):
    """Whether a hold of the ring `label` counts for `axis`: the '0' ring and the rings turned
    about that axis alone."""
    if label == "0":
        return True
    if " " in label:
        return False
    return label[0] in ("LR" if axis == "yaw" else "UD")


def holds_of(o, entry, rings, poses):
    """One dict per hold: its ring, times and the means of every pose's angles (deg) and of the
    device's eye midpoint (T, mm) over the hold's images where every pose is valid."""
    t = (o["devts"] - o["devts"][0]) * 1e-6
    eye_T = (o["eye_mid_S"] - o["c"]) @ o["R_TS"].T
    valid = o["dll_valid"].copy()
    for p in poses.values():
        valid &= p["valid"]
    out = []
    for t0, t1, label in entry["holds"]:
        if label not in rings:
            C.fail(
                f"{entry['name']}: the hold at {t0} s names a ring laser_targets has not: {label}"
            )
        m = valid & (t >= t0) & (t < t1)
        e = eye_T[m]
        e = e[np.isfinite(e).all(1)]
        if m.sum() < 5 or len(e) == 0:
            why = (
                f"{int(m.sum())} images with every pose valid"
                if m.sum() < 5
                else "no gaze frame with both eyeball centres"
            )
            print(f"  {entry['name']}: hold {label} at {t0:.1f} s left out: {why}")
            continue
        h = dict(label=label, t0=t0, t1=t1, n=int(m.sum()), eye=e.mean(0))
        h["dll"] = o["dll_rot"][m].mean(0)
        for name, p in poses.items():
            h[name] = p["rot"][m].mean(0)
        out.append(h)
    return out


def truth(h, ring, screen_centre, beam):
    """The true (pitch, yaw) of hold h, deg: the beam from the eye midpoint moved by the beam's
    offset to the ring."""
    turn = C.compose_yxz_v(np.radians(h["dll"][None]))[0]
    origin = h["eye"] + turn @ np.array([0.0, beam["up"], beam["back"]])
    d = np.array([ring["x_mm"], ring["y_mm"], 0.0]) + np.asarray(screen_centre, float) - origin
    return np.array(
        [
            math.degrees(math.atan2(d[1], math.hypot(d[0], d[2]))),
            math.degrees(math.atan2(-d[0], -d[2])),
        ]
    )


def yaw_ring(label, rings):
    """The nominal yaw (deg, unsigned) of a ring turned in yaw alone; None for any other."""
    if " " in label or label[0] not in "LR":
        return None
    return abs(rings[label]["yaw_deg_nominal"])


def zero_neighbours(hs):
    """Per hold, the '0' holds next to it in time: the last to start before it and the first to
    start after it."""
    zeros = sorted((h for h in hs if h["label"] == "0"), key=lambda z: z["t0"])
    out = []
    for h in hs:
        before = [z for z in zeros if z["t0"] < h["t0"]][-1:]
        after = [z for z in zeros if z["t0"] > h["t0"]][:1]
        out.append(before + after)
    return out


def neck_lever(hs, rings):
    """The neck's lever, mm, and how many holds it rests on: over the yaw holds of the
    LEVER_RINGS_DEG rings, the device's eye midpoint moves sideways by -L (sin y - sin y0) from the
    neighbouring '0' holds' (y the true yaw, least squares through the origin); None without
    such holds."""
    num = den = 0.0
    n = 0
    for h, zs in zip(hs, zero_neighbours(hs)):
        if yaw_ring(h["label"], rings) not in LEVER_RINGS_DEG or not zs:
            continue
        x0 = np.mean([z["eye"][0] for z in zs])
        y0 = math.radians(np.mean([z["truth"][1] for z in zs]))
        ds = math.sin(math.radians(h["truth"][1])) - math.sin(y0)
        num -= (h["eye"][0] - x0) * ds
        den += ds * ds
        n += 1
    return (num / den, n) if den > 0 else (None, 0)


def neck_eyes(hs, rings, entry, lever):
    """Give the holds of the NECK_RINGS_DEG yaw rings the eye midpoint of a rigid neck turn and
    the truth that goes with it: from the neighbouring '0' holds' midpoint e0 at true yaw y0, the
    eyes turn about a vertical axis `lever` mm behind them, e0 + lever (sin y0 - sin y, 0,
    cos y0 - cos y), at the hold's own true yaw y (a fixed point)."""
    for h, zs in zip(hs, zero_neighbours(hs)):
        if yaw_ring(h["label"], rings) not in NECK_RINGS_DEG or not zs:
            continue
        e0 = np.mean([z["eye"] for z in zs], 0)
        y0 = math.radians(np.mean([z["truth"][1] for z in zs]))
        y = math.radians(h["truth"][1])
        h["eye_device"] = h["eye"]
        for _ in range(50):
            h["eye"] = e0 + lever * np.array(
                [math.sin(y0) - math.sin(y), 0.0, math.cos(y0) - math.cos(y)]
            )
            h["truth"] = truth(h, rings[h["label"]], entry["screen_centre_mm"], entry["beam_mm"])
            y, y_was = math.radians(h["truth"][1]), y
            if abs(y - y_was) < 1e-12:
                break
        else:
            C.fail(
                f"{entry['name']}: hold {h['label']} at {h['t0']:.1f} s: the neck model's true yaw "
                f"did not settle ({math.degrees(y - y_was):+.2g} deg in the last step)"
            )


def ols_slope(x, y):
    x = np.asarray(x, float) - np.mean(x)
    return float(x @ (np.asarray(y, float) - np.mean(y)) / (x @ x))


def slope_ci(x, y, seed=0):
    """The slope of y on x and its 95 % bootstrap interval over the points (the holds)."""
    x, y = np.asarray(x, float), np.asarray(y, float)
    rng = np.random.default_rng(seed)
    boot = []
    for _ in range(BOOT):
        i = rng.integers(0, len(x), len(x))
        if np.ptp(x[i]) > 1.0:
            boot.append(ols_slope(x[i], y[i]))
    return ols_slope(x, y), np.percentile(boot, [2.5, 97.5])


def pooled_slope(groups):
    """The slope of y on x over every group, each group its own intercept: groups = [(x, y)]."""
    xs = np.concatenate([np.asarray(x, float) - np.mean(x) for x, _ in groups])
    ys = np.concatenate([np.asarray(y, float) - np.mean(y) for _, y in groups])
    return float(xs @ ys / (xs @ xs))


def axis_points(holds, axis, pose):
    """(true angles, the pose's angles) of the holds that count for axis, deg."""
    k = dict(AXES)[axis]
    sel = [h for h in holds if on_axis(h["label"], axis)]
    return np.array([h["truth"][k] for h in sel]), np.array([h[pose][k] for h in sel])


def write_fitted(path, gains, report, sessions, core, levers):
    """Put the gains into a fitted.json, with the fingerprint again and their provenance; refused
    when its constants without their gains are not core, the ones the gains were fitted on."""
    with open(path) as f:
        doc = json.load(f)
    H = doc["head_params"]
    theirs = ref.fingerprint(dict(H, rotation_gain=list(ref.UNIT_GAIN)))
    ours = ref.fingerprint(ref.head_params(core))
    if theirs != ours:
        C.fail(
            f"{path}: its constants without their gains (fingerprint {theirs:016x}) are not the "
            f"ones the gains were fitted on ({ours:016x}): write them first (fit.py --fitted), or "
            "give that file's constants as --params"
        )
    H["rotation_gain"] = [float(gains["pitch"]), float(gains["yaw"]), 1.0]
    doc["fingerprint"] = f"{ref.fingerprint(H):016x}"
    doc["rotation_gain"] = dict(
        about="head_params.rotation_gain as tools/headpose/truth.py --fitted wrote it: per axis 1 "
        "/ the slope of the core pose (every gain 1) on laser-measured truth, the sessions pooled; "
        "z is 1 (the laser does not see roll). The eyes at the 20 and 25 deg yaw rings are those "
        "of a rigid neck turn, neck_lever_mm the lever per session. loso: each session's slope "
        "with the gains fitted on the others.",
        fitted=datetime.date.today().isoformat(),
        sessions=sessions,
        neck_lever_mm=levers,
        core_slope=report["core"],
        loso_slope=report["loso"],
    )
    with open(path, "w") as f:
        json.dump(doc, f, indent=1)
        f.write("\n")
    print(f"\nwrote {path}: rotation_gain {H['rotation_gain']}, fingerprint {doc['fingerprint']}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    C.add_session_args(ap)
    ap.add_argument("--holds", default=HOLDS, help="the holds and the setups (laser_holds.json)")
    ap.add_argument("--targets", default=TARGETS, help="the rings (laser_targets.json)")
    ap.add_argument(
        "--params",
        help="the constants of the shipped pose: a fit.py JSON or fitted.json (default: "
        "fitted.json here, HeadParams::FITTED)",
    )
    ap.add_argument("--fitted", help="write the gains fitted here into this fitted.json")
    ap.add_argument("--gates", action="store_true", help="check the holds file's gates")
    ap.add_argument("--holds-table", action="store_true", help="print every hold")
    args = ap.parse_args()
    with open(args.holds) as f:
        holds_doc = json.load(f)
    with open(args.targets) as f:
        rings = {r["label"]: r for r in json.load(f)["targets"]}
    shipped = C.load_params(args.params)[0] if args.params else dict(ref.FITTED)
    core = dict(shipped, rotation_gain=list(ref.UNIT_GAIN))
    gain_now = shipped.get("rotation_gain", ref.UNIT_GAIN)

    sessions = C.load_sessions(args)
    per = {}
    levers = {}
    for o in sessions:
        entry = entry_of(holds_doc, o)
        poses = {"core": C.run_reference(o, core), "shipped": C.run_reference(o, shipped)}
        hs = holds_of(o, entry, rings, poses)
        for h in hs:
            h["truth"] = truth(h, rings[h["label"]], entry["screen_centre_mm"], entry["beam_mm"])
        lever, n_lever = neck_lever(hs, rings)
        if lever is not None and not LEVER_BOUNDS_MM[0] <= lever <= LEVER_BOUNDS_MM[1]:
            print(
                f"  {entry['name']}: neck lever {lever:.1f} mm from {n_lever} holds, outside "
                f"{LEVER_BOUNDS_MM[0]:g}-{LEVER_BOUNDS_MM[1]:g} mm: not taken"
            )
            lever = None
        levers[entry["name"]] = None if lever is None else round(lever, 1)
        if lever is None:
            print(f"  {entry['name']}: no neck lever: the device's eyes at every ring")
        else:
            neck_eyes(hs, rings, entry, lever)
            print(f"  {entry['name']}: neck lever {lever:.1f} mm, from {n_lever} holds")
        per[entry["name"]] = (o, entry, hs)

    names = list(per)
    print(f"\nshipped rotation gains (x, y, z): {', '.join(f'{g:.4f}' for g in gain_now)}")
    print("slopes of the pose's angles on the true ones, 95 % interval over the holds")
    report = {"core": {}, "loso": {}, "shipped": {}}
    pooled = {}
    for axis, _ in AXES:
        print(f"\n{axis}:")
        print(f"  {'session':8} {'holds':>5} | {'DLL':^21} | {'core':^21} | {'shipped':^21}")
        for name in names:
            o, entry, hs = per[name]
            x, _ = axis_points(hs, axis, "dll")
            cells = []
            for pose in ("dll", "core", "shipped"):
                s, ci = slope_ci(x, axis_points(hs, axis, pose)[1])
                cells.append(f"{s:.3f} [{ci[0]:.3f}, {ci[1]:.3f}]")
                report.setdefault(pose, {}).setdefault(axis, {})[name] = round(s, 4)
            print(f"  {name:8} {len(x):5d} | " + " | ".join(cells))
        groups = {n: axis_points(per[n][2], axis, "core") for n in names}
        pooled[axis] = pooled_slope(list(groups.values()))
        report["core"][axis]["pooled"] = round(pooled[axis], 4)
        print(f"  core pooled over the sessions: {pooled[axis]:.4f} -> gain {1 / pooled[axis]:.4f}")
        for name in names:
            others = [groups[n] for n in names if n != name]
            if not others:
                continue
            g = 1.0 / pooled_slope(others)
            s = ols_slope(*groups[name]) * g
            report["loso"].setdefault(axis, {})[name] = round(s, 4)
            print(f"  leave {name} out: gain {g:.4f}, {name}'s slope with it {s:.3f}")
    gains = {axis: 1.0 / pooled[axis] for axis, _ in AXES}
    print(
        f"\ngains fitted on {', '.join(names)}: yaw {gains['yaw']:.4f}, pitch {gains['pitch']:.4f}"
    )

    if args.holds_table:
        print(
            f"\n{'session':8} {'ring':>8} {'t0 s':>6} {'n':>4} | true y/p | DLL y/p | shipped y/p"
        )
        for name in names:
            for h in per[name][2]:
                tr, d, s = h["truth"], h["dll"], h["shipped"]
                print(
                    f"{name:8} {h['label']:>8} {h['t0']:6.1f} {h['n']:4d} | {tr[1]:6.1f} "
                    f"{tr[0]:6.1f} | {d[1]:6.1f} {d[0]:6.1f} | {s[1]:6.1f} {s[0]:6.1f}"
                )

    if args.fitted:
        write_fitted(
            args.fitted,
            gains,
            report,
            {
                per[n][1]["name"]: dict(
                    log=os.path.basename(per[n][0]["paths"]["log"]),
                    clock_offset_us=int(per[n][0]["k"]),
                    images=int(per[n][0]["n"]),
                )
                for n in names
            },
            core,
            levers,
        )

    if args.gates:
        gates = holds_doc.get("gates", {})
        thresholds = gates.get("thresholds", {})
        failed = unchecked = 0
        print(f"\ngates: |shipped slope - 1| at most ({gates.get('rule', '')})")
        for name in names:
            th = thresholds.get(name)
            if th is None:
                print(f"  {name}: no thresholds")
                unchecked += 1
                continue
            for axis, _ in AXES:
                s = report["shipped"][axis][name]
                ok = abs(s - 1.0) <= th[axis]
                failed += not ok
                print(
                    f"  {name} {axis}: {s:.3f}, |s - 1| {abs(s - 1):.3f} vs {th[axis]}: "
                    f"{'pass' if ok else 'FAIL'}"
                )
        verdict = "fail" if failed else ("incomplete" if unchecked else "pass")
        print(f"truth gates: {verdict}")
        sys.exit(1 if failed else (3 if unchecked else 0))


if __name__ == "__main__":
    main()
