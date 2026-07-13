# Zutai official website

The official site is itself a Zutai browser application. `main.zt` owns the
Elm-style `Model` / `Msg` / `update` / `view` loop — the `Model` tracks the
active mode (`#immediate` / `#general`), a live service roster, and a draft
field. `sections.zt` contains the page components (header, hero, modes, live
demo, why, pipeline, get-started, footer) plus a typed syntax highlighter that
colors pre-tokenized code samples. The demo section is an interactive dogfood:
it runs the hero's own typed `Service` / `healthy` / `ready` code over a keyed,
health-sorted roster the reader deploys, pauses, error-bumps, and removes,
exercising the browser kernel's retained-tree DOM reconciler (keyed reorder,
attribute and list diffing) on every click. `styles.zt` produces a typed
`css.Stylesheet`, and `content.zti` keeps all editorial copy, code tokens, seed
data, and link data as inert immediate-mode data — no logic, so every section is
data-driven.

Build the production bundle from the repository root:

```sh
just web-build
# Equivalent direct invocation:
cargo run -p zutai-web -- build website/main.zt
```

The build writes a prerendered `dist/index.html`, copies `website/public/`, and
places content-addressed JavaScript and WebAssembly under `dist/_zutai/`.
`just web-preview` serves that output locally with Wrangler Pages.

The Cloudflare Pages Direct Upload project `zutai-lang` uses `main` as its
production branch. The repository workflow publishes `main` to production and
same-repository pull requests to `pr-<number>` preview branches. Fork pull
requests run the build job but never enter the credential-bearing deploy job.

Configure these GitHub Actions secrets before enabling delivery:

- `CLOUDFLARE_ACCOUNT_ID`
- `CLOUDFLARE_API_TOKEN`, scoped to the account with Cloudflare Pages Edit

The project has already been bootstrapped. CI is the normal publication path;
an authenticated maintainer can reproduce a production upload with:

```sh
wrangler pages deploy dist --project-name=zutai-lang --branch=main
```
