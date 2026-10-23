# Contributing

Thanks for looking. This is a reverse-engineered driver for one piece of
hardware, which shapes what help is useful and how a change gets judged.

## What this project is bound by

`libtobii.so` presents **exactly the 153 entry points of Tobii's Stream Engine
4.1.0.3**, so that a program built for the Windows SDK links and runs against
it. That contract is not a style preference: `make verify-abi` fails if a
symbol appears, disappears or stops being declared in the headers, and CI runs
it on every pull request.

Where an answer cannot be the Stream Engine's, it diverges **deliberately and
in writing**. The README's "where they differ from Windows" lists every such
place, `tools/abi/README.md` carries the derivation with the DLL addresses it
came from, and a change that adds a divergence is expected to add its entry.
"It seemed more sensible" is a reason; it just has to be written down.

## Before you open a pull request

Run the same gates CI runs:

```sh
make check        # rustfmt, clippy -D warnings, the whole test suite, the docs
make verify-abi   # the 153 exports, and a C program compiled against the headers
```

Neither needs a tracker. Three tests whose inputs are captures of a face are
gated behind environment variables and skip without them.

A few things the reviewer will look for:

- **A test that fails without the change.** For a bug fix, the test that
  reproduces it; for a behaviour change, the test that pins the new answer.
  The suite is large (1124 tests) because that is how this codebase is kept
  honest about a device nobody can mock.
- **No `unwrap()` or `expect()` on a path that can fail at runtime.** `expect`
  is for invariants whose violation is a bug; everything else returns an
  error with context.
- **`// SAFETY:` on every `unsafe` block**, saying what makes it sound — most
  of the FFI surface is `unsafe` by necessity and the comments are what keep
  it reviewable.
- **A claim about the Stream Engine carries its evidence.** If you write that
  the DLL answers something, cite the address you read it at (image base
  `0x180000000`) and say whether you read it or inferred it. The existing docs
  do this throughout; it is the only reason they can be checked years later.

Commit messages are prose, not a change-type prefix: say what changed and
*why*, and name what you decided against. The log is the project's design
record.

## Changes that need hardware

Anything touching `tobii-usb`, the daemon's engine or a tracker command can
only be judged on a real Eye Tracker 5. If you have one, say in the pull
request what you ran and what the tracker answered, and attach the relevant
journal lines. If you do not, say that too — a well-reasoned patch with "not
tested on hardware" is welcome and will simply wait for someone who can run
it.

**Never add a probe that writes to the tracker without a dry run first.**
`tobii5-init-replay state-probe` and `stream-probe` exist for this: a closed
whitelist of commands, `--dry-run` by default, an explicit flag for writes, and
every reply logged. Extend them rather than writing something ad hoc.

## What is especially welcome

- **Another tracker.** Only the Eye Tracker 5 (`2104:0313`) is supported and
  verified. A 5L, 4C or anything else would need its stream catalogue
  (command 1200), its generation string (1420) and its property list (1330)
  captured — that alone, even without code, is a real contribution. See
  `tools/abi/README.md` for how those are read.
- **Distribution packaging.** `make package` builds a `.deb`, `.rpm` and
  tarball; a COPR or AUR recipe that tracks releases would help more people
  than most code changes.
- **Anything in the README's "Limitations".** Those are known gaps, not
  oversights, and each says what is missing.

## Research material

`fixtures/` is gitignored and holds the captures, logs and analysis the
driver was built from. It is not in the repository (some of it carries the
tracker's serial number, and all of it is large). The write-ups reference
paths inside it; if you need one to follow an argument, ask.

## Reporting a problem

See `.github/ISSUE_TEMPLATE/`. The short version: a driver bug without the
daemon's journal, your tracker's firmware and the version you are running is
usually not reproducible, and the template asks for exactly those.

## Licence

MIT, as the repository. By contributing you agree your work is published under
it.
