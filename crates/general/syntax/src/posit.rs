use fast_posit::{Posit, RoundFrom};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositSpec {
    pub nbits: u8,
    pub es: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositLiteral {
    pub spec: PositSpec,
    pub bits: u64,
}

impl PositSpec {
    pub fn new(nbits: u8, es: u8) -> Option<Self> {
        match nbits {
            32 if es < 32 => Some(Self { nbits, es }),
            64 if es < 64 => Some(Self { nbits, es }),
            _ => None,
        }
    }

    pub fn default_for_width(nbits: u8) -> Option<Self> {
        Self::new(nbits, 2)
    }

    pub fn type_name(&self) -> String {
        if self.es == 2 {
            format!("Posit{}", self.nbits)
        } else {
            format!("Posit{}e{}", self.nbits, self.es)
        }
    }

    pub fn literal_postfix(&self) -> String {
        if self.es == 2 {
            format!("p{}", self.nbits)
        } else {
            format!("p{}e{}", self.nbits, self.es)
        }
    }
}

pub fn parse_posit_number_type_postfix(s: &str) -> Option<(PositSpec, usize)> {
    let rest = s.strip_prefix('p')?;
    let (nbits, width_len, rest) = if let Some(rest) = rest.strip_prefix("32") {
        (32, 2, rest)
    } else if let Some(rest) = rest.strip_prefix("64") {
        (64, 2, rest)
    } else {
        return None;
    };
    let (spec, suffix_len) = parse_posit_suffix(nbits, rest)?;
    Some((spec, 1 + width_len + suffix_len))
}

pub fn parse_posit_type_name(name: &str) -> Option<PositSpec> {
    let rest = name.strip_prefix("Posit")?;
    let (nbits, rest) = if let Some(rest) = rest.strip_prefix("32") {
        (32, rest)
    } else if let Some(rest) = rest.strip_prefix("64") {
        (64, rest)
    } else {
        return None;
    };
    let (spec, consumed) = parse_posit_suffix(nbits, rest)?;
    (consumed == rest.len()).then_some(spec)
}

fn parse_posit_suffix(nbits: u8, rest: &str) -> Option<(PositSpec, usize)> {
    if rest.is_empty() {
        return PositSpec::default_for_width(nbits).map(|spec| (spec, 0));
    }

    let digits = rest.strip_prefix('e')?;
    let (es, digit_len) = parse_decimal_u8(digits)?;
    let spec = PositSpec::new(nbits, es)?;
    Some((spec, 1 + digit_len))
}

fn parse_decimal_u8(s: &str) -> Option<(u8, usize)> {
    let bytes = s.as_bytes();
    let first = *bytes.first()?;
    if first == b'0' {
        return Some((0, 1));
    }
    if !first.is_ascii_digit() {
        return None;
    }

    let mut value: u16 = 0;
    let mut len = 0;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
        if value > u8::MAX as u16 {
            return None;
        }
        len += 1;
    }
    Some((value as u8, len))
}

pub fn round_f64_to_posit_literal(spec: PositSpec, value: f64) -> PositLiteral {
    match spec.nbits {
        32 => round_p32_by_es(spec, value),
        64 => round_p64_by_es(spec, value),
        _ => unreachable!("invalid posit width"),
    }
}

fn round_p32<const ES: u32>(spec: PositSpec, value: f64) -> PositLiteral {
    let posit = Posit::<32, ES, i64>::round_from(value);
    PositLiteral {
        spec,
        bits: posit.to_bits() as u32 as u64,
    }
}

fn round_p64<const ES: u32>(spec: PositSpec, value: f64) -> PositLiteral {
    let posit = Posit::<64, ES, i128>::round_from(value);
    PositLiteral {
        spec,
        bits: posit.to_bits() as u64,
    }
}

