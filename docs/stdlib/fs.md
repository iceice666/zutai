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
- Request record shapes for `fs.writeText` and `fs.write` are private to the
  module because they contain opaque host handles.

Opaque handles are not renderable program outputs and are rejected by the CLI
render/JSON gates and native entry gate.

`stdlib.fs` also exports effect aliases for common signatures:

| Name | Meaning |
| --- | --- |
| `ReadLine A` | `A ! { fs.readLine : Reader -> Text?; }` |
| `WriteText A` | `A ! { fs.writeText : WriteTextRequest -> Unit; }` |
| `ScopedRead A` | `A ! { fs.openRead; fs.readLine; fs.closeRead; }` |
| `ScopedWrite A` | `A ! { fs.openWrite; fs.writeText; fs.flush; fs.closeWrite; }` |
| `ScopedReadWrite A` | Combined scoped read/write row. |
| `WholeRead A` | `A ! { fs.read : Path -> Text; }` |
| `WholeWrite A` | `A ! { fs.write : WriteAllRequest -> Unit; }` |
| `WholeFile A` | Combined whole-file read/write row. |

Closed operation packs such as `ScopedReadWriteEffects` are exported for
composition with `...Pack` row spreads. Row spreads currently name simple local
aliases, so bind a short alias first when composing an imported pack:
`LocalEffects :: type fs.ScopedReadWriteEffects;`.

## API

| Name | Type | Notes |
| --- | --- | --- |
| `openRead` | `FsRead -> Path -> Reader ! { fs.openRead : Path -> Reader; }` | Opens a text reader. |
| `readLine` | `FsRead -> Reader -> ReadLine Text?` | Reads one UTF-8 line; strips one trailing `\n` and one optional preceding `\r`; EOF is `#none`. |
| `closeRead` | `FsRead -> Reader -> Unit ! { fs.closeRead : Reader -> Unit; }` | Idempotent for known handles. |
| `openWrite` | `FsWrite -> Path -> Writer ! { fs.openWrite : Path -> Writer; }` | Creates or truncates a text writer. |
| `writeText` | `FsWrite -> Writer -> Text -> WriteText Unit` | Writes bytes exactly; no implicit newline. |
| `flush` | `FsWrite -> Writer -> Unit ! { fs.flush : Writer -> Unit; }` | Explicit flush. |
| `closeWrite` | `FsWrite -> Writer -> Unit ! { fs.closeWrite : Writer -> Unit; }` | Flushes before close; idempotent for known handles. |
| `withReader` | `FsRead -> Path -> (Reader -> A ! { ...ReadLineEffects; ...e; }) -> A ! { ...ScopedReadEffects; ...e; }` | Bracket helper; closes in `finally` when the callback settles. |
| `withWriter` | `FsWrite -> Path -> (Writer -> A ! { ...WriteTextEffects; fs.flush : Writer -> Unit; ...e; }) -> A ! { ...ScopedWriteEffects; ...e; }` | Bracket helper; closes in `finally` when the callback settles. |
| `readAll` | `FsRead -> Path -> WholeRead Text` | Compatibility wrapper over existing `fs.read`. |
| `writeAll` | `FsWrite -> Path -> Text -> WholeWrite Unit` | Compatibility wrapper over existing `fs.write`. |

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

path :: Path = "examples/stdlib_fs_lines.out";

main :: { read : FsRead; write : FsWrite; } -> fs.ScopedReadWrite Text
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

path :: Path = "examples/stdlib_fs_manual.out";

main :: { read : FsRead; write : FsWrite; } -> fs.ScopedReadWrite Text?
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

path :: Path = "examples/stdlib_fs_whole_file.out";

main :: { read : FsRead; write : FsWrite; } -> fs.WholeFile Bool
  = caps => [
    fs.writeAll caps.write path "contents\n";
    fs.readAll caps.read path == "contents\n"
  ];

main
```
