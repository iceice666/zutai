# Standard Library: HTML

## Status

Accepted and implemented as an explicit filesystem source module:
`html ::= import stdlib.html`. The module builds typed document data for the
browser kernel; it does not expose raw HTML or script constructors.

The module source lives at `stdlib/packages/web/modules/html.zt` and is
registered by the filesystem stdlib manifest. `stdlib.html` imports
`stdlib.css` for structured style declarations and stylesheets.

## Small document

```zt
html ::= import stdlib.html;
css ::= import stdlib.css;

Msg :: type { #save; };
save :: Msg = #save;

(html.document
  "en"
  "Profile"
  { html.description "Profile editor"; }
  { html.bodyClass "page"; }
  {
    html.main { html.classAttr "card"; html.styles { css.padding (css.rem 1.5); }; } {
      html.h1 {;} { html.text "Profile"; };
      html.button { html.onClick save; } { html.text "Save"; };
    };
  })
```

A browser entry returns a `browser.Program`, but `view` always produces
`html.Document Msg`:

```zt
browser ::= import stdlib.browser;
html ::= import stdlib.html;

Model :: type { saved : Bool; };
Msg :: type { #save; };
saveMessage :: Msg = #save;

init :: Unit -> browser.Browser Model = _ => { saved = false; };
update :: Msg -> Model -> browser.Browser Model
  = message model => match message { | #save => model with { saved = true; }; };
view :: Model -> html.Document Msg
  = model => (html.document "en" "Profile" {;} {;} {
      html.button { html.onClick saveMessage; } {
        html.text (if model.saved then "Saved" else "Save");
      };
    });

browser.application init update view
```

## Core types

```zt
Document Msg
Html Msg
Attribute Msg
StaticAttribute
EventHandler Msg
EventOptions
Head
Tag
AttributeName
```

`Document Msg` contains `language`, `title`, `head`, `bodyAttributes`, and
`body`. `Html Msg` is a closed `#text` / `#element` union. Event handlers carry
values or functions that produce the program's `Msg`, so wiring the wrong
message type is a type error rather than a runtime callback failure.

## Constructors

Document and head helpers include:

```zt
document meta description viewport author robots themeColor
openGraph ogTitle ogType ogDescription ogImage ogUrl
link stylesheetLink iconLink canonicalLink base style
```

Element helpers cover the supported closed tag set, including `main`, `section`,
`div`, `span`, `form`, `button`, `input`, `selectElement`, `option`, tables, and
headings. Void helpers (`br`, `img`, `input`) do not accept children.

Common attribute helpers:

```zt
idAttr classAttr href src alt placeholder inputType role titleAttr
aria dataAttribute disabledAttr required readOnly
value checked styles key
```

Use `value` and `checked` for controlled inputs. The browser reconciler compares
those properties with live DOM state on every render. Use `key` on siblings
whose identity must survive insertion, removal, or reordering; sibling keys must
be unique.

Events:

```zt
onClick onInput onChange onSubmit onBlur onFocus onKeyDown onKeyUp
```

Every event has an `*With` form that accepts `EventOptions`. `onInput`,
`onChange`, `onKeyDown`, and `onKeyUp` accept `Text -> Msg`; the other helpers
accept a `Msg` value. `preventDefault`, `stopPropagation`, and
`preventDefaultAndStopPropagation` provide common options.

## Safety and support level

- HTML, attributes, head nodes, and event variants are closed typed data.
- There is no raw-HTML, script, or `http-equiv` constructor.
- Prerendering rejects duplicate document IDs, duplicate sibling keys, children
  on void elements, unsafe URL schemes, invalid HTML names, and multiple base
  elements.
- Native analysis and persistent reference-interpreter sessions build and decode
  documents. Terminal `run` does not render a browser `Program`; `zutai-web
  build` evaluates the entry, prerenders the initial document, and emits a
  portable bundle for the Wasm browser kernel.
- The browser kernel tests the same program natively and through the
  `wasm-bindgen-test` hydration scenario.

See `website/main.zt` for a full application and
`crates/browser/kernel/tests/fixture/mod.rs` for a compact executable fixture.
