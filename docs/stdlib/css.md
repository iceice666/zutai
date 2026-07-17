# Standard Library: CSS

## Status

Accepted and implemented as an explicit filesystem source module:
`css ::= import stdlib.css`. The module represents stylesheets as typed data;
the browser kernel validates and renders that tree during prerendering and DOM
updates.

The module source lives at `stdlib/packages/web/modules/css.zt` and is
registered by the filesystem stdlib manifest.

## Small stylesheet

```zt
css ::= import stdlib.css;

accent ::= css.hex "#0f7285";

css.stylesheet {
  css.rule { css.class "card"; } {
    css.padding (css.rem 1.5);
    css.borderRadius (css.rem 0.75);
    css.background (css.namedColor "white");
  };
  css.rule { css.hover (css.class "card"); } {
    css.color accent;
  };
  css.reducedMotion {
    css.rule { css.all; } { css.transition (css.keyword "none"); };
  };
}
```

Attach a stylesheet to a document head with `html.style sheet`, or attach a
small declaration list to an element with `html.styles declarations`.

## Core types

```zt
Stylesheet
Rule
Selector
Pseudo
MediaQuery
KeyframeStop
Keyframe
Property
Declaration
Value
LengthUnit
DurationUnit
Separator
```

`Rule` is a closed union of structured style rules, media rules, keyframes, and
an explicit unsafe escape. Selectors, media queries, properties, and values are
also typed unions rather than strings.

## Constructors

Rules and selectors:

```zt
stylesheet rule media keyframes keyframe from to atPercent
all tag class id compound descendant child
hover focus focusVisible disabled firstChild lastChild
```

Media queries:

```zt
minWidthQuery maxWidthQuery prefersDark prefersLight
prefersReducedMotion andQueries reducedMotion
```

Values:

```zt
keyword number px rem em percent vw vh fr ch vmin vmax
milliseconds seconds hex namedColor cssString
space comma slash function variable variableOr
```

Declarations include typed helpers for common layout, box, color, typography,
and interaction properties:

```zt
display position width height margin padding gap
alignItems justifyContent flexDirection flexWrap
color background border borderRadius boxShadow
fontFamily fontSize fontWeight lineHeight
textAlign textDecoration cursor overflow opacity
transform transition animation outline
```

Use `customProperty name value` for a valid custom property and
`defineVariable name value` for a CSS variable definition. `important`
transforms an existing declaration.

## Safety and support level

Structured CSS is the default path. The renderer rejects invalid identifiers,
invalid numeric values, and non-finite numbers. Structured styles are rendered
by the native prerenderer and by the Wasm browser kernel from the same decoded
`Stylesheet` value.

The module exposes visibly named escape hatches:

```zt
unsafeRawRule unsafeRawSelector unsafeRawMedia
unsafeRawValue unsafeRawDeclaration
```

Unsafe raw rules, selectors, media queries, and values are rejected unless the
host explicitly enables unsafe CSS. Unsafe raw property names are refused by
browser document decoding. These are opt-in boundary operations, not a fallback
for ordinary styling.

See `website/styles.zt` for a full typed design system and
`crates/browser/kernel/src/css.rs` for renderer validation coverage.
