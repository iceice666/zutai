#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

impl NumberType {
    pub fn parse(run: &str) -> Option<Self> {
        match run {
            "i8" => Some(Self::I8),
            "i16" => Some(Self::I16),
            "i32" => Some(Self::I32),
            "i64" => Some(Self::I64),
            "u8" => Some(Self::U8),
            "u16" => Some(Self::U16),
            "u32" => Some(Self::U32),
            "u64" => Some(Self::U64),
            "f32" => Some(Self::F32),
            "f64" => Some(Self::F64),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    pub fn is_unsigned(self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::U32 | Self::U64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostfixCheck {
    None,
    Valid(NumberType),
    Unknown,
    IntOnFloatBody,
    UnsignedNegative,
}

pub fn classify_postfix(run: &str, has_sign: bool, has_frac_or_exp: bool) -> PostfixCheck {
    let Some(postfix) = NumberType::parse(run) else {
        return if run.is_empty() {
            PostfixCheck::None
        } else {
            PostfixCheck::Unknown
        };
    };

    if postfix.is_unsigned() && has_sign {
        PostfixCheck::UnsignedNegative
    } else if !postfix.is_float() && has_frac_or_exp {
        PostfixCheck::IntOnFloatBody
    } else {
        PostfixCheck::Valid(postfix)
    }
}
