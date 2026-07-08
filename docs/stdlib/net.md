# Standard Library: Net

`stdlib.net` is the explicit source module for the current text TCP host
effects. Import it with:

```zt
net ::= import stdlib.net;
```

It is not ambient. The module factors the existing `net.listen`, `net.accept`,
`net.read`, `net.write`, and `net.close` host operations into reusable effect
aliases, thin helper functions, and one scoped connection helper.

## Types

The current network host boundary represents listener and connection handles as
`Int`. `net.write` writes to the runtime's current accepted connection, matching
the existing host-operation behavior.

`stdlib.net` exports effect aliases for common signatures:

| Name | Meaning |
| --- | --- |
| `Listen A` | `A ! { net.listen : Int -> Int; }` |
| `Accept A` | `A ! { net.accept : Int -> Int; }` |
| `Read A` | `A ! { net.read : Int -> Text; }` |
| `Write A` | `A ! { net.write : Text -> Unit; }` |
| `Close A` | `A ! { net.close : Int -> Unit; }` |
| `Connection A` | `A ! { net.accept; net.read; net.write; net.close; }` |
| `Server A` | `A ! { net.listen; net.accept; net.read; net.write; net.close; }` |

Closed operation packs such as `ConnectionEffects` and `ServerEffects` are
exported for composition with `* Pack` row spreads.

## API

| Name | Type | Notes |
| --- | --- | --- |
| `listen` | `Net -> Int -> Listen Int` | Binds a loopback TCP listener on the given port. |
| `accept` | `Net -> Int -> Accept Int` | Blocks until one client connects and returns a connection handle. |
| `read` | `Net -> Int -> Read Text` | Reads one UTF-8 line and strips trailing line ending characters. |
| `write` | `Net -> Text -> Write Unit` | Writes text to the current accepted connection and flushes. |
| `close` | `Net -> Int -> Close Unit` | Closes a connection handle. |
| `withConnection` | `Net -> Int -> (Int -> A ! { net.read; net.write; ...e; }) -> A ! { * ConnectionEffects; ...e; }` | Accepts one connection from a listener, passes the connection handle to the callback, and closes it in `finally` when the callback settles. |

This slice is intentionally small: no socket options, async/nonblocking IO,
binary bytes, address selection, or higher-level protocol helpers are part of
the current module. Listener lifetime stays explicit; `withConnection` scopes
only one accepted connection.

## Examples

Full runnable examples live in `examples/`:

- `examples/net_echo.zt` accepts one TCP connection on port 7777 with
  `withConnection` and echoes one line.
- `examples/echo_http.zt` accepts HTTP requests on port 8080 and echoes the
  request line in a small response, scoping each accepted connection.

```zt
net ::= import stdlib.net;

serveOnce :: Net -> Int -> net.Server Text
  = cap port => [
    listener := net.listen cap port;
    net.withConnection cap listener (\conn. [
      line := net.read cap conn;
      net.write cap line;
      line
    ])
  ];

main :: Net -> net.Server Text
  = cap => serveOnce cap 7777;

main
```
