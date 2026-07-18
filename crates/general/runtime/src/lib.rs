//! Runtime library for compiled Zutai general-mode programs.
//!
//! This crate implements the `@zutai.*` symbols that `zutai-codegen` references
//! and defines the binary representation of runtime values. The full contract
//! is specified in `docs/compiler/runtime-abi.md`; this module is the v0 skeleton of
//! Phase 18 (Runtime & ABI).
//!
//! Every Zutai value is an `i64` (D-0002, untagged, statically dispatched):
//! immediates (`Int`, `Bool`, `Float` bits, dense atom/variant tags) are stored
//! inline; heap values (records, tuples, lists, variants, text) are pointers
//! cast to `i64`, each beginning with a one-word header (D-0009).
//!
//! Memory is a per-thread bump arena (D-0008): each OS thread owns a
//! `thread_local!` arena whose owned chunks are never leaked, bounded by
//! `ZUTAI_HEAP_MAX` (default 2 GiB) so a runaway allocation aborts with a clear
//! diagnostic instead of OOM-killing the host. `zutai.free` is a no-op in v0.
//!
//! Closures (D-0003) are built and applied inline by codegen, so no closure
//! symbol lives here. `TAG_CLOSURE` is reserved for the header layout only.
/// Version of the native runtime ABI consumed by generated code.
///
/// Increment this when a symbol signature, value representation, descriptor
/// layout, or host boundary changes incompatibly.
pub const ABI_VERSION: u32 = 1;

