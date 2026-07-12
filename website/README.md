# Zutai official website

The official site is itself a Zutai browser application. `main.zt` owns the
Elm-style `Model` / `Msg` / `update` / `view` loop, `sections.zt` contains the
page components, `styles.zt` produces a typed `css.Stylesheet`, and
`content.zti` keeps editorial copy as inert immediate-mode data.

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
