# Location compatibility fixtures

The Rust location functions must answer exactly like the Python app they
replace. These files record the Python app's answers for synthetic inputs:
local paths, invalid input, devices, SMB, labels, breadcrumbs and file sizes.
No personal file system is read.

| File | Purpose |
| --- | --- |
| `inputs.py` | The inputs for `v2.0.0:desktop/core.py`. |
| `generate_python.py` | Runs `v2.0.0:desktop/core.py` on them and prints `python.json`. |
| `generate_javascript.cjs` | Runs the display helpers of `v2.0.0:desktop/ui/app.js` on its own inputs and prints `javascript.json`. |
| `python.json`, `javascript.json` | The captured answers. |
| `support.rs` | Rust helpers shared by the `location_*` tests. |

`location_python.rs`, `location_external.rs` and `location_javascript.rs`
compare the Rust port with the captured answers; they embed the JSON and need
neither Python nor Node. `location_fixture_drift.rs` runs both generators
against the Python app's sources (`desktop/` of tag `v2.0.0`, which
`native/tools/check.py` extracts and names in `OX_PYTHON_APP`) and fails when
an answer changed, so the fixtures cannot drift from them. It needs `python3`
and `node`. If `node` is a
version-manager shim, which may not work with the disposable home folder of
`native/tools/check.py`, set `OX_NODE` to the executable it runs
(`OX_NODE=$(node -p process.execPath)`).

## Regenerating

The Python app no longer changes, so only a change to the inputs needs new
answers. Run from the repository root:

```sh
desktop=$(python3 native/tools/python_app.py)
python3 native/crates/ox-core/tests/location_fixtures/generate_python.py "$desktop" \
  > native/crates/ox-core/tests/location_fixtures/python.json
node native/crates/ox-core/tests/location_fixtures/generate_javascript.cjs "$desktop/ui/app.js" \
  > native/crates/ox-core/tests/location_fixtures/javascript.json
```

Review the changed answers in the diff, then make the Rust port pass again.

## How the answers are captured

`generate_python.py` records each answer as a value, as an `error` (a message
that a `raise` in `core.py` wrote, which the port must repeat word for word) or
as `rejected` (Python's standard library refused the input in its own words;
the port must refuse it too). It does not write `__pycache__` into the Python
app's sources.

`generate_javascript.cjs` cannot run `app.js` itself, which needs a browser
document. It finds the helpers by name, together with every `app.js` function
they mention, and runs them in a separate V8 context. The helpers catch their
own errors, so a helper that read something the script does not provide
(`document`, a function it failed to extract) would record a wrong fallback
answer. The context records every such global, and the script fails if there
is one.

## Known differences

- `parentUri` below a device root keeps a trailing slash
  (`mtp://[usb:001,010]/DCIM/`). The Rust port returns the canonical form
  without it, and `location_javascript.rs` compares canonical forms.
