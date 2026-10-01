# Parity inventories and the regression process

The native Rust + GTK4 app replaces the Python/WebKit app of OpenXplorer 1.x
only when it can do everything the Python app does. This directory records what
that means and checks it.

The Python app has left the tree. Its last release is tag `v1.1.4`; its final
sources, 1.1.4 with fixes that were never released, are `desktop/` at tag
`v2.0.0`. Features cite those sources as `v2.0.0:desktop/<path>`, which
`git show` reads directly. `legacy.json` records what the checks need from
them: the bridge's operations and the names of its tests.

## The rules

The product owner set four rules. The inventory exists to enforce the first two.

1. **Only gain functionality.** The native app may never lose a behaviour the
   Python app has. Every such behaviour is listed and must be proven by a test.
2. **Dolphin is the minimum.** KDE Dolphin sets the minimum for functionality,
   usability and desktop integration. On Zorin/GNOME, integration means GNOME's
   own mechanisms (GIO, GVfs, the FileManager1 D-Bus interface, libsecret, the
   freedesktop Trash and thumbnail specifications).
3. **Keep the look.** The Windows 11 File Explorer look stays, refined toward
   the Windows 11 / WinUI 3 specification.
4. **Clean code.** Code must be clean, readable and idiomatic.

## Files

| File | Purpose |
| --- | --- |
| `features.toml` | Every behaviour the native app must provide, one `[[feature]]` per behaviour. |
| `bridge.json` | Every operation of the Python bridge in `v2.0.0:desktop/winspace.py`, with its native status. |
| `legacy.json` | The Python app's bridge operations and tests, read from tag `v2.0.0` when the app was removed. It never changes. |
| `check.py` | Validates both inventories, the parity markers and the cited tests, and applies the gates. |
| `bridge.py` | The checks for `bridge.json`. |
| `features.py`, `legacy.py`, `markers.py` | The checks for features, Python test citations and parity markers. |
| `test_check.py` | Tests for all of the above. |

`features.toml` merges six inventories: the current app's UI surfaces, its
interaction details, its Python backend and its documentation, the Dolphin
baseline, and GNOME desktop integration. Duplicates were merged, so each
behaviour appears once with all its sources and tests.

## Feature fields

The header of `features.toml` documents each key. In short:

- `id`: `AREA-NNN`, for example `NAV-001`. The prefix is the `area`.
- `title` and `behaviour`: what the user can observe, precise enough to test.
- `origin`: who requires it. `openxplorer` means the current app has it or its
  documentation promises it; `dolphin` means the Dolphin baseline; `gnome`
  means native GNOME integration. A feature can have several origins.
- `priority`: `must`, `should` or `could`. Everything the current app has is
  `must`.
- `openxplorer`: what the Python app does today: `has`, `partial` or `missing`.
- `sources`, `python_tests`, `bridge`: where the behaviour is defined, which
  tests in `v2.0.0:desktop/tests` exercise it, and which bridge operations it uses.
- `dolphin`, `gnome`: optional references to the Dolphin action and the GNOME
  mechanism.
- `native`: the native status, see below.
- `native_note`: evidence for `partial`, the reason for `n-a`, and notes for
  the port, such as bugs to fix rather than preserve.

## Native status

| Status | Meaning |
| --- | --- |
| `todo` | Not implemented, or implemented without a test. |
| `partial` | Part of the behaviour is implemented and tested. `native_note` names the tests and what is still missing. |
| `done` | The whole behaviour is implemented, and a native test that proves it carries a parity marker. |
| `n-a` | The behaviour cannot apply to the native app, for example because it only exists in the WebKit interface. `native_note` must say why and which feature covers the user-visible rule instead. |

`n-a` is a product decision, not a way to skip work. Use it only when the
behaviour truly has no native counterpart.

## How to mark a feature done

1. Implement the whole behaviour described in `behaviour`. Read the
   `sources` and the cited `python_tests`: they are the specification.
