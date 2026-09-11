//! Exact decimal arithmetic; storage is bounded by written digits, not exponents.
use std::cmp::Ordering;

#[derive(Clone, Debug)]
struct Signed {
    negative: bool,
    digits: Vec<u8>,
}
fn normalize(digits: &mut Vec<u8>) {
    let first = digits.iter().position(|d| *d != 0).unwrap_or(digits.len());
    digits.drain(..first);
}
fn cmp(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
fn add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0;
    for i in 0..a.len().max(b.len()) {
        let x = a.len().checked_sub(i + 1).map_or(0, |i| a[i]);
        let y = b.len().checked_sub(i + 1).map_or(0, |i| b[i]);
        let n = x + y + carry;
        out.push(n % 10);
        carry = n / 10;
    }
    if carry != 0 { out.push(carry); }
    out.reverse();
    out
}
// Precondition a >= b; decimal subtraction never expands an exponent.
fn subtract(a: &mut Vec<u8>, b: &[u8]) {
    let mut borrow = 0i16;
    let len = a.len();
    for i in 0..len {
        let y = b.len().checked_sub(i + 1).map_or(0, |i| b[i]);
        let n = i16::from(a[len-i-1]) - i16::from(y) - borrow;
        a[len-i-1] = n.rem_euclid(10) as u8;
        borrow = i16::from(n < 0);
    }
    normalize(a);
}
impl Signed {
    fn parse(text: &str) -> Self {
        let negative = text.starts_with('-');
        let text = text.strip_prefix(['-', '+']).unwrap_or(text);
        let mut digits = text.bytes().map(|b| b-b'0').collect();
        normalize(&mut digits);
        Self { negative: negative && !digits.is_empty(), digits }
    }
    fn count(count: usize) -> Self { Self::parse(&count.to_string()) }
    fn plus(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            Self { negative: self.negative, digits: add(&self.digits, &other.digits) }
        } else {
            let (larger, smaller) = if cmp(&self.digits, &other.digits).is_lt() { (other,self) } else { (self,other) };
            let mut digits = larger.digits.clone();
            subtract(&mut digits,&smaller.digits);
            Self { negative: larger.negative && !digits.is_empty(), digits }
        }
    }
    fn minus(&self, other: &Self) -> Self {
        self.plus(&Self { negative: !other.negative && !other.digits.is_empty(), digits: other.digits.clone() })
    }
    fn cmp(&self, other: &Self) -> Ordering {
        if self.negative != other.negative { return other.negative.cmp(&self.negative); }
        let order = cmp(&self.digits,&other.digits);
        if self.negative { order.reverse() } else { order }
    }
}

#[derive(Debug)]
pub(super) struct Exact {
    negative: bool,
    digits: Vec<u8>,
    exponent: Signed,
    order: Signed,
}
impl Exact {
    pub(super) fn parse(token: &str, cap: usize) -> Result<Self,String> {
        if token.len() > cap { return Err(format!("exact numeric operand exceeds {cap} source bytes")); }
        // All tokens originate from JsonNumber or compiler-checked instructions.
        let negative = token.starts_with('-');
        let unsigned = token.strip_prefix('-').unwrap_or(token);
        let (coefficient, written) = unsigned.split_once(['e','E']).unwrap_or((unsigned,"0"));
        let fraction = coefficient.split_once('.').map_or(0, |(_,f)|f.len());
        let mut digits: Vec<u8> = coefficient.bytes().filter(|b|*b != b'.').map(|b|b-b'0').collect();
        normalize(&mut digits);
        let mut trailing = 0;
        while digits.last() == Some(&0) { digits.pop(); trailing += 1; }
        let exponent = if digits.is_empty() { Signed::count(0) } else { Signed::parse(written).minus(&Signed::count(fraction)).plus(&Signed::count(trailing)) };
        let order = exponent.plus(&Signed::count(digits.len()));
        Ok(Self { negative: negative && !digits.is_empty(), digits, exponent, order })
    }
    pub(super) fn integral(&self) -> bool { !self.exponent.negative }
    pub(super) fn compare(&self, other: &Self) -> Ordering {
        if self.negative != other.negative { return other.negative.cmp(&self.negative); }
        let mut order = match (self.digits.is_empty(),other.digits.is_empty()) {
            (true,true) => Ordering::Equal,
            (true,false) => Ordering::Less,
            (false,true) => Ordering::Greater,
            (false,false) => self.order.cmp(&other.order),
        };
        if order.is_eq() {
            for i in 0..self.digits.len().max(other.digits.len()) {
                order = self.digits.get(i).unwrap_or(&0).cmp(other.digits.get(i).unwrap_or(&0));
                if !order.is_eq() { break; }
            }
        }
        if self.negative { order.reverse() } else { order }
    }
    pub(super) fn multiple_of(&self, divisor: &Self, work: &mut usize) -> Result<bool,String> {
        if self.digits.is_empty() { return Ok(true); }
        let delta = self.exponent.minus(&divisor.exponent);
        // A normalized coefficient has no factor of ten; negative delta fails.
        if delta.negative { return Ok(false); }
        let mut denominator = divisor.digits.clone();
        // Cancel only available factors of 2 and 5 supplied by 10^delta.
        // Each division shrinks a written coefficient. Never materialize 10^delta.
        for factor in [2,5] {
            let mut used = 0usize;
            loop {
                if Signed::count(used).cmp(&delta).is_ge() { break; }
                if denominator.last().is_none_or(|last| last % factor != 0) { break; }
                divide_small(&mut denominator, factor, work)?;
                used += 1;
            }
        }
        if denominator == [1] { return Ok(true); }
        match cmp(&self.digits, &denominator) {
            Ordering::Less => return Ok(false),
            Ordering::Equal => return Ok(true),
            Ordering::Greater => {}
        }
        Ok(remainder(&self.digits,&denominator,work)?.is_empty())
    }
}
fn charge(work: &mut usize, amount: usize) -> Result<(),String> {
    *work = work.checked_sub(amount).ok_or_else(|| "exact numeric work exceeds its shared evaluation-step budget".to_owned())?;
    Ok(())
}
fn divide_small(digits: &mut Vec<u8>, divisor: u8, work: &mut usize) -> Result<(),String> {
    charge(work,digits.len())?;
    let mut remainder = 0;
    for digit in digits.iter_mut() {
        let value = remainder * 10 + *digit;
        *digit = value / divisor;
        remainder = value % divisor;
    }
    normalize(digits);
    Ok(())
}
fn remainder(digits: &[u8], divisor: &[u8], work: &mut usize) -> Result<Vec<u8>,String> {
    let mut remainder = Vec::with_capacity(divisor.len()+1);
    // The first divisor.len()-1 digits cannot require a subtraction.
    let prefix = divisor.len().saturating_sub(1).min(digits.len());
    charge(work, prefix)?;
    remainder.extend_from_slice(&digits[..prefix]);
    for digit in &digits[prefix..] {
        charge(work,1)?;
        if !remainder.is_empty() || *digit != 0 { remainder.push(*digit); }
        loop {
            charge(work,remainder.len().max(divisor.len()))?;
            if cmp(&remainder,divisor).is_lt() { break; }
            subtract(&mut remainder,divisor);
        }
    }
    Ok(remainder)
}
