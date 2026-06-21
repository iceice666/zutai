use std::cmp::Ordering;

use fast_posit::{Posit, RoundInto};
use zutai_syntax::posit::{PositLiteral, PositSpec, round_f64_to_posit_literal};

use crate::EvalError;

#[derive(Clone, Copy)]
enum ArithmeticOp {
    Add,
    Sub,
    Mul,
    Div,
}

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
            32 => $func::<32>($($arg),*),
            33 => $func::<33>($($arg),*),
            34 => $func::<34>($($arg),*),
            35 => $func::<35>($($arg),*),
            36 => $func::<36>($($arg),*),
            37 => $func::<37>($($arg),*),
            38 => $func::<38>($($arg),*),
            39 => $func::<39>($($arg),*),
            40 => $func::<40>($($arg),*),
            41 => $func::<41>($($arg),*),
            42 => $func::<42>($($arg),*),
            43 => $func::<43>($($arg),*),
            44 => $func::<44>($($arg),*),
            45 => $func::<45>($($arg),*),
            46 => $func::<46>($($arg),*),
            47 => $func::<47>($($arg),*),
            48 => $func::<48>($($arg),*),
            49 => $func::<49>($($arg),*),
            50 => $func::<50>($($arg),*),
            51 => $func::<51>($($arg),*),
            52 => $func::<52>($($arg),*),
            53 => $func::<53>($($arg),*),
            54 => $func::<54>($($arg),*),
            55 => $func::<55>($($arg),*),
            56 => $func::<56>($($arg),*),
            57 => $func::<57>($($arg),*),
            58 => $func::<58>($($arg),*),
            59 => $func::<59>($($arg),*),
            60 => $func::<60>($($arg),*),
            61 => $func::<61>($($arg),*),
            62 => $func::<62>($($arg),*),
            63 => $func::<63>($($arg),*),
            _ => unreachable!("invalid p64 exponent size"),
        }
    };
}

macro_rules! match_p64_div_es {
    ($es:expr, $spec:expr, $lhs:expr, $rhs:expr) => {
        match $es {
            0 => div_p64_i64::<0>($spec, $lhs, $rhs),
            1 => div_p64_i64::<1>($spec, $lhs, $rhs),
            2 => div_p64_i64::<2>($spec, $lhs, $rhs),
            3 => div_p64_i64::<3>($spec, $lhs, $rhs),
            4 => div_p64_i64::<4>($spec, $lhs, $rhs),
            5 => div_p64_i64::<5>($spec, $lhs, $rhs),
            6 => div_p64_i64::<6>($spec, $lhs, $rhs),
            7 => div_p64_i64::<7>($spec, $lhs, $rhs),
            8 => div_p64_i64::<8>($spec, $lhs, $rhs),
            9 => div_p64_i64::<9>($spec, $lhs, $rhs),
            10 => div_p64_i64::<10>($spec, $lhs, $rhs),
            11 => div_p64_i64::<11>($spec, $lhs, $rhs),
            12 => div_p64_i64::<12>($spec, $lhs, $rhs),
            13 => div_p64_i64::<13>($spec, $lhs, $rhs),
            14 => div_p64_i64::<14>($spec, $lhs, $rhs),
            15 => div_p64_i64::<15>($spec, $lhs, $rhs),
            16 => div_p64_i64::<16>($spec, $lhs, $rhs),
            17 => div_p64_i64::<17>($spec, $lhs, $rhs),
            18 => div_p64_i64::<18>($spec, $lhs, $rhs),
            19 => div_p64_i64::<19>($spec, $lhs, $rhs),
            20 => div_p64_i64::<20>($spec, $lhs, $rhs),
            21 => div_p64_i64::<21>($spec, $lhs, $rhs),
            22 => div_p64_i64::<22>($spec, $lhs, $rhs),
            23 => div_p64_i64::<23>($spec, $lhs, $rhs),
            24 => div_p64_i64::<24>($spec, $lhs, $rhs),
            25 => div_p64_i64::<25>($spec, $lhs, $rhs),
            26 => div_p64_i64::<26>($spec, $lhs, $rhs),
            27 => div_p64_i64::<27>($spec, $lhs, $rhs),
            28 => div_p64_i64::<28>($spec, $lhs, $rhs),
            29 => div_p64_i64::<29>($spec, $lhs, $rhs),
            30 => div_p64_i64::<30>($spec, $lhs, $rhs),
            31 => div_p64_i64::<31>($spec, $lhs, $rhs),
            32 => div_p64_i64::<32>($spec, $lhs, $rhs),
            33 => div_p64_i64::<33>($spec, $lhs, $rhs),
            34 => div_p64_i64::<34>($spec, $lhs, $rhs),
            35 => div_p64_i64::<35>($spec, $lhs, $rhs),
            36 => div_p64_i64::<36>($spec, $lhs, $rhs),
            37 => div_p64_i64::<37>($spec, $lhs, $rhs),
            38 => div_p64_i64::<38>($spec, $lhs, $rhs),
            39 => div_p64_i64::<39>($spec, $lhs, $rhs),
            40 => div_p64_i64::<40>($spec, $lhs, $rhs),
            41 => div_p64_i64::<41>($spec, $lhs, $rhs),
            42 => div_p64_i64::<42>($spec, $lhs, $rhs),
            43 => div_p64_i64::<43>($spec, $lhs, $rhs),
            44 => div_p64_i64::<44>($spec, $lhs, $rhs),
            45 => div_p64_i64::<45>($spec, $lhs, $rhs),
            46 => div_p64_i64::<46>($spec, $lhs, $rhs),
            47 => div_p64_i64::<47>($spec, $lhs, $rhs),
            48 => div_p64_i64::<48>($spec, $lhs, $rhs),
            49 => div_p64_i64::<49>($spec, $lhs, $rhs),
            50 => div_p64_i64::<50>($spec, $lhs, $rhs),
            51 => div_p64_i64::<51>($spec, $lhs, $rhs),
            52 => div_p64_i64::<52>($spec, $lhs, $rhs),
            53 => div_p64_i64::<53>($spec, $lhs, $rhs),
            54 => div_p64_i64::<54>($spec, $lhs, $rhs),
            55 => div_p64_i64::<55>($spec, $lhs, $rhs),
            56 => div_p64_i64::<56>($spec, $lhs, $rhs),
            57..=63 => div_p64_via_f64($spec, $lhs, $rhs),
            _ => unreachable!("invalid p64 exponent size"),
        }
    };
}

