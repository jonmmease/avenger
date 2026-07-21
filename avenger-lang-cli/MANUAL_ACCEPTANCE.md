# Native `watch` acceptance

Run this checklist before claiming release support for a desktop platform. It
requires a real interactive desktop session; headless unit tests and a
successful build do not satisfy the window, resize, or interaction rows.

Record the date, OS version, winit backend, GPU/driver, editor and save mode,
commit, command, and result. Test every row on macOS and on at least one of
Linux or Windows. Run `cargo check` and the non-headful CLI suite on the third
platform.

## Setup

Build and test from the workspace:

```sh
cargo build --release -p avenger-lang-cli
cargo test --release -p avenger-lang-cli -- --nocapture
```

Copy `avenger-lang-cli/tests/fixtures/watch_project` to a disposable directory;
do not edit the checked-in fixture. Launch it with cache reporting:

```sh
target/release/avenger watch /absolute/path/to/watch_project/chart.avenger \
  --log-cache
```

On Windows, use `target\release\avenger.exe` and an absolute Windows path.
Linux testing requires a working Wayland or X11 session and a WGPU-supported
adapter. Keep stdout and stderr for the acceptance record.

## Checklist

- [ ] The initial chart appears and remains interactive in one window.
- [ ] Drag the virtual canvas frame's corner or edge handle before and after a
  reload; content and hit testing follow the new size, and the native window
  itself is not recreated. Resizing only the native window changes surface
  space, not the chart's authored virtual canvas.
- [ ] Save the root in place, then through the editor's atomic rename mode.
  Each produces one successful latest-generation reload.
- [ ] Save three or more times inside the debounce interval. The final content
  appears without stale intermediate installation or an event backlog.
- [ ] Edit `marks/badge.mark.avenger` and `data/rows.csv`; both reload.
- [ ] Add `theme css from 'theme.css';` inside the chart, create the CSS file,
  then edit it. The external theme is reported as a dependency and reloads.
- [ ] Add an import whose file is missing. One diagnostic batch appears on
  stdout, creating the file repairs without restart, removing the import drops
  the file from the active watch closure, and editing the removed file no
  longer reloads.
- [ ] Introduce a syntax error and repair it. The last-good chart stays
  interactive, the title shows and clears its error state, and each failed
  generation prints exactly one diagnostic batch.
- [ ] Run
  `avenger-lang-cli/tests/fixtures/interactive_state/chart.avenger`. Drag
  through blank plot space to create the red store-backed marker, then click a
  blue point to grow the amber param-backed indicator. Make a harmless source
  edit: both must survive reload. Click the same blue point again: the amber
  indicator must shrink, proving the migrated selection toggled off. Repeat
  with an incompatible state-type/contract edit and verify only that state
  resets.
- [ ] Begin a drag, reload before releasing it, and verify the documented
  gesture/cursor reset without a crash or stuck cursor.
- [ ] Trigger a reload and immediately close the window. Repeat during a known
  slow compile. The process exits without deadlock, panic, or a five-second
  worker-shutdown timeout.
- [ ] Close an idle window and terminate another run with `Ctrl-C`; both exit
  successfully and leave no `avenger` process.
- [ ] With cache enabled, a style-only edit reports eligible hits and a data
  edit reports invalidation. Repeat the visual result with `--no-cache` and
  with `AVENGER_PHYSICAL_CACHE=0`; both disabled modes report no cache use.

## Result record

```text
Date/commit:
OS/backend:
GPU/driver:
Editor/save mode:
Commands:
Rows passed:
Rows failed (with logs/reproduction):
Tester:
```

Do not check a platform based solely on CI, fake-host tests, or another
platform's results. Attach failures to the CLI hardening tracker before release.
