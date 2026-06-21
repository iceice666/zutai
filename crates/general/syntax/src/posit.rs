use fast_posit::{Posit, RoundFrom};
use num_bigint::BigUint;
use num_traits::{One, Zero};

use std::cmp::Ordering;

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

const MAX_DECIMAL_SCALE: i128 = 1_000_000;

struct DecimalLiteral {
    negative: bool,
    coefficient: BigUint,
    decimal_exponent: i128,
}

impl DecimalLiteral {
    fn parse(literal: &str) -> Option<Self> {
        let bytes = literal.as_bytes();
        let mut index = 0;
        let negative = bytes.first() == Some(&b'-');
        if negative {
            index = 1;
        }

        let mut coefficient = BigUint::zero();
        let mut digit_count = 0usize;
        while let Some(&byte) = bytes.get(index) {
            if !byte.is_ascii_digit() {
                break;
            }
            coefficient *= 10u8;
            coefficient += byte - b'0';
            digit_count += 1;
            index += 1;
        }
        if digit_count == 0 {
            return None;
        }

        let mut fractional_digits = 0i128;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let fractional_start = index;
            while let Some(&byte) = bytes.get(index) {
                if !byte.is_ascii_digit() {
                    break;
                }
                coefficient *= 10u8;
                coefficient += byte - b'0';
                fractional_digits = fractional_digits.checked_add(1)?;
                index += 1;
            }
            if index == fractional_start {
                return None;
            }
        }

        let mut exponent = 0i128.checked_sub(fractional_digits)?;
        if matches!(bytes.get(index), Some(b'e' | b'E')) {
            index += 1;
            let exponent_negative = match bytes.get(index) {
                Some(b'+') => {
                    index += 1;
                    false
                }
                Some(b'-') => {
                    index += 1;
                    true
                }
                _ => false,
            };

            let exponent_start = index;
            let mut explicit_exponent = 0i128;
            while let Some(&byte) = bytes.get(index) {
                if !byte.is_ascii_digit() {
                    break;
                }
                explicit_exponent = explicit_exponent
                    .saturating_mul(10)
                    .saturating_add(i128::from(byte - b'0'));
                index += 1;
            }
            if index == exponent_start {
                return None;
            }

            let signed_exponent = if exponent_negative {
                explicit_exponent.saturating_neg()
            } else {
                explicit_exponent
            };
            exponent = exponent.saturating_add(signed_exponent);
        }

        if index != bytes.len() {
            return None;
        }

        Some(Self {
            negative,
            coefficient,
            decimal_exponent: exponent,
        })
    }
}

struct FractionBits {
    remainder: BigUint,
    denominator: BigUint,
}

impl FractionBits {
    fn new(numerator: &BigUint, denominator: &BigUint, binary_floor: i128) -> Self {
        if binary_floor >= 0 {
            let denominator = denominator << (binary_floor as usize);
            let remainder = numerator.clone() - &denominator;
            Self {
                remainder,
                denominator,
            }
        } else {
            let numerator = numerator << ((-binary_floor) as usize);
            let remainder = numerator - denominator;
            Self {
                remainder,
                denominator: denominator.clone(),
            }
        }
    }

    fn next(&mut self) -> bool {
        self.remainder <<= 1usize;
        if self.remainder.cmp(&self.denominator) != Ordering::Less {
            self.remainder -= &self.denominator;
            true
        } else {
            false
        }
    }

    fn has_remaining(&self) -> bool {
        !self.remainder.is_zero()
    }
}

struct PositBitSink {
    payload: u64,
    seen: usize,
    precision: usize,
    round_bit: bool,
    sticky: bool,
}

impl PositBitSink {
    fn new(precision: usize) -> Self {
        Self {
            payload: 0,
            seen: 0,
            precision,
            round_bit: false,
            sticky: false,
        }
    }

    fn push(&mut self, bit: bool) {
        if self.seen < self.precision {
            self.payload = (self.payload << 1) | u64::from(bit);
        } else if self.seen == self.precision {
            self.round_bit = bit;
        } else if bit {
            self.sticky = true;
        }
        self.seen += 1;
    }

    fn needs_round_bit(&self) -> bool {
        self.seen <= self.precision
    }