pub(crate) fn posit_add(lhs: PositLiteral, rhs: PositLiteral) -> Result<PositLiteral, EvalError> {
    posit_binop(lhs, rhs, ArithmeticOp::Add)
}

pub(crate) fn posit_sub(lhs: PositLiteral, rhs: PositLiteral) -> Result<PositLiteral, EvalError> {
    posit_binop(lhs, rhs, ArithmeticOp::Sub)
}

pub(crate) fn posit_mul(lhs: PositLiteral, rhs: PositLiteral) -> Result<PositLiteral, EvalError> {
    posit_binop(lhs, rhs, ArithmeticOp::Mul)
}

pub(crate) fn posit_div(lhs: PositLiteral, rhs: PositLiteral) -> Result<PositLiteral, EvalError> {
    posit_binop(lhs, rhs, ArithmeticOp::Div)
}

pub(crate) fn posit_cmp(lhs: PositLiteral, rhs: PositLiteral) -> Result<Ordering, EvalError> {
    ensure_matching_specs(lhs, rhs)?;
    let spec = lhs.spec;
    Ok(match spec.nbits {
        32 => match_p32_es!(spec.es, cmp_p32, lhs.bits, rhs.bits),
        64 => match_p64_es!(spec.es, cmp_p64, lhs.bits, rhs.bits),
        _ => unreachable!("invalid posit width"),
    })
}

pub(crate) fn format_posit(literal: PositLiteral) -> String {
    let value = match literal.spec.nbits {
        32 => match_p32_es!(literal.spec.es, to_f64_p32, literal.bits),
        64 => match_p64_es!(literal.spec.es, to_f64_p64, literal.bits),
        _ => unreachable!("invalid posit width"),
    };
    let mut out = format!("{value:?}");
    if value.is_finite() && out.ends_with(".0") {
        out.truncate(out.len() - 2);
    }
    out.push_str(&literal.spec.literal_postfix());
    out
}

