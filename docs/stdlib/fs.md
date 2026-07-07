# Standard Library: FS

`stdlib.fs` is the explicit source module for text filesystem effects. Import it
with:

```zt
fs ::= import stdlib.fs;
```

It is not ambient. Existing compiler-backed `fs.read`, `fs.write`, `load.zti`,
`load.zt`, and `io.print` behavior remains available exactly as before.

## Types

- `Reader` and `Writer` are opaque host support types. Source can pass them to
  the functions that produced them, but cannot inspect or construct them.
- `Path` is the existing text-shaped host path support type.
- `WriteTextRequest = { contents : Text; writer : Writer; }`
- `WriteAllRequest = { contents : Text; path : Path; }`

Opaque handles are not renderable program outputs and are rejected by the CLI
render/JSON gates and native entry gate.

## API

| Name | Type | Notes |
| --- | --- | --- |
| `openRead` | `FsRead -> Path -> Reader` | Opens a text reader; requires `fs.openRead`. |
| `readLine` | `FsRead -> Reader -> Text?` | Reads one UTF-8 line; strips one trailing `\n` and one optional preceding `\r`; EOF is `#none`. |
| `closeRead` | `FsRead -> Reader -> Unit` | Idempotent for known handles. |
| `openWrite` | `FsWrite -> Path -> Writer` | Creates or truncates a text writer. |
| `writeText` | `FsWrite -> Writer -> Text -> Unit` | Writes bytes exactly; no implicit newline. |
| `flush` | `FsWrite -> Writer -> Unit` | Explicit flush. |
| `closeWrite` | `FsWrite -> Writer -> Unit` | Flushes before close; idempotent for known handles. |
| `withReader` | `FsRead -> Path -> (Reader -> A ! {...}) -> A` | Bracket helper; closes in `finally` when the callback settles. |
| `withWriter` | `FsWrite -> Path -> (Writer -> A ! {...}) -> A` | Bracket helper; closes in `finally` when the callback settles. |
| `readAll` | `FsRead -> Path -> Text` | Compatibility wrapper over existing `fs.read`. |
| `writeAll` | `FsWrite -> Path -> Text -> Unit` | Compatibility wrapper over existing `fs.write`. |

The first slice is synchronous and text-only. Append, seek, binary bytes, async,
and nonblocking IO remain out of scope.

## Examples

Full runnable examples live in `examples/`:

- `examples/stdlib_fs_lines.zt` writes two lines with `withWriter`, reads them
  back with `withReader`, and confirms EOF is `#none`.
- `examples/stdlib_fs_manual.zt` shows explicit open/write/flush/close and
  open/read/close calls, including idempotent double close.
- `examples/stdlib_fs_whole_file.zt` shows `writeAll`/`readAll` as compatibility
  wrappers over the older whole-file host effects.

Bracketed line IO is the preferred shape when a handle should never escape its
scope:

```zt
fs ::= import stdlib.fs;
WriteTextRequest :: type { contents : Text; writer : Writer; };

path :: Path = "examples/stdlib_fs_lines.out";

main :: { read : FsRead; write : FsWrite; } -> Text ! { fs.openWrite : Path -> Writer; fs.writeText : WriteTextRequest -> Unit; fs.flush : Writer -> Unit; fs.closeWrite : Writer -> Unit; fs.openRead : Path -> Reader; fs.readLine : Reader -> Text?; fs.closeRead : Reader -> Unit; }
  = caps => [
    fs.withWriter caps.write path (\writer. [
      fs.writeText caps.write writer "alpha\n";
      fs.writeText caps.write writer "beta\n"
    ]);
    fs.withReader caps.read path (\reader.
      match fs.readLine caps.read reader {
        | #some (line) => line;
        | #none => "<empty>";
      }
    )
  ];

main
```

Manual handles are useful when the lifetime is still local but the code wants
explicit flush or close points:

```zt
fs ::= import stdlib.fs;
WriteTextRequest :: type { contents : Text; writer : Writer; };

path :: Path = "examples/stdlib_fs_manual.out";

main :: { read : FsRead; write : FsWrite; } -> Text? ! { fs.openWrite : Path -> Writer; fs.writeText : WriteTextRequest -> Unit; fs.flush : Writer -> Unit; fs.closeWrite : Writer -> Unit; fs.openRead : Path -> Reader; fs.readLine : Reader -> Text?; fs.closeRead : Reader -> Unit; }
  = caps => [
    writer := fs.openWrite caps.write path;
    fs.writeText caps.write writer "manual\n";
    fs.flush caps.write writer;
    fs.closeWrite caps.write writer;

    reader := fs.openRead caps.read path;
    line := fs.readLine caps.read reader;
    fs.closeRead caps.read reader;
    line
  ];

main
```

For whole-file compatibility, use `readAll` and `writeAll`; these preserve the
existing `fs.read`/`fs.write` behavior behind explicit `stdlib.fs` names:

```zt
fs ::= import stdlib.fs;
WriteAllRequest :: type { contents : Text; path : Path; };

path :: Path = "examples/stdlib_fs_whole_file.out";

main :: { read : FsRead; write : FsWrite; } -> Bool ! { fs.write : WriteAllRequest -> Unit; fs.read : Path -> Text; }
  = caps => [
    fs.writeAll caps.write path "contents\n";
    fs.readAll caps.read path == "contents\n"
  ];

main
```