use fast_posit::{Posit, RoundInto};
use zutai_eval::{EvalError, Value as EvalValue};

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::slice;
use std::str;
use std::sync::{
    Mutex, Once, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Object headers (D-0009, Role B reserves the high bits) ──────────────────────

const TAG_RECORD: i64 = 1;
const TAG_TUPLE: i64 = 2;
const TAG_CONS: i64 = 3;
const TAG_NIL: i64 = 4;
const TAG_VARIANT: i64 = 5;
const TAG_TEXT: i64 = 6;
/// Reserved: closures are emitted inline by codegen, not allocated here.
#[allow(dead_code)]
const TAG_CLOSURE: i64 = 7;

/// Pack a header word: low byte = kind tag, next bits = length/arity.
/// High bits stay zero in v0; the current conservative GC does not use layout ids.
const fn header(tag: i64, count: u64) -> i64 {
    ((count << 8) as i64) | tag
}

fn header_tag(h: i64) -> i64 {
    h & 0xff
}

fn header_count(h: i64) -> u64 {
    ((h as u64) >> 8) & 0x00ff_ffff_ffff_ffff
}

// ── Type descriptors (D-0009, Role A — what `show` walks) ───────────────────────

const DESC_INT: i64 = 0;
const DESC_BOOL: i64 = 1;
const DESC_FLOAT: i64 = 2;
const DESC_TEXT: i64 = 3;
const DESC_ATOM: i64 = 4;
const DESC_RECORD: i64 = 5;
const DESC_TUPLE: i64 = 6;
const DESC_LIST: i64 = 7;
const DESC_OPTIONAL: i64 = 8;
const DESC_MAYBE: i64 = 9;
const DESC_VARIANT: i64 = 10;
const DESC_POSIT: i64 = 11;

macro_rules! match_p32_es {
    ($es:expr, $func:ident $(, $arg:expr)* $(,)?) => {
        match $es {
            0 => $func::<0>($($arg),*),
            1 => $func::<1>($($arg),*),
            2 => $func::<2>($($arg),*),
            3 => $func::<3>($($arg),*),
            4 => $func::<4>($($arg),*),
            5 => $func::<5>($($arg),*),
            6 => $func::<6>($($arg),*),
            7 => $func::<7>($($arg),*),
            8 => $func::<8>($($arg),*),
            9 => $func::<9>($($arg),*),
            10 => $func::<10>($($arg),*),
            11 => $func::<11>($($arg),*),
            12 => $func::<12>($($arg),*),
            13 => $func::<13>($($arg),*),
            14 => $func::<14>($($arg),*),
            15 => $func::<15>($($arg),*),
            16 => $func::<16>($($arg),*),
            17 => $func::<17>($($arg),*),
            18 => $func::<18>($($arg),*),
            19 => $func::<19>($($arg),*),
            20 => $func::<20>($($arg),*),
            21 => $func::<21>($($arg),*),
            22 => $func::<22>($($arg),*),
            23 => $func::<23>($($arg),*),
            24 => $func::<24>($($arg),*),
            25 => $func::<25>($($arg),*),
            26 => $func::<26>($($arg),*),
            27 => $func::<27>($($arg),*),
            28 => $func::<28>($($arg),*),
            29 => $func::<29>($($arg),*),
            30 => $func::<30>($($arg),*),
            31 => $func::<31>($($arg),*),
            _ => unreachable!("invalid p32 exponent size"),
        }
    };
}

macro_rules! match_p64_es {
    ($es:expr, $func:ident $(, $arg:expr)* $(,)?) => {
        match $es {
            0 => $func::<0>($($arg),*), 1 => $func::<1>($($arg),*),
            2 => $func::<2>($($arg),*), 3 => $func::<3>($($arg),*),
            4 => $func::<4>($($arg),*), 5 => $func::<5>($($arg),*),
            6 => $func::<6>($($arg),*), 7 => $func::<7>($($arg),*),
            8 => $func::<8>($($arg),*), 9 => $func::<9>($($arg),*),
            10 => $func::<10>($($arg),*), 11 => $func::<11>($($arg),*),
            12 => $func::<12>($($arg),*), 13 => $func::<13>($($arg),*),
            14 => $func::<14>($($arg),*), 15 => $func::<15>($($arg),*),
            16 => $func::<16>($($arg),*), 17 => $func::<17>($($arg),*),
            18 => $func::<18>($($arg),*), 19 => $func::<19>($($arg),*),
            20 => $func::<20>($($arg),*), 21 => $func::<21>($($arg),*),
            22 => $func::<22>($($arg),*), 23 => $func::<23>($($arg),*),
            24 => $func::<24>($($arg),*), 25 => $func::<25>($($arg),*),
            26 => $func::<26>($($arg),*), 27 => $func::<27>($($arg),*),
            28 => $func::<28>($($arg),*), 29 => $func::<29>($($arg),*),
            30 => $func::<30>($($arg),*), 31 => $func::<31>($($arg),*),
            32 => $func::<32>($($arg),*), 33 => $func::<33>($($arg),*),
            34 => $func::<34>($($arg),*), 35 => $func::<35>($($arg),*),
            36 => $func::<36>($($arg),*), 37 => $func::<37>($($arg),*),
            38 => $func::<38>($($arg),*), 39 => $func::<39>($($arg),*),
            40 => $func::<40>($($arg),*), 41 => $func::<41>($($arg),*),
            42 => $func::<42>($($arg),*), 43 => $func::<43>($($arg),*),
            44 => $func::<44>($($arg),*), 45 => $func::<45>($($arg),*),
            46 => $func::<46>($($arg),*), 47 => $func::<47>($($arg),*),
            48 => $func::<48>($($arg),*), 49 => $func::<49>($($arg),*),
            50 => $func::<50>($($arg),*), 51 => $func::<51>($($arg),*),
            52 => $func::<52>($($arg),*), 53 => $func::<53>($($arg),*),
            54 => $func::<54>($($arg),*), 55 => $func::<55>($($arg),*),
            56 => $func::<56>($($arg),*), 57 => $func::<57>($($arg),*),
            58 => $func::<58>($($arg),*), 59 => $func::<59>($($arg),*),
            60 => $func::<60>($($arg),*), 61 => $func::<61>($($arg),*),
            62 => $func::<62>($($arg),*), 63 => $func::<63>($($arg),*),
            _ => unreachable!("invalid p64 exponent size"),
        }
    };
}

mod arena;
mod host;
mod numeric;
mod show;
mod text;
mod value;

pub(crate) use arena::*;
pub use {host::*, numeric::*, show::*, text::*, value::*};

#[cfg(test)]
mod tests;
