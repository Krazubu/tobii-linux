#!/usr/bin/env python3
"""Count the connected tracker's serial number in the files under the given
paths, in every encoding a binary, a capture or a dump could carry it in,
without ever printing it.

Exits 1 if it is found anywhere. With no tracker connected (in CI, say) there
is nothing to look for: it says so and exits 0.

    python3 -I packaging/scan-serial.py PATH...
"""

import sys
from pathlib import Path

VENDOR, PRODUCT = "2104", "0313"


def tracker_serial():
    try:
        devices = sorted(Path("/sys/bus/usb/devices").iterdir())
    except OSError:  # no USB at all, as on a CI runner
        return None
    for dev in devices:
        try:
            if (dev / "idVendor").read_text().strip() == VENDOR and (
                dev / "idProduct"
            ).read_text().strip() == PRODUCT:
                return (dev / "serial").read_text().strip()
        except OSError:
            continue
    return None


def spaced(hex_digits):
    return " ".join(hex_digits[i : i + 2] for i in range(0, len(hex_digits), 2))


def encodings(serial):
    """Name -> the byte strings the serial takes in that encoding."""
    forms = {}
    for name, raw in (("ASCII", serial.encode()), ("UTF-16LE", serial.encode("utf-16-le"))):
        digits = raw.hex()
        forms[name] = [raw]
        forms[name + " as hex"] = [
            text.encode() for text in (digits, digits.upper(), spaced(digits), spaced(digits.upper()))
        ]
    return forms


def files_under(paths):
    for root in map(Path, paths):
        candidates = [root] if root.is_file() else sorted(root.rglob("*"))
        yield from (f for f in candidates if f.is_file() and not f.is_symlink())


def main(paths):
    serial = tracker_serial()
    if not serial:
        print("scan-serial: no tracker connected, nothing to look for")
        return 0
    forms = encodings(serial)
    scanned = found = 0
    for f in files_under(paths):
        data = f.read_bytes()
        scanned += 1
        for name, needles in forms.items():
            hits = sum(data.count(needle) for needle in needles)
            if hits:
                found += hits
                print(f"scan-serial: {f}: the serial, {hits} times in {name}")
    print(f"scan-serial: {scanned} files, the serial found {found} times")
    return 1 if found else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:]))
