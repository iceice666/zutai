# Zutai Package System and Split Standard Library

Approved 2026-07-13.

## Alignment brief

- **Outcome:** Add a package layer above Zutai's existing `.zt` module semantics and split the standard-library distribution into coherent package units.
- **Audience:** Zutai application and library authors working in local multi-package workspaces; compiler, LSP, native, and web-bundle consumers must observe the same resolution behavior.
- **Success criteria:** A root `.zt` program can import modules from declared local path dependencies; existing relative and `stdlib.*` imports remain compatible; diagnostics identify manifest and resolution failures precisely; interpreter, native lowering, LSP analysis, and portable web bundles receive the same resolved sources.
- **Must-haves:** Zutai-native inert manifests named `zutai.zti`; local path dependencies; dependency aliases; explicit module maps; transitive package resolution; cycle, duplicate, unsafe-path, missing-module, and compatibility diagnostics; four physical stdlib units; documentation and focused acceptance coverage.
- **Non-goals:** Remote registries, network fetching, Git dependencies, semver solving, lockfiles, build scripts, executable manifests, conditional dependencies, features, or new export syntax.
- **Compatibility:** Preserve quoted relative imports, `import stdlib.<module>`, module-record exports, selective import bindings, the quoted-import subtree security boundary, and current ambient prelude behavior.
- **Manifest decision:** Use `zutai.zti`, not `.zt`, because package metadata is inert data needed before module resolution and stdlib loading. The package loader must not evaluate code while discovering the dependency graph.
- **Package model:** A package has a name, compiler compatibility, a map of public module names to package-relative `.zt` files, and dependency aliases mapped to local package paths. Dotted imports use the first segment as a dependency alias and the remaining segments as its public module name. `stdlib` remains a reserved compatibility alias supplied by the toolchain.
- **Stdlib shape:** `base` owns prelude, stream, optional, result, num, text, cmp, and list; `data` owns data, validate, config, and reflect; `system` owns fs and net; `web` owns css, html, and browser. An umbrella index preserves all existing `stdlib.*` paths and compiler compatibility as one selected toolchain distribution.
- **Implementation approach:** Extend source ownership and import resolution without changing parser syntax. Discover the nearest package manifest from the root source, validate the local dependency graph, resolve dotted imports through aliases, and record the transitively used package sources for portable bundles. Refactor stdlib loading into the same package primitives where practical while retaining its configured-root selection contract.
- **Acceptance checks:** Existing import tests remain green; add local direct/transitive dependency tests, alias and module lookup tests, manifest validation/security tests, dependency-cycle tests, LSP analysis coverage, native/import lowering coverage where locally available, and web-bundle round trips containing only required package/stdlib sources. Run formatting, focused crate tests, workspace tests, and clippy when practical.
- **Rollback risk:** Resolver changes affect every semantic entry point. Keep the old source forms and stdlib alias stable, introduce package discovery behind roots that actually contain `zutai.zti`, and make package metadata transport explicit rather than process-global.

## Fresh-thread prompt

Use this plan as the project brief. First read the whole brief, then implement it. Preserve the stated constraints, non-goals, and acceptance checks. Read `docs/spec/07-modules/modules.md` before changing import semantics and keep `.zti` inert. If anything is ambiguous, ask only the smallest blocking question before building.
