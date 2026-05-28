mod event;
mod grammar;
mod input;

use std::cell::Cell;

use rowan::GreenNode;

use crate::{SyntaxError, SyntaxKind, lexer::tokenize, token_set::TokenSet};

use event::Event;
use input::Tokens;

const STEP_LIMIT: u32 = 10_000_000;

// ── Public entry point ────────────────────────────────────────────────────────

pub(crate) fn parse(src: &str) -> (GreenNode, Vec<SyntaxError>) {
    let raw = tokenize(src);
    let tokens = Tokens::from_raw(&raw);
    let mut p = Parser::new(tokens);
    grammar::file(&mut p);
    let events = p.into_events();
    event::process(events, &raw, src)
}

// ── Parser ────────────────────────────────────────────────────────────────────

pub(crate) struct Parser {
    tokens: Tokens,
    pos: usize,
    events: Vec<Event>,
    steps: Cell<u32>,
}

impl Parser {
    fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            pos: 0,
            events: Vec::new(),
            steps: Cell::new(0),
        }
    }

    fn into_events(self) -> Vec<Event> {
        self.events
    }

    // ── Peeking ───────────────────────────────────────────────────────────────

    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        let steps = self.steps.get() + 1;
        assert!(
            steps < STEP_LIMIT,
            "parser step limit exceeded — possible infinite loop"
        );
        self.steps.set(steps);
        self.tokens.kind(self.pos + n)
    }

    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    pub(crate) fn nth_at(&self, n: usize, kind: SyntaxKind) -> bool {
        self.nth(n) == kind
    }

    pub(crate) fn at_eof(&self) -> bool {
        self.at(SyntaxKind::EOF)
    }

    /// True when the non-trivia tokens at `pos` and `pos+1` are raw-adjacent (no trivia between).
    pub(crate) fn raw_adjacent(&self) -> bool {
        self.tokens.is_raw_adjacent(self.pos)
    }

    /// True when the non-trivia tokens at `pos-1` and `pos` are raw-adjacent.
    /// Used after a `bump` to check if the token just consumed was adjacent to the current one.
    pub(crate) fn prev_raw_adjacent(&self) -> bool {
        self.pos > 0 && self.tokens.is_raw_adjacent(self.pos - 1)
    }

    /// True when the non-trivia tokens at `pos+offset` and `pos+offset+1` are raw-adjacent.
    /// Used for bounded lookahead adjacency checks (e.g. brace disambiguation, M8 field names).
    pub(crate) fn raw_adjacent_at(&self, offset: usize) -> bool {
        self.tokens.is_raw_adjacent(self.pos + offset)
    }

    // ── Consuming ─────────────────────────────────────────────────────────────

    fn do_bump(&mut self, kind: SyntaxKind) {
        self.events.push(Event::Token { kind });
        self.pos += 1;
        self.steps.set(0);
    }

    /// Bump the current token (any kind).
    pub(crate) fn bump_any(&mut self) {
        if !self.at_eof() {
            let k = self.current();
            self.do_bump(k);
        }
    }

    /// Bump exactly `kind`; panics if current token isn't `kind`.
    pub(crate) fn bump(&mut self, kind: SyntaxKind) {
        assert!(
            self.at(kind),
            "expected {:?}, got {:?}",
            kind,
            self.current()
        );
        self.do_bump(kind);
    }

    /// Bump if current == `kind`; return whether consumed.
    pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.do_bump(kind);
            true
        } else {
            false
        }
    }

    /// Bump `kind` if present; otherwise emit an error and return `false`.
    pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
        if self.eat(kind) {
            true
        } else {
            self.error(format!("expected {kind:?}"));
            false
        }
    }

    /// Emit an error event (does not consume any token).
    pub(crate) fn error(&mut self, msg: impl Into<String>) {
        self.events.push(Event::Error { msg: msg.into() });
    }

    /// Error recovery: emit a diagnostic, then consume stray tokens into an
    /// `ERROR_NODE` until a token in `recovery` or a declaration-start is seen.
    /// Always bumps at least one token (unless at EOF) so the loop makes progress.
    pub(crate) fn err_recover(&mut self, msg: impl Into<String>, recovery: TokenSet) {
        if self.at_eof() {
            self.error(msg);
            return;
        }
        // If we're already at a recovery point, emit a zero-width error and stop.
        if recovery.contains(self.current()) {
            self.error(msg);
            return;
        }
        let err_m = self.start();
        self.error(msg.into());
        // Consume at least one token, then stop at recovery / decl-start.
        self.bump_any();
        while !self.at_eof() && !recovery.contains(self.current()) && !self.is_decl_start() {
            self.bump_any();
        }
        err_m.complete(self, SyntaxKind::ERROR_NODE);
    }

    /// True when the current token is an `IDENT` whose next significant token is
    /// one of `:=`, `:`, or `::` — i.e. the start of a top-level declaration.
    pub(crate) fn is_decl_start(&self) -> bool {
        self.at(SyntaxKind::IDENT)
            && matches!(
                self.nth(1),
                SyntaxKind::COLON_EQ | SyntaxKind::COLON | SyntaxKind::COLON_COLON
            )
    }

    // ── Markers ───────────────────────────────────────────────────────────────

    pub(crate) fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::tombstone());
        Marker {
            pos,
            bomb: DropBomb::new(),
        }
    }
}

