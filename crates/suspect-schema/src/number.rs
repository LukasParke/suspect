//! Owned, normalized decimal numbers for exact schema semantics.
//!
//! Values are sign × coefficient × 10^exponent. The exponent is a BigInt,
//! never a request to allocate exponent-sized storage. Decimal coefficients
//! have no leading/trailing zeroes, making comparison linear in their written
//! length. `num-bigint` belongs to the compiler/validator, and is not injected
//! into generated SDK packages.

use std::{cmp::Ordering, fmt};

use num_bigint::{BigInt, BigUint};
use num_traits::{One, ToPrimitive, Zero};

#[derive(Clone, Debug)]
pub(crate) struct ExactNumber {
    negative: bool,
    digits: Box<[u8]>,
    exponent: BigInt,
    /// exponent + coefficient length, cached for comparisons.
    order: BigInt,
    source: Box<str>,
}

#[derive(Debug)]
pub(crate) enum NumberError {
    ResourceLimit { cap: usize },
    NonFiniteOrInvalid,
}

impl fmt::Display for NumberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit { cap } => {
                write!(f, "exact numeric operand exceeds {cap} source bytes")
            }
            Self::NonFiniteOrInvalid => f.write_str("expected a finite numeric value"),
        }
    }
}

impl ExactNumber {
    /// Normalization removes coefficient trailing zeroes, so only a negative
    /// remaining exponent can make the mathematical value fractional.
    pub(crate) fn is_integral(&self) -> bool {
        self.exponent >= BigInt::zero()
    }

    pub(crate) fn is_negative(&self) -> bool {
        self.negative
    }

    /// Converts only nonnegative, integral, addressable values. No exponent
    /// expansion occurs: at most `usize::MAX.ilog10()` multiplications fit.
    pub(crate) fn to_usize(&self) -> Option<usize> {
        if self.negative {
            return None;
        }
        if self.digits.is_empty() {
            return Some(0);
        }
        let power = self.exponent.to_u32()?;
        if power > usize::MAX.ilog10() {
            return None;
        }
        let coefficient = self.digits.iter().try_fold(0usize, |value, &digit| {
            value
                .checked_mul(10)?
                .checked_add(usize::from(digit - b'0'))
        })?;
        coefficient.checked_mul(10usize.checked_pow(power)?)
    }