2. Write a native test that proves it: a Rust unit or integration test,
   or, once the first one exists, a UI test under `native/ui-tests/`. The
   test must run in `python3 native/tools/check.py`.
3. Put a parity marker in the test's doc comment, naming every feature it
   proves:

   ```rust
   /// parity: NAV-001, NAV-005
   #[test]
   fn back_and_forward_walk_the_tab_history() {
   ```

4. Set `native = "done"` in `features.toml` and update `native_note`.
5. Run the checks (below).

If the test covers only part of the behaviour, keep the marker and set
`native = "partial"`, saying in `native_note` what is still missing. The check
rejects a marker on a feature that is still `todo` or `n-a`, a marker naming an
unknown feature, and `done` without a marker.

## Changing the inventory

- **Ids are permanent.** Never renumber, reuse or delete an id: tests, commits
  and reviews refer to them. A behaviour that stops applying keeps its id and
  becomes `n-a` with a note. New behaviours get the next free number in their
  area.
- Add a feature when you find a behaviour that is missing.
- Keep keys in the documented order. Every cited test and bridge operation
  must exist, and every bridge operation must be cited by some feature.

## Bridge operations

`bridge.json` lists every operation that `dispatch` in `v2.0.0:desktop/winspace.py`
handled, exactly once: the operations `legacy.json` records. They were read
from the dispatcher's source by a reader that failed closed on any use of the
operation name it could not read (see `native/parity/dispatch.py` at tag
`v2.0.0`). An operation missing from `bridge.json`, or one that the Python app
never had, fails the check.

| Status | Meaning |
| --- | --- |
| `pending` | Not ported, or ported without tests. `note` says what is missing. |
| `core-tested` | The `ox-core` logic is ported and tested. The native UI is not yet proven, so this does not unblock replacement. |
| `native-tested` | The whole workflow is proven in the native app. |

A tested status must cite at least one test in `evidence`, as
`path/to/file.rs::test_name` relative to the repository root. The check
confirms that the file defines `fn test_name` under a `#[test]` attribute,
or `#[gtk::test]` for the window tests (comments and other attributes may
sit between them), and that it is not
`#[ignore]`d. Cite the file that defines the function, not the file that
includes it: the tests that `ox-core/tests/transfer.rs` pulls in through
`#[path]` modules are defined in `ox-core/tests/transfer_cases/`.

## Gates

| Command | Passes when |
| --- | --- |
| `python3 native/parity/check.py` | Both inventories and all markers are valid. Always run this. |
| `--gate replace` | Every feature with origin `openxplorer` that the Python app has (`has` or `partial`) is `done` or `n-a`. This is the replacement gate for rule 1. |
| `--gate dolphin` | Every `must` feature with origin `dolphin` is `done` or `n-a`. This is the Dolphin baseline for rule 2. |
| `--require-replacement` | Every `bridge.json` operation is `native-tested`. |

The native app may replace the Python app only when `--gate replace`,
`--gate dolphin` and `--require-replacement` all pass, together with the manual
acceptance checks in [ROADMAP.md](../ROADMAP.md) (real SMB servers, phones,
Wayland drag and drop, assistive technology). Local tests cannot certify those.

## Running the checks

The checks need Python 3.12 or later and nothing outside the standard
library. From the repository root:

```sh
python3 native/parity/check.py                   # validate, print status per area
python3 native/parity/check.py --python-tests    # also list Python tests no feature cites
python3 native/parity/check.py --gate replace --gate dolphin
python3 -m unittest discover -s native/parity -p 'test_*.py'
```

`python3 native/tools/check.py` runs the validation and these unit tests along
with the native build and test suite.

`--python-tests` lists tests of `v2.0.0:desktop/tests` that no feature cites. Every
test there that checks a user-visible behaviour should be cited by the feature
it protects; the remaining entries are harness checks such as "No JavaScript
exceptions".