macro_rules! match_p32_es {
    ($es:expr, $spec:expr, $value:expr) => {
        match $es {
            0 => round_p32::<0>($spec, $value),
            1 => round_p32::<1>($spec, $value),
            2 => round_p32::<2>($spec, $value),
            3 => round_p32::<3>($spec, $value),
            4 => round_p32::<4>($spec, $value),
            5 => round_p32::<5>($spec, $value),
            6 => round_p32::<6>($spec, $value),
            7 => round_p32::<7>($spec, $value),
            8 => round_p32::<8>($spec, $value),
            9 => round_p32::<9>($spec, $value),
            10 => round_p32::<10>($spec, $value),
            11 => round_p32::<11>($spec, $value),
            12 => round_p32::<12>($spec, $value),
            13 => round_p32::<13>($spec, $value),
            14 => round_p32::<14>($spec, $value),
            15 => round_p32::<15>($spec, $value),
            16 => round_p32::<16>($spec, $value),
            17 => round_p32::<17>($spec, $value),
            18 => round_p32::<18>($spec, $value),
            19 => round_p32::<19>($spec, $value),
            20 => round_p32::<20>($spec, $value),
            21 => round_p32::<21>($spec, $value),
            22 => round_p32::<22>($spec, $value),
            23 => round_p32::<23>($spec, $value),
            24 => round_p32::<24>($spec, $value),
            25 => round_p32::<25>($spec, $value),
            26 => round_p32::<26>($spec, $value),
            27 => round_p32::<27>($spec, $value),
            28 => round_p32::<28>($spec, $value),
            29 => round_p32::<29>($spec, $value),
            30 => round_p32::<30>($spec, $value),
            31 => round_p32::<31>($spec, $value),
            _ => unreachable!("invalid p32 exponent size"),
        }
    };
}

macro_rules! match_p64_es {
    ($es:expr, $spec:expr, $value:expr) => {
        match $es {
            0 => round_p64::<0>($spec, $value),
            1 => round_p64::<1>($spec, $value),
            2 => round_p64::<2>($spec, $value),
            3 => round_p64::<3>($spec, $value),
            4 => round_p64::<4>($spec, $value),
            5 => round_p64::<5>($spec, $value),
            6 => round_p64::<6>($spec, $value),
            7 => round_p64::<7>($spec, $value),
            8 => round_p64::<8>($spec, $value),
            9 => round_p64::<9>($spec, $value),
            10 => round_p64::<10>($spec, $value),
            11 => round_p64::<11>($spec, $value),
            12 => round_p64::<12>($spec, $value),
            13 => round_p64::<13>($spec, $value),
            14 => round_p64::<14>($spec, $value),
            15 => round_p64::<15>($spec, $value),
            16 => round_p64::<16>($spec, $value),
            17 => round_p64::<17>($spec, $value),
            18 => round_p64::<18>($spec, $value),
            19 => round_p64::<19>($spec, $value),
            20 => round_p64::<20>($spec, $value),
            21 => round_p64::<21>($spec, $value),
            22 => round_p64::<22>($spec, $value),
            23 => round_p64::<23>($spec, $value),
            24 => round_p64::<24>($spec, $value),
            25 => round_p64::<25>($spec, $value),
            26 => round_p64::<26>($spec, $value),
            27 => round_p64::<27>($spec, $value),
            28 => round_p64::<28>($spec, $value),
            29 => round_p64::<29>($spec, $value),
            30 => round_p64::<30>($spec, $value),
            31 => round_p64::<31>($spec, $value),
            32 => round_p64::<32>($spec, $value),
            33 => round_p64::<33>($spec, $value),
            34 => round_p64::<34>($spec, $value),
            35 => round_p64::<35>($spec, $value),
            36 => round_p64::<36>($spec, $value),
            37 => round_p64::<37>($spec, $value),
            38 => round_p64::<38>($spec, $value),
            39 => round_p64::<39>($spec, $value),
            40 => round_p64::<40>($spec, $value),
            41 => round_p64::<41>($spec, $value),
            42 => round_p64::<42>($spec, $value),
            43 => round_p64::<43>($spec, $value),
            44 => round_p64::<44>($spec, $value),
            45 => round_p64::<45>($spec, $value),
            46 => round_p64::<46>($spec, $value),
            47 => round_p64::<47>($spec, $value),
            48 => round_p64::<48>($spec, $value),
            49 => round_p64::<49>($spec, $value),
            50 => round_p64::<50>($spec, $value),
            51 => round_p64::<51>($spec, $value),
            52 => round_p64::<52>($spec, $value),
            53 => round_p64::<53>($spec, $value),
            54 => round_p64::<54>($spec, $value),
            55 => round_p64::<55>($spec, $value),
            56 => round_p64::<56>($spec, $value),
            57 => round_p64::<57>($spec, $value),
            58 => round_p64::<58>($spec, $value),
            59 => round_p64::<59>($spec, $value),
            60 => round_p64::<60>($spec, $value),
            61 => round_p64::<61>($spec, $value),
            62 => round_p64::<62>($spec, $value),
            63 => round_p64::<63>($spec, $value),
            _ => unreachable!("invalid p64 exponent size"),
        }
    };
}

fn round_p32_by_es(spec: PositSpec, value: f64) -> PositLiteral {
    match_p32_es!(spec.es, spec, value)
}

fn round_p64_by_es(spec: PositSpec, value: f64) -> PositLiteral {
    match_p64_es!(spec.es, spec, value)
}
