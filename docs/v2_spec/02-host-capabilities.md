# Host Capabilities (v2)

[Algebraic effects (v1)](../spec/v1/05-effects.md) split an effectful computation
into two parts: the effect row records *what* may happen, and a capability value
records *who authorized it*. v1 ships exactly one host capability — `io.print`,
provided ambiently by the `run` boundary — and reserves filesystem, environment,
clock, randomness, and networking as non-ambient capabilities. v2 makes those
host capabilities real.

---

## The Capability Model

A host operation is an ordinary effect operation whose use additionally requires
a capability value in scope. The effect row is checked by the type system; the
capability value is threaded explicitly as a function parameter. Holding the
value is the authorization.

```zt
readConfig :: FsRead -> Text ! { fs.read : Path -> Text; }
  = fs => perform fs.read "zutai.toml";
```

`readConfig` may perform `fs.read` because `fs.read` is in its effect row and it
holds an `FsRead` capability. A caller cannot invoke `readConfig` without
supplying an `FsRead` value, so authority propagates up the call graph by
ordinary parameter passing.

Capability types (`FsRead`, `Env`, …) are opaque: a program obtains capability
values only from the entry boundary, never by construction. As in the v1
example, the capability parameter is advisory — its presence in the signature
conveys authority and it is not consumed by `perform`.

---

## The Entry Boundary

The host grants capabilities at the program entry. A program declares the
capabilities it needs as the fields of its entry parameter and the effect row it
discharges through the host:

```zt
main :: { fs : FsRead; out : IoPrint; } -> Unit ! { fs.read : Path -> Text; io.print : Text -> Text; }
  = caps => {
    contents = readConfig caps.fs;
    perform io.print contents;
  };
```

The `run`/native boundary supplies a capability only if the host grants it. A
residual host operation whose capability the host does not provide is a boundary
error, not a silent no-op.

`io.print` remains available ambiently for source compatibility with v0 and v1;
it may also be requested as an explicit `IoPrint` capability.

---

## Standard Capabilities

v2 defines a standard capability set. Each row lists the operation, its standard
signature, and the capability that authorizes it:

| Operation   | Signature                                       | Capability        |
|-------------|-------------------------------------------------|-------------------|
| `io.print`  | `Text -> Text`                                  | `IoPrint` (ambient) |
| `fs.read`   | `Path -> Text`                                  | `FsRead`          |
| `fs.write`  | `{ path : Path; contents : Text; } -> Unit`     | `FsWrite`         |
| `env.get`   | `Text -> Text?`                                 | `Env`             |
| `clock.now` | `Unit -> Instant`                               | `Clock`           |
| `rng.next`  | `Unit -> Int`                                   | `Rng`             |

`Path`, `Instant`, and related types are standard-library types (see
[`stdlib`](../stdlib/00-index.md)). Networking (`net.*`) is sketched but not
standardized in v2.

Clock and randomness are effects, never ambient reads, so the pure core stays
deterministic: every source of nondeterminism is explicit in a type.

---

## Handling Host Operations

Because host capabilities are effects, `handle` intercepts them like any other
operation. This is how tests mock the host without touching it:

```zt
withFakeFs :: <A> (FsRead -> A ! { fs.read : Path -> Text; }) -> A
  = body =>
    handle body testFs with {
      value   = \a => a;
      fs.read = \path => resume "stub contents";
    };
```

The handler discharges `fs.read`, removing it from the surrounding row, so
`withFakeFs body` is pure. `testFs` is any `FsRead` value; it is never inspected.
The same construction layers logging, sandboxing, or quota handlers over host
operations.

---

## Authority and Safety

v2 capability authority is *advisory*: a capability value is ordinary data, and
possessing it is the authorization. This is enough to make host access explicit
and locally auditable — a function's type states every capability it needs — and
to make host operations mockable through handlers.

v2 does **not** make capabilities unforgeable. Cryptographically or
type-theoretically unforgeable capability tokens, and a typing rule that binds a
specific capability *value* to authority over a specific operation, are reserved
for a future version. The pure core still has no ambient host access of any kind
beyond `io.print`.

---

## Support Level

Host capabilities are dispatched by the runtime effect driver, and so depend on
the v1 effect runtime — the CPS effect lowering and runtime dispatch that
replace compile-time effect folding. The standard set has **landed**
(`docs/ARCHIVED.md` Phase 27): the capability type names `FsRead`, `FsWrite`,
`Env`, `Clock`, `Rng`, and explicit `IoPrint` are seeded in the root scope
(with `Path`/`Instant` as text-shaped boundary types), THIR effect rows
recognize `fs.read`, `fs.write`, `env.get`, `clock.now`, and `rng.next` with
advisory authority, and TLC keeps residual host effects explicit, rejecting
ungranted operations before TLC→DC. The CLI `run`, `dataflow`, and native/LLVM
compile boundaries grant the standard set and lower granted residual effects to
a Dataflow Core `HostOp` node that ANF/SSA/codegen preserve and the
runtime/evaluator dispatch. Ambient `io.print` stays source-compatible, and
`handle` can still intercept any host operation before the boundary. A residual
host operation the host does not grant is still rejected before code generation.
