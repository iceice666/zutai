# Zutai official website

The official site is itself a Zutai browser application and a local package.
`zutai.zti` declares `main.zt` plus the path dependency under `packages/demo/`;
that package owns the interactive service domain (`Service`, status
classification, and roster update helpers). `main.zt` keeps the Elm-style
`Model` / `Msg` / `update` / `view` loop small, while `sections.zt` contains the
page components (header, hero, modes, live demo, why, pipeline, get-started,
footer) plus a typed syntax highlighter. The demo runs the package's typed
`Service` / `healthy` / `status` code over a keyed, health-sorted roster the
reader deploys, pauses, error-bumps, and removes, exercising the browser
kernel's retained-tree DOM reconciler (keyed reorder, attribute and list
diffing), controlled input updates, and browser focus effects on every click.
`styles.zt` produces a typed `css.Stylesheet`, and `content.zti` keeps all
editorial copy, code tokens, seed data, and link data as inert immediate-mode
data. Native and wasm browser tests consume the same portable website bundle,
including the local package graph and exact stdlib closure.

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
