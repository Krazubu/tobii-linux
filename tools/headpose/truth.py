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
+). The pointer may sit askew on the head, so only changes of angle count: the slopes, per axis
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
            C.fail(f"{o['name']}: the hold at {t0} s names a ring laser_targets has not: {label}")
        m = valid & (t >= t0) & (t < t1)
        e = eye_T[m]
        e = e[np.isfinite(e).all(1)]
        if m.sum() < 5 or len(e) == 0:
            print(f"  {o['name']}: hold {label} at {t0:.1f} s: too few valid images, left out")
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


def write_fitted(path, gains, report, sessions):
    """Put the gains into a fitted.json, with the fingerprint again and their provenance."""
    with open(path) as f:
        doc = json.load(f)
    H = doc["head_params"]
    H["rotation_gain"] = [float(gains["pitch"]), float(gains["yaw"]), 1.0]
    doc["fingerprint"] = f"{ref.fingerprint(H):016x}"
    doc["rotation_gain"] = dict(
        about="head_params.rotation_gain as tools/headpose/truth.py --fitted wrote it: per axis 1 "
        "/ the slope of the core pose (every gain 1) on laser-measured truth, the sessions pooled; "
        "z is 1 (the laser does not see roll). loso: each session's slope with the gains fitted on "
        "the others.",
        fitted=datetime.date.today().isoformat(),
        sessions=sessions,
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
    for o in sessions:
        entry = entry_of(holds_doc, o)
        poses = {"core": C.run_reference(o, core), "shipped": C.run_reference(o, shipped)}
        hs = holds_of(o, entry, rings, poses)
        for h in hs:
            h["truth"] = truth(h, rings[h["label"]], entry["screen_centre_mm"], entry["beam_mm"])
        per[entry["name"]] = (o, entry, hs)

    names = list(per)
    print(f"\nshipped rotation gains (x, y, z): {', '.join(f'{g:.4f}' for g in gain_now)}")
    print("slopes of the pose's angles on the true ones, 95 % interval over the holds")
    report = {"core": {}, "loso": {}, "shipped": {}}
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
        pooled = pooled_slope(list(groups.values()))
        report["core"][axis]["pooled"] = round(pooled, 4)
        print(f"  core pooled over the sessions: {pooled:.4f} -> gain {1 / pooled:.4f}")
        for name in names:
            others = [groups[n] for n in names if n != name]
            if not others:
                continue
            g = 1.0 / pooled_slope(others)
            s = ols_slope(*groups[name]) * g
            report["loso"].setdefault(axis, {})[name] = round(s, 4)
            print(f"  leave {name} out: gain {g:.4f}, {name}'s slope with it {s:.3f}")
    gains = {axis: 1.0 / report["core"][axis]["pooled"] for axis, _ in AXES}
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
