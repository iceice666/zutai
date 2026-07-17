# Zutai Examples

These examples are meant to be read top to bottom. From this workspace, run the
CLI through Cargo:

```sh
cargo run -q -p zutai-cli -- run examples/service_health.zt
```

If `zutai-cli` is installed on your PATH, use the shorter form shown in each
example file:

```sh
zutai-cli run examples/service_health.zt
```

## Quick Workflow

```sh
just examples-check      # type-check every .zt example, including network demos
just examples-run        # run only examples that terminate on their own
just native-examples     # check/run/native-compile parity for larger examples
```

Without `just`, use Cargo directly:

```sh
cargo run -q -p zutai-cli -- check examples/service_health.zt
cargo run -q -p zutai-cli -- run examples/service_health.zt
```

## Guide

| Example | Shows | What to expect |
| --- | --- | --- |
| `service_health.zt` + `service_health.zti` | Importing inert data, typed records, validation, rollups | Prints an operational summary record |
| `canary_forecast.zt` | Infinite synthetic stream bounded by `takeList` | Prints a canary-risk report |
| `deploy_readiness.zt` + `deploy_readiness.zti` | Rollout gating from inert config, failed-check rollups, and text summaries | Prints a deployment readiness record |
| `stdlib_pipeline.zt` | `stdlib.num`, `stdlib.optional`, and `stdlib.result` pipelines | Prints one integer score |
| `stream_summary.zt` | List-to-stream transformation and lazy stream summary | Prints items, sum, and count |
| `host_stream_read.zt` | `stream { yield perform fs.read ... }` in a lazy cell at the host boundary | Reads its own source, proves the missing tail is not forced, and matches `run`/native output |
| `stdlib_fs_lines.zt` | `stdlib.fs` `withWriter`/`withReader` bracket helpers | Writes two lines, reads them back, and confirms EOF as `#none` |
| `stdlib_fs_manual.zt` | Manual `openWrite`/`writeText`/`flush`/`closeWrite` and `openRead`/`readLine`/`closeRead` | Shows explicit handle lifetime and idempotent double close |
| `stdlib_fs_whole_file.zt` | `stdlib.fs` `writeAll`/`readAll` compatibility wrappers | Writes and reads a whole text file through the old `fs.write`/`fs.read` host boundary |
| `text_report.zt` | `stdlib.text` normalization, replacement, joining, and parsing | Prints a text report record |
| `from_data_runtime.zt` + `from_data_runtime.zti` | Hygienic staging-backed `FromData` derivation at `loadZti` | Prints a typed nested config validation result and ignores an extra input field |
| `stdlib_ergonomics.zt` + `stdlib_ergonomics.zti` | Records, tagged unions, streams, nested `FromData` derive, and an explicit `Load` capability in one flow | Prints a typed health report; `run` and native output match |
| `host_capabilities.zt` + `host_capabilities_mock.zt` | `stdlib.env`, `stdlib.clock`, `stdlib.rng`, and `stdlib.load` composed through one explicit capability record, plus source-handler mocks for every wrapper | Host-backed stable fields and shapes agree between `run` and native output; the mock fixture prints deterministic intercepted values |
| `stdlib_browser.zt` | Small typed `stdlib.html` / `stdlib.css` / `stdlib.browser` application | `check` passes; use `zutai-web build` for browser execution |
| `net_echo.zt` | `stdlib.net` `withConnection` for one TCP request | Waits on port 7777 until a client sends a line |
| `echo_http.zt` | Recursive `stdlib.net` `withConnection` for a tiny HTTP responder | Waits on port 8080 and keeps accepting clients |

The filesystem examples should be run from the workspace root because host
`Path` values are ordinary process-relative paths, not source-relative imports.
`host_stream_read.zt` wraps a stream in a source handler that would return
`"handler-mock"` if it owned the lazy cell, then forces one cell and reads this
source file through the host boundary instead. A second cell points at a missing
file; the successful output is the laziness check. The `stdlib_fs_*.zt`
examples create ignored `examples/*.out` files.

`stdlib_browser.zt` is intentionally check/build-oriented: a browser program
returns callable `init`/`update`/`view` fields and is executed by `zutai-web`, not
rendered as a terminal value by `zutai-cli run`.

## Network Demos

The network examples use explicit `stdlib.net` helpers and type-check like the
others, but `run` waits for a local client.

For `net_echo.zt`, start the server in one terminal:

```sh
cargo run -q -p zutai-cli -- run examples/net_echo.zt
```

Then send a line from another terminal:

```sh
printf 'hello from zutai\n' | nc 127.0.0.1 7777
```

For `echo_http.zt`, start the server in one terminal:

```sh
cargo run -q -p zutai-cli -- run examples/echo_http.zt
```

Then request it from another terminal:

```sh
curl http://127.0.0.1:8080/
```

## Browser Demo

Start a local page backed by the compiled deployment-readiness example:

```sh
just browser-demo
```

The recipe compiles `examples/deploy_readiness.zt` with `--emit=lib`, builds the
standalone Rust host in `examples/browser_demo/host.rs`, and serves
`http://127.0.0.1:8787/` plus `http://127.0.0.1:8787/api/deploy-readiness`.
Pass a different bind address as the first recipe argument:

```sh
just browser-demo 127.0.0.1:9000
```

## Generated Artifacts

Native compilation and filesystem examples may leave generated files such as
`examples/*.ll`, `examples/*.o`, `examples/*.out`, `examples/net_echo`, or
`examples/echo_http`. They are ignored by git and can be deleted when you no
longer need them.
