//! Browser execution support for Zutai programs.
//!
//! The platform-neutral half of this crate owns the web bundle schema,
//! browser-program decoding, CSS serialization, and build-time prerendering.
//! The `wasm32` half retains the evaluated model and handler closures while it
//! hydrates and patches the live DOM.

mod bundle;
mod css;
mod decode;
mod document;
mod render;

#[cfg(target_arch = "wasm32")]
mod dom;

pub use bundle::WebBundleV3;
pub use css::render_stylesheet;
pub use decode::{BrowserProgram, decode_document, decode_program};
pub use document::*;
pub use render::{PrerenderedPage, prerender_document};

#[cfg(target_arch = "wasm32")]
pub use dom::start;