fn posit_binop(
    lhs: PositLiteral,
    rhs: PositLiteral,
    op: ArithmeticOp,
) -> Result<PositLiteral, EvalError> {
    ensure_matching_specs(lhs, rhs)?;
    let spec = lhs.spec;
    Ok(match (spec.nbits, op) {
        (32, _) => match_p32_es!(spec.es, arith_p32, spec, lhs.bits, rhs.bits, op),
        (64, ArithmeticOp::Div) => match_p64_div_es!(spec.es, spec, lhs.bits, rhs.bits),
        (64, _) => match_p64_es!(spec.es, arith_p64, spec, lhs.bits, rhs.bits, op),
        _ => unreachable!("invalid posit width"),
    })
}

fn ensure_matching_specs(lhs: PositLiteral, rhs: PositLiteral) -> Result<(), EvalError> {
    if lhs.spec == rhs.spec {
        Ok(())
    } else {
        Err(EvalError::TypeMismatch {
            expected: "matching posit type",
            found: "different posit type",
        })
    }
}

fn arith_p32<const ES: u32>(
    spec: PositSpec,
    lhs_bits: u64,
    rhs_bits: u64,
    op: ArithmeticOp,
) -> PositLiteral {
    let lhs = Posit::<32, ES, i64>::from_bits(lhs_bits as u32 as i32 as i64);
    let rhs = Posit::<32, ES, i64>::from_bits(rhs_bits as u32 as i32 as i64);
    let value = match op {
        ArithmeticOp::Add => lhs + rhs,
        ArithmeticOp::Sub => lhs - rhs,
        ArithmeticOp::Mul => lhs * rhs,
        ArithmeticOp::Div => lhs / rhs,
    };
    PositLiteral {
        spec,
        bits: value.to_bits() as u32 as u64,
    }
}

fn arith_p64<const ES: u32>(
    spec: PositSpec,
    lhs_bits: u64,
    rhs_bits: u64,
    op: ArithmeticOp,
) -> PositLiteral {
    let lhs = Posit::<64, ES, i128>::from_bits(lhs_bits as i64 as i128);
    let rhs = Posit::<64, ES, i128>::from_bits(rhs_bits as i64 as i128);
    let value = match op {
        ArithmeticOp::Add => lhs + rhs,
        ArithmeticOp::Sub => lhs - rhs,
        ArithmeticOp::Mul => lhs * rhs,
        ArithmeticOp::Div => lhs / rhs,
    };
    PositLiteral {
        spec,
        bits: value.to_bits() as u64,
    }
}

fn div_p64_i64<const ES: u32>(spec: PositSpec, lhs_bits: u64, rhs_bits: u64) -> PositLiteral {
    let lhs = Posit::<64, ES, i64>::from_bits(lhs_bits as i64);
    let rhs = Posit::<64, ES, i64>::from_bits(rhs_bits as i64);
    let value = lhs / rhs;
    PositLiteral {
        spec,
        bits: value.to_bits() as u64,
    }
}

fn div_p64_via_f64(spec: PositSpec, lhs_bits: u64, rhs_bits: u64) -> PositLiteral {
    let lhs = match_p64_es!(spec.es, to_f64_p64, lhs_bits);
    let rhs = match_p64_es!(spec.es, to_f64_p64, rhs_bits);
    round_f64_to_posit_literal(spec, lhs / rhs)
}

fn cmp_p32<const ES: u32>(lhs_bits: u64, rhs_bits: u64) -> Ordering {
    let lhs = Posit::<32, ES, i64>::from_bits(lhs_bits as u32 as i32 as i64);
    let rhs = Posit::<32, ES, i64>::from_bits(rhs_bits as u32 as i32 as i64);
    lhs.cmp(&rhs)
}

fn cmp_p64<const ES: u32>(lhs_bits: u64, rhs_bits: u64) -> Ordering {
    let lhs = Posit::<64, ES, i128>::from_bits(lhs_bits as i64 as i128);
    let rhs = Posit::<64, ES, i128>::from_bits(rhs_bits as i64 as i128);
    lhs.cmp(&rhs)
}

fn to_f64_p32<const ES: u32>(bits: u64) -> f64 {
    Posit::<32, ES, i64>::from_bits(bits as u32 as i32 as i64).round_into()
}

fn to_f64_p64<const ES: u32>(bits: u64) -> f64 {
    Posit::<64, ES, i128>::from_bits(bits as i64 as i128).round_into()
}
