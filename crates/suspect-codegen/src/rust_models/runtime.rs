//! Exact JSON values and independent absence/null for experimental models.
//!
//! These containers are not schema codecs. No numeric operation uses a float.

/// A present value which may be JSON null.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nullable<T> {
    /// The JSON null value.
    Null,
    /// A present, non-null value.
    Value(T),
}

/// An optional property, with absent and present-null kept distinct.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Presence<T> {
    /// The property is absent from the object.
    #[default]
    Absent,
    /// The property is present with the JSON null value.
    Null,
    /// The property is present with a non-null value.
    Value(T),
}

/// No JSON value can satisfy a false schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Never {}

/// Any JSON value, preserving exact numeric tokens.
pub type JsonValue = Nullable<JsonNonNullValue>;

/// The non-null part of the JSON domain; arrays and objects may contain null.
#[derive(Debug, Clone)]
pub enum JsonNonNullValue {
    /// A JSON Boolean.
    Bool(bool),
    /// A Unicode JSON string.
    String(std::string::String),
    /// An exact JSON decimal token.
    Number(JsonNumber),
    /// A JSON array.
    Array(std::vec::Vec<JsonValue>),
    /// A JSON object; key order is not a JSON semantic constraint.
    Object(std::collections::BTreeMap<std::string::String, JsonValue>),
}

/// Invalid JSON number syntax or a non-integral integer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberError;

impl std::fmt::Display for NumberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("expected a JSON number token (and an integral value for JsonInteger)")
    }
}
impl std::error::Error for NumberError {}

/// An exact, validated JSON number token, including arbitrarily large exponents.
///
/// Parsing checks the JSON numeric grammar only, not source schema constraints.
/// Storage and parsing are linear in token length. The exponent is never expanded.
/// There is no floating point conversion or token-based mathematical equality.
#[derive(Debug, Clone)]
pub struct JsonNumber(std::string::String);

impl std::str::FromStr for JsonNumber {
    type Err = NumberError;
    fn from_str(token: &str) -> Result<Self, Self::Err> {
        parts(token)?;
        Ok(Self(token.into()))
    }
}
impl JsonNumber {
    /// Original token, including exponent spelling and negative zero.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A JSON number token whose mathematical value is integral.
///
/// `100e-2` and `1e400` are integers; `1e-400` is not. Source constraints
/// remain codec obligations. Equality is deliberately not defined by spelling.
#[derive(Debug, Clone)]
pub struct JsonInteger(std::string::String);

impl std::str::FromStr for JsonInteger {
    type Err = NumberError;
    fn from_str(token: &str) -> Result<Self, Self::Err> {
        let number = parts(token)?;
        if !number.integral() {
            return Err(NumberError);
        }
        Ok(Self(token.into()))
    }
}
impl JsonInteger {
    /// Original token, without rounding or normalizing its spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Exact conversion, or `None` when the mathematical value is out of range.
    #[must_use]
    pub fn to_i128(&self) -> Option<i128> {
        let number = parts(&self.0).ok()?;
        let magnitude = number.magnitude()?;
        if number.negative && magnitude != 0 {
            if magnitude == (i128::MAX as u128) + 1 {
                Some(i128::MIN)
            } else {
                i128::try_from(magnitude).ok()?.checked_neg()
            }
        } else {
            i128::try_from(magnitude).ok()
        }
    }
    /// Exact unsigned conversion; every spelling of mathematical zero is zero.
    #[must_use]
    pub fn to_u128(&self) -> Option<u128> {
        let number = parts(&self.0).ok()?;
        let magnitude = number.magnitude()?;
        (!number.negative || magnitude == 0).then_some(magnitude)
    }
}

struct Parts {
    negative: bool,
    digits: std::string::String,
    // Saturation is symbolic: any magnitude beyond i128 exceeds any possible
    // in-memory token length. Its sign suffices for integrality and conversion.
    shift: i128,
}
impl Parts {
    fn zero(&self) -> bool {
        self.digits.bytes().all(|digit| digit == b'0')
    }
    fn integral(&self) -> bool {
        self.zero()
            || self.shift >= 0
            || self.shift.unsigned_abs()
                <= self.digits.bytes().rev().take_while(|d| *d == b'0').count() as u128
    }
    fn magnitude(&self) -> Option<u128> {
        if self.zero() {
            return Some(0);
        }
        if !self.integral() {
            return None;
        }
        let mut digits = self.digits.trim_start_matches('0');
        if self.shift < 0 {
            let remove = usize::try_from(self.shift.unsigned_abs()).ok()?;
            digits = &digits[..digits.len().checked_sub(remove)?];
        }
        let append = usize::try_from(self.shift.max(0)).ok()?;
        // u128 has at most 39 decimal digits. Never expand a huge exponent.
        if digits.len().checked_add(append)? > 39 {
            return None;
        }
        let mut value = 0_u128;
        for digit in digits.bytes() {
            value = value
                .checked_mul(10)?
                .checked_add(u128::from(digit - b'0'))?;
        }
        for _ in 0..append {
            value = value.checked_mul(10)?;
        }
        Some(value)
    }
}

// RFC 8259 number grammar. Whitespace, a leading '+', leading zeroes, NaN,
// Infinity and incomplete fractions/exponents are rejected. Decimal digits
// and the exponent are scanned once; no recursion or exponent-sized allocation.
fn parts(token: &str) -> Result<Parts, NumberError> {
    let bytes = token.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let mut index = usize::from(negative);
    let start = index;
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return Err(NumberError),
    }
    let mut digits = token[start..index].to_owned();
    let mut fraction = 0_usize;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        fraction = index - start;
        if fraction == 0 {
            return Err(NumberError);
        }
        digits.push_str(&token[start..index]);
    }
    let mut exponent = 0_i128;
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        let exponent_negative = bytes.get(index) == Some(&b'-');
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while let Some(digit) = bytes.get(index).filter(|digit| digit.is_ascii_digit()) {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i128::from(*digit - b'0'));
            index += 1;
        }
        if index == start {
            return Err(NumberError);
        }
        if exponent_negative {
            exponent = -exponent;
        }
    }
    if index != bytes.len() {
        return Err(NumberError);
    }
    Ok(Parts {
        negative,
        digits,
        shift: exponent.saturating_sub(fraction as i128),
    })
}

/// An extra-field key duplicates a declared wire property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraFieldError(pub std::string::String);
impl std::fmt::Display for ExtraFieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "extra field {:?} duplicates a declared wire property",
            self.0
        )
    }
}
impl std::error::Error for ExtraFieldError {}
