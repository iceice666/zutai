# Archived Decisions

These closed stabilization items stay here so old risk decisions remain visible.
New unresolved work should become an open milestone in the [roadmap](roadmap.md).

- [x] **Compiler entry-type gate cleanup** — CLI `compile` and `dataflow`
  reject final runtime `Type` values before TLC→DC/LLVM lowering, including raw
  `type Int` entries and alias-value entries such as `MyInt :: type Int; MyInt`.
- [x] **v0 spec conformance sweep** — code fences from `docs/spec/` are
  extracted and routed through `check`/`run` for `.zt` survivors and the immediate
  parser for `.zti` survivors; stable survivors are promoted to acceptance tests.
- [x] **Diagnostic polish** — record-vs-record type mismatches render source-like
  record shapes, including optional fields and row tails; row-tail spread
  overlaps report the spread source and existing/incoming shapes.
- [x] **TLC-first evaluator cutover** — default evaluation runs through TLC for
  executable value programs; THIR remains the explicit regression oracle and
  runtime `Type`/reflection boundary.


Older milestone-specific decisions and superseded implementation choices remain
in the dated [implementation history](../history/README.md).

## Package distribution: locked Git source snapshots

_Accepted and implemented 2026-07-16; retained here as the package trust and identity contract._

Zutai extends the existing local package graph with **locked Git sources**,
not a registry or a semantic-version solver. Imports remain alias-based
(`math.vector`); repository locations and revisions never become language-level
module names.

### Source model

- Local path dependencies remain the development/workspace mechanism.
- Remote dependencies use generic HTTPS Git repositories. The first remote
  implementation does not accept SSH/scp syntax, `file:` URLs, registries,
  submodules, Git LFS, executable hooks, or package build scripts.
- Manifest format 2 represents the source as an explicit tagged block rather
  than inferring a variant from whichever fields happen to be present:

  ```zti
  {
    formatVersion = 2;
    name = "app";
    compilerCompatibility = ">=0.1.0, <0.2.0";
    modules = [];
    dependencies = [
      {
        alias = "local_math";
        source = { kind = #path; path = "../math"; };
      };
      {
        alias = "remote_math";
        source = {
          kind = #git;
          url = "https://github.com/example/math.git";
          rev = "refs/tags/v1.4.0";
          subdir = ".";
        };
      };
    ];
  }
  ```

- `rev` is an update selector, not the build identity. It must be either a full
  Git object ID for the repository's advertised object format or a fully
  qualified `refs/heads/...` / `refs/tags/...` name; abbreviated IDs, ambiguous
  shorthand, symbolic `HEAD`, and an implicit default branch are rejected.
  Annotated tags are peeled to commits, and only commit objects are accepted.
  `subdir` defaults to `.` and selects a package in a repository monorepo.
- Format-1 manifests retain their current exact compiler-compatibility check.
  Format 2 treats `compilerCompatibility` as a SemVer requirement. Dependency
  versions themselves are not solved: changing a desired tag/ref is an explicit
  manifest edit, and refreshing a moving ref is an explicit update operation.

Git URL normalization is intentionally narrow: reject credentials, query
strings, and fragments; lowercase the scheme and DNS host; remove the default
HTTPS port and path dot-segments; otherwise preserve the repository path (in
particular, do not guess whether `.git` is equivalent). HTTPS redirects may not
downgrade transport. The lock records the final normalized `resolvedUrl`, and a
later fetch must reach the same URL unless an explicit update rewrites the lock.

There is no package `version` field in format 2. Without a registry or solver it
would be unactionable metadata: the requested Git ref expresses update intent,
while the locked commit and source hash express exact identity.

### Identity and graph rules

A package node is identified by its source instance and package root, not by its
manifest `name`:

- root/local node: canonical package path represented relative to the root
  package;
- Git node: normalized URL + locked commit + package `subdir`.

Path dependencies inherit their owner's source domain. A path reached from a
root/local package remains a local node. A path reached from a Git package must
stay inside the same locked repository tree and becomes a Git node with the
same URL and commit but a normalized target subdir. This permits monorepo sibling
packages, shares one cached repository snapshot, and prevents fetched source
from reaching into the host filesystem.

Dependency aliases remain local to the owning package and point to node IDs.
Consequently, two revisions of a package may coexist transitively without a
global solver or name collision. `name` remains validated diagnostic metadata.
Cycles are detected over node IDs.


### Lockfile and reproducibility

The root package owns `zutai.lock.zti`. A lockfile is required whenever the
resolved graph contains a Git source. It records, without timestamps:

- a deterministic node ID and package name;
- the source variant, normalized URL, requested ref, exact commit, and subdir;
- a `sha256:` source-tree hash and package-manifest hash;
- every alias-to-node edge in the complete transitive graph; and
- the lock format and generating compiler version.

A canonical lock has this shape (hash and ID values abbreviated here only):

