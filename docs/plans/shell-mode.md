# Shell mode — post-v1 surface layer over general mode

**Status:** post-v1 planning note; not part of the v0 or v1 language specs.

## Context

Zutai general mode (`.zt`) is spec'd as pure, lazy, and statically typed. The question is
whether Zutai can also serve as an interactive shell language — something a user types
commands into at a REPL, pipes data through, and uses to drive the system.

This document is a post-v1 design plan. Shell mode depends on language machinery that
is deliberately out of v0/v1's critical path — especially constraints/witnesses for
command-specific rendering and a settled effect-system encoding. Sections marked
**Decided** are committed for this future shell-mode design; sub-questions flagged
inline are the next round to settle before this plan becomes a spec.

## Why "scripting" is the wrong framing

Initial instinct was to make general mode feel more "scripting-like" by deferring type
checking to runtime. This was rejected:

- **Interpreted ≠ dynamically typed.** GHCi, OCaml's REPL, TypeScript via ts-node all
  fully typecheck before evaluating. Interpreter vs compiler is orthogonal to static vs
  dynamic typing.
- **Lazy + dynamic is a famously bad combo.** In a lazy language, type errors surface
  when a thunk is forced — arbitrarily far from where the bad value was constructed.
  Error locality is destroyed. This is a primary reason Haskell is statically typed.
- **Pure + scripting fights itself.** Scripts want pervasive I/O; purity forces effect
  ceremony that breaks the lightweight feel.

## Why "shell mode" is the right framing

Shell mode reframes the problem so the lazy/typed/pure combo works *for* the language:

- **Demand-driven pipelines are stream-friendly.** A stage doesn't run until something
  downstream pulls; `ls | head -5` doesn't enumerate the whole directory.
- **Structured values through pipes are well-trodden.** Nushell, PowerShell, and Elvish
  all demonstrate this design space. The value vocabulary `.zti` already describes —
  records, lists, atoms — is exactly what flows between pipe stages, so no new value
  model is needed.
- **Static types catch composition errors at REPL-prompt time**, not deep inside a
  forced thunk.

## Core design move

Shell mode is **not** a third semantic core. It's general mode with two additions:

1. **REPL-first surface syntax** — bare identifiers in command position resolve to
   functions; quoted strings are literals; newlines separate statements; `--key value`
   pairs desugar to record fields. See "Bare words and flags."
2. **Ambient effect context** — top-level statements run inside an implicit `Shell`
   effect, so users don't write `do`-block plumbing for side effects. `Shell` itself is
   user-composed from fine-grained primitives. See "Effect granularity."

No new pipeline operator: general mode already provides `|>` (see
`04-general-mode/functions.md` §8.6) and shell mode reuses it as-is. The type system,
evaluator, and core semantics are the same. Two surface syntaxes, one semantic core.

## Pipelines

