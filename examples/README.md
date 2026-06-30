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
just native-examples     # check/run/native-compile parity for the larger pure demos
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
| `text_report.zt` | `stdlib.text` normalization, replacement, joining, and parsing | Prints a text report record |
| `net_echo.zt` | Host network effects for one TCP request | Waits on port 7777 until a client sends a line |
| `echo_http.zt` | Recursive host network effects for a tiny HTTP responder | Waits on port 8080 and keeps accepting clients |

## Network Demos

The network examples type-check like the others, but `run` waits for a local
client.

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

## Generated Artifacts

Native compilation may leave generated files such as `examples/*.ll`,
`examples/*.o`, `examples/net_echo`, or `examples/echo_http`. They are ignored by
git and can be deleted when you no longer need them.