```zti
{
  formatVersion = 1;
  generatedBy = "zutai-cli 0.1.0";
  root = "pkg:4d2f...";
  packages = [
    {
      id = "pkg:4d2f...";
      name = "app";
      source = { kind = #path; path = "."; };
      manifestHash = "sha256:...";
      dependencies = [
        { alias = "math"; package = "pkg:91ac..."; };
      ];
    };
    {
      id = "pkg:91ac...";
      name = "math";
      source = {
        kind = #git;
        url = "https://github.com/example/math.git";
        resolvedUrl = "https://github.com/example/math.git";
        requestedRev = "refs/tags/v1.4.0";
        commit = "0123456789abcdef0123456789abcdef01234567";
        objectFormat = #sha1;
        subdir = ".";
        treeHash = "sha256:...";
      };
      manifestHash = "sha256:...";
      dependencies = [];
    };
  ];
}
```

The lock grammar requires full, unabbreviated object IDs and digests and records
the Git object format (`sha1` or `sha256`) beside each commit. Node IDs are
`pkg:` plus a lowercase base32 SHA-256 digest over a versioned, length-prefixed
source-identity tuple. Local identity uses the normalized path spelled relative
to the root package; reaching one canonical local root through two different
path identities is an error. Git identity uses normalized URL, object format,
exact commit bytes, and normalized subdir. IDs are opaque to imports and source
code.

Only Git sources carry `treeHash`; ordinary local source edits do not make the
lock stale. Every node carries `manifestHash`, so changes to local dependency
edges, module publication, compatibility, or package identity require `sync`.
Local path strings and dependency arrays are part of the canonical lock and are
emitted in bytewise order by node ID and alias.


The Git source-tree hash covers the complete committed repository tree, not only
one package subdir, because a fetched package may use an in-repository path
dependency. The canonical byte stream starts with a lock-format-specific magic
value and then encodes each tracked entry in raw Git path order as mode, UTF-8
path length/path, content length/content. Only regular files with modes `100644`
and `100755` are accepted; symlinks, Git links/submodules, non-UTF-8 paths,
backslashes, and path components unsafe to materialize portably are rejected.
The hash therefore verifies the immutable snapshot without trusting Git SHA-1.
Changing a local manifest invalidates its lock node; changed fetched bytes or a
mutable ref without an explicit update fails integrity verification.

### Acquisition, cache, and trust

Package acquisition is an explicit native preflight, never part of parsing,
name resolution, type checking, LSP analysis, Wasm execution, or backend
lowering:

- `zutai-cli package sync` validates all manifests, preserves every unchanged
  locked selector, resolves only new or changed selectors, removes unreachable
  nodes, atomically rewrites the lock, and fills the cache;
- `zutai-cli package fetch` fills the cache strictly from an unchanged valid
  lock, fetching exact commits rather than re-resolving requested refs;
- `zutai-cli package update [alias ...]` deliberately re-resolves all or selected
  requested refs, then atomically rewrites the lock and fills the cache.

Normal `check`, `run`, `compile`, `lsp`, and web-build analysis do not access the
network or rewrite the lock. They issue an actionable error when the lock is
stale or a locked snapshot is absent. `sync --offline` and `fetch --offline`
must succeed only from already cached Git objects/snapshots.

The implementation belongs in a dedicated `zutai-package` crate: portable
manifest/lock/graph types compile on every target, while Git acquisition and
cache mutation are native-only. `zutai-semantic` consumes a fully prepared
immutable graph and remains responsible for import resolution and analysis.

The native cache uses immutable content-addressed snapshots under the platform
cache directory, keyed by source-tree hash. Fetching occurs into a temporary
location, verifies commit and source hash, then installs by atomic rename under
an interprocess lock. Compilation reads the immutable snapshot, never a mutable
Git checkout. HTTPS credentials may come from host credential configuration but
must never be serialized into a manifest, lockfile, diagnostic, or cache key.

Acquisition must enforce configured limits before materialization: redirect
count, fetched bytes, entry count, per-file bytes, total expanded bytes, and path
depth/length. Limit failures are source-located package errors. The fetcher must
disable recursive submodule and Git-LFS smudge behavior and never execute hooks
or checkout filters. Repository object verification and tree-hash verification
both occur before an immutable snapshot becomes visible in the cache.

The checked-in lockfile is the project's trust decision. There is no global
checksum authority in the first implementation. A changed hash, rewritten tag,
unavailable commit, unsafe tree entry, URL redirect to a non-HTTPS transport, or
manifest/source mismatch is a hard error rather than a fallback.

Format-2 path dependencies remain the explicit local-development mechanism;
the first implementation has no separate override/patch table. A project that
needs to modify a transitive dependency can point the relevant parent package at
a local checkout. This keeps one dependency mechanism and avoids an invisible
root-only graph mutation.

### Portable and browser builds

The portable package graph changes from package-name keys to lock node IDs and
records only source text actually needed by analysis. Native web-bundle creation
must start from a valid locked/cached graph. The bundle remains the Wasm trust
boundary: the browser kernel performs no Git, filesystem, lockfile, or network
resolution.

This design deliberately leaves registries, semver solving, package signing,
and transparent source mirrors out. They require independent evidence and can
be added without changing alias-based imports or the locked node identity.
