use std::mem;

use rowan::GreenNodeBuilder;
use text_size::TextSize;

use crate::{SyntaxError, SyntaxKind, lexer::Token};

/// An entry in the flat event stream emitted by the parser.
pub(crate) enum Event {
    /// Open a new node. `forward_parent` is the distance (in the events vec) from this entry to
    /// the `Start` of an enclosing node opened via `CompletedMarker::precede` — used to emit the
    /// enclosing node first when processing.
    Start {
        kind: SyntaxKind,
        forward_parent: Option<u32>,
    },
    /// Close the most recently opened node.
    Finish,
    /// Consume exactly one non-trivia raw token.
    Token { kind: SyntaxKind },
    /// Record a parse error at the current raw position.
    Error { msg: String },
}

impl Event {
    pub(crate) fn tombstone() -> Self {
        Self::Start {
            kind: SyntaxKind::TOMBSTONE,
            forward_parent: None,
        }
    }
}

/// Replay `events` into a `rowan` green tree, re-attaching trivia losslessly.
///
/// Trivia rule: leading trivia before a non-trivia token is emitted as leaves directly preceding
/// that token inside whatever node is currently open. Trailing trivia (after the last non-trivia
/// token) is flushed into the root before it closes.
pub(crate) fn process(
    mut events: Vec<Event>,
    raw: &[Token],
    src: &str,
) -> (rowan::GreenNode, Vec<SyntaxError>) {
    let mut builder = Builder {
        inner: GreenNodeBuilder::new(),
        raw,
        src,
        raw_pos: 0,
        text_pos: 0,
        depth: 0,
        errors: Vec::new(),
    };

    let len = events.len();
    for i in 0..len {
        // Take the event out (replacing with tombstone) to avoid borrowing issues.
        let ev = mem::replace(&mut events[i], Event::tombstone());
        match ev {
            Event::Start {
                kind: SyntaxKind::TOMBSTONE,
                forward_parent: None,
            } => {
                // Abandoned marker or already-consumed ancestor — skip.
            }
            Event::Start {
                kind,
                forward_parent,
            } => {
                // Walk the forward_parent chain to collect all ancestor kinds. Each ancestor is
                // `mem::replace`d with a tombstone so the outer loop skips it when we reach it.
                let mut kinds = Vec::new();
                kinds.push(kind);
                let mut fp = forward_parent;
                while let Some(delta) = fp {
                    let anc_idx = i + delta as usize;
                    let ancestor = mem::replace(&mut events[anc_idx], Event::tombstone());
                    match ancestor {
                        Event::Start {
                            kind: ak,
                            forward_parent: afp,
                        } => {
                            if ak != SyntaxKind::TOMBSTONE {
                                kinds.push(ak);
                            }
                            fp = afp;
                        }
                        _ => unreachable!("forward_parent must point to a Start event"),
                    }
                }
                // Emit outermost (last collected) first, then inward.
                for k in kinds.into_iter().rev() {
                    builder.start_node(k);
                }
            }
            Event::Finish => builder.finish_node(),
            Event::Token { kind } => builder.token(kind),
            Event::Error { msg } => builder.error(msg),
        }
    }

    let root = builder.inner.finish();
    (root, builder.errors)
}

struct Builder<'raw, 'src> {
    inner: GreenNodeBuilder<'static>,
    raw: &'raw [Token],
    src: &'src str,
    raw_pos: usize,
    text_pos: usize,
    depth: usize,
    errors: Vec<SyntaxError>,
}

impl Builder<'_, '_> {
    fn eat_trivia(&mut self) {
        while let Some(tok) = self.raw.get(self.raw_pos) {
            if !tok.kind.is_trivia() {
                break;
            }
            let len = tok.len as usize;
            let text = &self.src[self.text_pos..self.text_pos + len];
            self.inner.token(tok.kind.into(), text);
            self.raw_pos += 1;
            self.text_pos += len;
        }
    }

    fn token(&mut self, kind: SyntaxKind) {
        self.eat_trivia();
        if let Some(tok) = self.raw.get(self.raw_pos) {
            debug_assert_eq!(
                tok.kind, kind,
                "event stream out of sync with raw tokens at pos {}",
                self.raw_pos
            );
            let len = tok.len as usize;
            let text = &self.src[self.text_pos..self.text_pos + len];
            self.inner.token(kind.into(), text);
            self.raw_pos += 1;
            self.text_pos += len;
        }
    }

    fn start_node(&mut self, kind: SyntaxKind) {
        self.depth += 1;
        self.inner.start_node(kind.into());
    }

    fn finish_node(&mut self) {
        self.depth -= 1;
        // Flush trailing trivia into the root before it closes.
        if self.depth == 0 {
            self.eat_trivia();
        }
        self.inner.finish_node();
    }

    fn error(&mut self, msg: String) {
        self.errors.push(SyntaxError {
            message: msg,
            offset: TextSize::new(self.text_pos as u32),
        });
    }
}