    fn rounded_payload(self) -> u64 {
        let mut payload = self.payload;
        if self.round_bit && (self.sticky || payload & 1 != 0) {
            payload += 1;
        }
        payload
    }
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

pub fn parse_posit_literal(spec: PositSpec, literal: &str) -> Option<PositLiteral> {
    let decimal = DecimalLiteral::parse(literal)?;
    let bits = if decimal.coefficient.is_zero() {
        0
    } else {
        round_decimal_to_posit_bits(spec, decimal)?
    };
    Some(PositLiteral { spec, bits })
}

fn round_decimal_to_posit_bits(spec: PositSpec, decimal: DecimalLiteral) -> Option<u64> {
    let max_binary_exponent = max_finite_binary_exponent(spec);
    let max_payload = max_positive_payload(spec);
    let negative = decimal.negative;

    if decimal.decimal_exponent > MAX_DECIMAL_SCALE {
        if decimal_positive_scale_exceeds_max(
            &decimal.coefficient,
            decimal.decimal_exponent,
            max_binary_exponent,
        ) {
            return Some(apply_sign(spec.nbits, negative, max_payload));
        }
        return None;
    }
    if decimal.decimal_exponent < -MAX_DECIMAL_SCALE {
        if decimal_negative_scale_below_min(
            &decimal.coefficient,
            decimal.decimal_exponent,
            -max_binary_exponent,
        ) {
            return Some(apply_sign(spec.nbits, negative, 1));
        }
        return None;
    }

    let (numerator, denominator, binary_exponent) = scaled_decimal_ratio(decimal)?;
    let ratio_binary_floor = floor_log2_ratio(&numerator, &denominator);
    let binary_floor = ratio_binary_floor.checked_add(binary_exponent)?;

    let payload = if binary_floor >= max_binary_exponent {
        max_payload
    } else if binary_floor < -max_binary_exponent {
        1
    } else {
        let fraction = FractionBits::new(&numerator, &denominator, ratio_binary_floor);
        round_normalized_to_payload(spec, binary_floor, fraction)
    };

    Some(apply_sign(spec.nbits, negative, payload))
}

fn scaled_decimal_ratio(decimal: DecimalLiteral) -> Option<(BigUint, BigUint, i128)> {
    if decimal.decimal_exponent >= 0 {
        let scale = pow5(decimal.decimal_exponent)?;
        Some((
            decimal.coefficient * scale,
            BigUint::one(),
            decimal.decimal_exponent,
        ))
    } else {
        let scale = pow5(-decimal.decimal_exponent)?;
        Some((decimal.coefficient, scale, decimal.decimal_exponent))
    }
}

fn pow5(exponent: i128) -> Option<BigUint> {
    if exponent == 0 {
        return Some(BigUint::one());
    }
    if !(0..=MAX_DECIMAL_SCALE).contains(&exponent) {
        return None;
    }
    Some(BigUint::from(5u8).pow(exponent as u32))
}

fn floor_log2_ratio(numerator: &BigUint, denominator: &BigUint) -> i128 {
    let candidate = numerator.bits() as i128 - denominator.bits() as i128;
    if candidate >= 0 {
        let scaled_denominator = denominator << (candidate as usize);
        if numerator.cmp(&scaled_denominator) == Ordering::Less {
            candidate - 1
        } else {
            candidate
        }
    } else {
        let scaled_numerator = numerator << ((-candidate) as usize);
        if scaled_numerator.cmp(denominator) == Ordering::Less {
            candidate - 1
        } else {
            candidate
        }
    }
}

fn round_normalized_to_payload(
    spec: PositSpec,
    binary_floor: i128,
    mut fraction: FractionBits,
) -> u64 {
    let precision = usize::from(spec.nbits - 1);
    let max_payload = max_positive_payload(spec);
    let exponent_unit = 1i128 << spec.es;
    let regime = binary_floor.div_euclid(exponent_unit);
    let exponent = binary_floor.rem_euclid(exponent_unit) as u128;
    let mut sink = PositBitSink::new(precision);

    if regime >= 0 {
        for _ in 0..=regime as usize {
            sink.push(true);
        }
        sink.push(false);
    } else {
        for _ in 0..(-regime) as usize {
            sink.push(false);
        }
        sink.push(true);
    }

    for bit in (0..usize::from(spec.es)).rev() {
        sink.push(((exponent >> bit) & 1) != 0);
    }

    while sink.needs_round_bit() {
        sink.push(fraction.next());
    }
    if fraction.has_remaining() {
        sink.sticky = true;
    }

    let payload = sink.rounded_payload();
    if payload == 0 {
        1
    } else {
        payload.min(max_payload)
    }
}

fn max_finite_binary_exponent(spec: PositSpec) -> i128 {
    i128::from(spec.nbits - 2) * (1i128 << spec.es)
}

fn max_positive_payload(spec: PositSpec) -> u64 {
    (1u64 << (spec.nbits - 1)) - 1
}

fn apply_sign(nbits: u8, negative: bool, payload: u64) -> u64 {
    if !negative {
        return payload;
    }

    let mask = if nbits == 64 {
        u64::MAX
    } else {
        (1u64 << nbits) - 1
    };
    (!payload).wrapping_add(1) & mask
}

fn decimal_positive_scale_exceeds_max(
    coefficient: &BigUint,
    decimal_exponent: i128,
    max_binary_exponent: i128,
) -> bool {
    let coefficient_floor = coefficient.bits() as i128 - 1;
    coefficient_floor.saturating_add(decimal_exponent.saturating_mul(3)) > max_binary_exponent
}

fn decimal_negative_scale_below_min(
    coefficient: &BigUint,
    decimal_exponent: i128,
    min_binary_exponent: i128,
) -> bool {
    let decimal_scale = decimal_exponent.saturating_neg();
    let coefficient_ceiling = coefficient.bits() as i128;
    coefficient_ceiling.saturating_sub(decimal_scale.saturating_mul(3)) < min_binary_exponent
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