Once values are structured and commands are functions, Unix `|` carries baggage from a
world (byte streams) Zutai has already left. Shell mode reuses general mode's existing
`|>` operator (spec'd in `04-general-mode/functions.md` §8.6) — reverse function
application, already settled by F#, Elixir, and OCaml:

```
ls "/tmp"
  |> filter (\e => e.size > 100)
  |> sort-by (\e => e.mtime)
```

is syntactic sugar for:

```
sort-by (\e => e.mtime) (filter (\e => e.size > 100) (ls "/tmp"))
```

Same AST, different surface.

**Decided:** no implicit `|>` on newline+indent. A bare newline is a statement separator
(see "Multi-line semantics"), not a pipe. Users write `|>` explicitly when they want a
pipe. This keeps the rule that newlines mean "next statement" honest, and avoids the
ambiguity of indentation-sensitive parsing.

## Multi-line semantics

Multiple statements on separate lines (or separated by `;`) form an implicit do-block:
each line is a statement in the ambient `Shell` effect, effects sequence top-to-bottom,
pure values don't need to. This is how OCaml's `;` and Haskell's `do` already work —
shell mode makes that sequencing implicit at the top level instead of requiring users
to open a block.

## Command definitions

**Decided:** a command is just a function. There is no separate command-definition
syntax — shell mode reuses general mode's `::` declaration form
(`04-general-mode/functions.md` §8.1):

```zt
greet :: Text -> Shell Unit
      :: name { print ("hello, " ++ name) }
```

Invoked at the REPL as `greet "world"` (shell-mode parsing: bare words → commands,
quoted strings → literals). The signature, currying, multi-clause patterns, and
inference rules all work as in general mode. Defining a shell command and defining a
function are the same operation.

This is the key reason shell mode is a surface-syntax layer and not a third semantic
core: there is no new declaration syntax to learn or specify.

## Typing external commands

Nushell-style: defined commands have signatures and participate in inference; undefined
external binaries default to a fallback signature like `Cmd : List Text -> Proc Bytes`.
Users can wrap unknown binaries later to upgrade them into the typed world.

This keeps Unix-tool compatibility without polluting the type system with implicit
`Any`.

## Bare words and flags

**Decided:** bare words like `foo` are always identifiers — same lexical category as in
general mode, never silently promoted to strings. What differs by mode is *resolution*,
not *lexing*:

- **Command position** (start of a statement, after `|>`, etc.): the identifier resolves
  as a function reference and the rest of the line is its arguments.
- **Expression position** (inside `()`, RHS of `:=`, anywhere a value is expected):
  the identifier resolves as a variable reference, exactly as in general mode.

This is how every working shell already handles it; the only new rule is "command
position exists." Strings still require quotes (`"foo"`); paths and other text-shaped
values too. Trading off some convenience for honesty — `rm foo` won't accidentally mean
"remove the file whose name is the value of variable `foo`."

**Flags are records.** A flag pair `--key value` desugars to a record field; the whole
flag block becomes a record argument. So:

```
circle --radius 5 --color "red"
```

desugars to:

```zt
circle { radius = 5; color = "red"; }
```

These are the same call. A command that accepts options is a function whose argument
type is a record:

```zt
circle :: { radius : Int; color : Text; } -> Shell Unit
       :: opts { ... }
```

callable from general mode with explicit record syntax, or from shell mode with
flag syntax. Same type, same evaluator, same dispatch — flag parsing is purely surface
sugar.

Open sub-questions:

- **Boolean flags.** Does bare `--verbose` desugar to `{ verbose = true; }`? Likely yes
  by convention, but the rule has to be precise about how the parser decides "no value
  follows" (next token is another `--flag`? end of line? specific lookahead?).
- **Short flags.** Are `-v` and `--verbose` aliases by default, or must the command's
  signature declare aliases explicitly? Explicit declaration is more honest and avoids
  collisions (`-r` could mean `--recursive` or `--radius`).
- **Positional + flag mixing.** When a command takes both positional args and a flag
  record, what's the desugaring rule? Likely: collect all `--key value` pairs into one
  trailing record argument, everything else is positional in order.

## Bytes-vs-structured boundary

Three boundaries, three different answers:

**1. Defined command → defined command: no ser/de.** Values pass in-memory as the
actual structured Zutai type. Both sides see the same type — this falls out of
"command = function" for free, no encoding involved.

**2. Zutai value → external process (send direction): trait-driven auto-convert,
keyed by the receiving command.** `ShellShow` is a two-parameter trait — the value
type *and* the command type. Different external commands have different argv
conventions, so the same value can render differently depending on the consumer:

```zt
ShellShow @Cmd T  -- how a value of type T renders when sent to Cmd
```

`Show T -> Text` remains a separate, single-arg trait for generic stringification.
`ShellShow @Cmd T` defaults to `Show T` when no command-specific instance is in scope,
so casual cases stay zero-ceremony. Specific commands can override:

```zt
-- git wants paths relative to the repo root
ShellShow @git Path :: p { relative-to-repo-root p }

-- curl wants paths expanded to file:// URLs
ShellShow @curl Path :: p { "file://" ++ absolute p }
```

Then:

```zt
git "commit" "-m" msg
```

dispatches `ShellShow @git` for each argument; `curl path` dispatches
`ShellShow @curl Path` for `path`. The command being applied selects the witness.

Open sub-questions:

- **Dispatch site.** Is `Cmd` in `ShellShow @Cmd T` always inferred from the function
  being applied, or can users specify it explicitly for unusual cases?
- **Currying interaction.** When a command is partially applied (`git-commit := git
  "commit"`), does `ShellShow @git-commit` exist as a distinct instance set, or does it
  inherit from `ShellShow @git`?

**3. External process → Zutai value (receive direction): explicit named parsers.**
Auto-convert is rejected here. Prior art from Nushell and PowerShell — both rejected
auto-deserialization for the same reasons:

- **Format ambiguity** — bytes might parse as JSON *and* YAML *and* a one-column CSV.
  Auto-detect is heuristic and fails silently.
- **No source-of-truth** — when `curl` returns bytes, the runtime can't know they're
  JSON without speculatively parsing.
- **Wrong-place errors** — if `cmd1 | cmd2` auto-converts and `cmd1` emits malformed
  data, the error surfaces inside `cmd2`'s typed code instead of at the boundary the
  user is reasoning about.
- **Cost** — speculatively parsing every external's output is expensive when most
  pipelines are plain text.

External output is `Bytes` (or `Text` after a UTF-8 decoder) until the user explicitly
parses it with named functions: `from-json`, `from-csv`, `from-yaml`, `lines`, `parse`.
Users compose one-line wrappers to upgrade unknown binaries into the typed world:

```zt
curl-json :: Url -> Shell Json
          :: u { curl u |> from-json }
```

This matches what Nu and PowerShell shipped after iterating on the problem: explicit
beats clever at the receive boundary, because the user knows the format and the
runtime doesn't.

## Effect granularity

**Decided:** effects are fine-grained at the type-system level — separate `FS`, `Proc`,
`Env`, `Net`, `Time`, `Random`, etc., not one monolithic `IO`. The user composes them
into their own `Shell` via an rc file (working name `.ztshrc`, exact name TBD):

```zt
-- .ztshrc
Shell := FS + Proc + Env + Net
```

`Shell` is therefore not a built-in type. It is a user-defined alias that the REPL and
script entry points use as the ambient effect set. The default rc ships a `Shell`
covering common interactive needs so casual users never have to think about it.

Why this works:

- **Power users can scope down.** A script that only reads files can declare its own
  `Shell := FS` and get a compile-time guarantee that no network or process calls slip
  in — the typechecker rejects them at the call site, not at runtime.
- **Capability-style sandboxing falls out for free.** Running a third-party script
  under a restricted `Shell` alias bounds what it can do without runtime sandboxing
  machinery.
- **Library code stays honest.** A function that only needs `FS` signs as
  `... -> FS A`, not `... -> Shell A`. It composes into any ambient that includes `FS`.
- **No new spec surface for "what is in Shell."** The answer is "whatever the rc says,"
  which is the only honest answer anyway.

Open sub-questions:

- **How effects compose syntactically.** `+` (row union) vs. `&` vs. some other
  operator. Depends on the post-v1 effect-system encoding chosen after row
  polymorphism and constraints/witnesses are settled.
- **rc file format.** Plain `.zt` evaluated at REPL start, or a restricted subset?
  Plain `.zt` is simpler but lets the rc do arbitrary things; restricted is safer.

## Laziness boundary

**Decided:** pipes/conduit-style streaming, not Haskell-style lazy IO. Three rules:

**1. Statements are eager and ordered.** Top-level statements and lines separated by
`;` or newlines run to completion in source order:

```
cd "/tmp"
ls
cd "/home"
```

Each statement finishes before the next starts — standard do-block semantics. Deferring
`cd` would break any user's shell intuition.

**2. Pure values inside a statement stay lazy.** General-mode rules
(`docs/v0_spec/04-general-mode/laziness-and-purity.md`) apply unchanged:

```
expensive := slow-pure-thing cfg
print "hi"
```

`expensive` is not computed unless demanded. Nothing about shell mode forces eager
evaluation of pure bindings.

**3. Pipelines are demand-driven, but each pull is committing.** This is the load-bearing
rule:

```
ls "/tmp" |> filter (\e => e.size > 100) |> head 5
```

- The whole pipeline is one `Shell` expression and does not run until forced.
- Once forced, `head 5` pulls items from `filter`, which pulls from `ls`. Stages run
  on demand.
- Each pulled item's effect fires **eagerly and immediately at pull time** — there is
  no deferred-effect thunk floating around.
- `head 5` stops after 5 items; later items' effects never fire.

This is the model `pipes`/`conduit`/`streamly` in Haskell converged on, and what
generator/iterator semantics in eager languages already do.

Why this is right and lazy IO is wrong:

- **Error locality.** Errors surface at the pulling stage when the pull happens, not
  inside an innocent consumer that eventually demands a thunk built three stages back.
- **Predictable resource lifetime.** A file handle opened by `read-lines` stays open
  while a consumer is pulling and closes deterministically when the pipeline ends.
  No "GC will eventually close it" surprises.
- **No `unsafeInterleaveIO` trap.** Pure suspended IO actions cannot escape into the
  pure value world — effects can only fire inside the streaming machinery.
- **Matches shell intuition.** `ls | head -5` does not list the whole directory in any
  shell. Lazy streams give that property for free; eager evaluation does not.

Edge cases:

- **Unused effects.** A `print "hi"` inside a pipeline that nothing forces produces
  no output. The typechecker should warn — analogous to GHC's `-Wunused-do-bind` —
  because dropped effects are almost always bugs.
- **REPL forcing.** Every top-level expression at the shell-mode REPL is implicitly
  forced (printed or discarded), so users never observe the "must be forced" rule
  interactively. Only script authors who build pipelines into named variables can
  observe it, and the unused-effect warning catches the common mistake.

Spec interaction: the existing "general mode is pure and lazy, no ambient effects"
rule is preserved verbatim. Streaming-effect rules are scoped to ambient `Shell`-family
effects only.

## Interpreter

Every shell is interpreted — no one AOT-compiles bash. Tree-walking the typed AST is
fine for the initial post-v1 shell implementation: the typecheck pass still happens
before evaluation, the interpreter just doesn't emit code. Performance work can wait
until non-trivial scripts exist and the hot paths are known.

## Open sub-questions

Pulled from the sections above, to settle before this plan becomes spec:

- **Bare words and flags.** Boolean-flag desugaring (`--verbose` → `{ verbose = true; }`,
  and how the parser decides "no value follows"); short-flag aliasing (default vs.
  declared); positional/flag mixing rules.
- **Bytes-vs-structured.** `ShellShow @Cmd T` dispatch-site inference; currying
  interaction (does `git "commit"` inherit `ShellShow @git` instances or get its own?).
- **Effect granularity.** Composition operator (`+` vs. `&`, tied to the post-v1
  effect-system encoding); rc file format (full `.zt` vs. restricted subset).
- **Laziness boundary.** Exact warning rules for unused effects (analogue of GHC's
  `-Wunused-do-bind`).

## Related

- Prior art: Nushell (structured pipes, typed signatures, falls back to bytes for
  unknowns), PowerShell (typed objects through pipes, OO-heavy), Elvish (structured
  pipes, dynamic).
- `docs/v0_spec/04-general-mode/laziness-and-purity.md` — current laziness/purity rules
  that shell mode must respect.