    pub(crate) fn is_positive(&self) -> bool {
        !self.negative && !self.digits.is_empty()
    }
    pub(crate) fn parse(source: &[u8], cap: usize) -> Result<Self, NumberError> {
        if source.len() > cap {
            return Err(NumberError::ResourceLimit { cap });
        }
        let source_text =
            std::str::from_utf8(source).map_err(|_| NumberError::NonFiniteOrInvalid)?;
        let (negative, unsigned) = match source.first() {
            Some(b'-') => (true, &source[1..]),
            Some(b'+') => (false, &source[1..]),
            _ => (false, source),
        };
        // YAML's core integer spellings retain their mathematical value.
        let radix = match unsigned.get(..2) {
            Some(b"0x" | b"0X") => Some(16),
            Some(b"0o" | b"0O") => Some(8),
            _ => None,
        };
        let (mut digits, mut exponent) = if let Some(radix) = radix {
            let value = BigUint::parse_bytes(&unsigned[2..], radix)
                .ok_or(NumberError::NonFiniteOrInvalid)?;
            (value.to_str_radix(10).into_bytes(), BigInt::zero())
        } else {
            let split = unsigned.iter().position(|b| matches!(b, b'e' | b'E'));
            let (mantissa, power) = split.map_or((unsigned, None), |i| {
                (&unsigned[..i], Some(&unsigned[i + 1..]))
            });
            let exponent = if let Some(power) = power {
                let power_digits = match power.first() {
                    Some(b'-' | b'+') => &power[1..],
                    _ => power,
                };
                if power_digits.is_empty() || !power_digits.iter().all(u8::is_ascii_digit) {
                    return Err(NumberError::NonFiniteOrInvalid);
                }
                BigInt::parse_bytes(power, 10).ok_or(NumberError::NonFiniteOrInvalid)?
            } else {
                BigInt::zero()
            };
            let mut digits = Vec::with_capacity(mantissa.len());
            let mut point = None;
            for &byte in mantissa {
                if byte.is_ascii_digit() {
                    digits.push(byte);
                } else if byte == b'.' && point.is_none() {
                    point = Some(digits.len());
                } else {
                    return Err(NumberError::NonFiniteOrInvalid);
                }
            }
            if digits.is_empty() {
                return Err(NumberError::NonFiniteOrInvalid);
            }
            let fraction = point.map_or(0, |p| digits.len() - p);
            (digits, exponent - BigInt::from(fraction))
        };
        let first = digits.iter().position(|&b| b != b'0');
        let negative = if let Some(first) = first {
            let end = digits.iter().rposition(|&b| b != b'0').unwrap() + 1;
            exponent += BigInt::from(digits.len() - end);
            digits = digits[first..end].to_vec();
            negative
        } else {
            digits.clear();
            exponent = BigInt::zero();
            false
        };
        let order = &exponent + BigInt::from(digits.len());
        Ok(Self {
            negative,
            digits: digits.into_boxed_slice(),
            exponent,
            order,
            source: source_text.into(),
        })
    }

    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let order = match (self.digits.is_empty(), other.digits.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.order.cmp(&other.order).then_with(|| {
                // Equal decimal order aligns the leading digits. Missing
                // coefficient digits are implicit zeroes; never expand 10^e.
                (0..self.digits.len().max(other.digits.len()))
                    .map(|i| {
                        self.digits
                            .get(i)
                            .unwrap_or(&b'0')
                            .cmp(other.digits.get(i).unwrap_or(&b'0'))
                    })
                    .find(|ord| *ord != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
            }),
        };
        if self.negative {
            order.reverse()
        } else {
            order
        }
    }
}

/// Pre-factored positive divisor. Only its written coefficient is expanded,
/// with at most a constant-factor growth for YAML radix integer spellings.
pub(crate) struct Divisor {
    number: ExactNumber,
    coprime: BigUint,
    twos: u64,
    fives: u64,
}

impl Divisor {
    pub(crate) fn new(number: ExactNumber) -> Self {
        debug_assert!(number.is_positive());
        let mut coprime = BigUint::parse_bytes(&number.digits, 10).unwrap();
        let twos = coprime.trailing_zeros().unwrap_or(0);
        coprime >>= twos;
        let mut fives = 0;
        while (&coprime % 5u8).is_zero() {
            coprime /= 5u8;
            fives += 1;
        }
        Self {
            number,
            coprime,
            twos,
            fives,
        }
    }

    pub(crate) fn contains(&self, value: &ExactNumber) -> bool {
        if value.digits.is_empty() {
            return true;
        }
        let delta = &value.exponent - &self.number.exponent;
        if delta < BigInt::zero() {
            // Both coefficients lack trailing zeroes, so the quotient would
            // require a power of ten to divide a coefficient ending nonzero.
            return false;
        }
        // Remove from the denominator only the powers of 2 and 5 supplied
        // by 10^delta. Saturation is safe: factor counts are bounded by the
        // coefficient's written length, even for an astronomical delta.
        let supplied = delta.to_u64().unwrap_or(u64::MAX);
        let missing_twos = self.twos.saturating_sub(supplied);
        let missing_fives = self.fives.saturating_sub(supplied);
        if self.coprime.is_one() && missing_twos == 0 && missing_fives == 0 {
            return true;
        }
        let mut remaining = &self.coprime << missing_twos;
        if missing_fives != 0 {
            remaining *= num_traits::Pow::pow(BigUint::from(5u8), missing_fives);
        }
        let numerator = BigUint::parse_bytes(&value.digits, 10).unwrap();
        (numerator % remaining).is_zero()
    }
}

impl fmt::Display for Divisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.number.fmt(f)
    }
}

impl fmt::Display for ExactNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}