// ── Marker ────────────────────────────────────────────────────────────────────

/// An open node marker. Must be completed or abandoned before it is dropped.
pub(crate) struct Marker {
    pos: u32,
    bomb: DropBomb,
}

impl Marker {
    /// Close this marker as a node of the given `kind`, returning a `CompletedMarker` that can be
    /// used with `precede` for left-associative Pratt folding.
    pub(crate) fn complete(mut self, p: &mut Parser, kind: SyntaxKind) -> CompletedMarker {
        self.bomb.defuse();
        let idx = self.pos as usize;
        match &mut p.events[idx] {
            Event::Start { kind: k, .. } => *k = kind,
            _ => unreachable!(),
        }
        p.events.push(Event::Finish);
        CompletedMarker {
            start_pos: self.pos,
        }
    }

    /// Abandon this marker — no node will be emitted. If this is the last event, the tombstone is
    /// popped; otherwise it remains and is skipped by the builder.
    pub(crate) fn abandon(mut self, p: &mut Parser) {
        self.bomb.defuse();
        let idx = self.pos as usize;
        if idx == p.events.len() - 1 {
            p.events.pop();
        }
        // else: leave the tombstone; process() skips TOMBSTONE Start events.
    }
}

// ── CompletedMarker ───────────────────────────────────────────────────────────

/// A completed node that can be wrapped in an outer node via `precede`.
pub(crate) struct CompletedMarker {
    start_pos: u32,
}

impl CompletedMarker {
    /// Open a new outer node that will *contain* this completed node. Returns the new marker.
    ///
    /// Works by setting `forward_parent` on this node's `Start` event to point to the new marker's
    /// `Start`, so the builder emits the outer node first.
    pub(crate) fn precede(self, p: &mut Parser) -> Marker {
        let m = p.start();
        let delta = m.pos - self.start_pos;
        match &mut p.events[self.start_pos as usize] {
            Event::Start { forward_parent, .. } => {
                *forward_parent = Some(delta);
            }
            _ => unreachable!(),
        }
        m
    }
}

// ── Drop bomb ─────────────────────────────────────────────────────────────────

struct DropBomb {
    armed: bool,
}

impl DropBomb {
    fn new() -> Self {
        Self { armed: true }
    }

    fn defuse(&mut self) {
        self.armed = false;
    }
}

impl Drop for DropBomb {
    fn drop(&mut self) {
        if self.armed && !std::thread::panicking() {
            panic!("Marker dropped without being completed or abandoned");
        }
    }
}
