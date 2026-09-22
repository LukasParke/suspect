"""Exact normalized decimals; exponents stay symbolic and no global limits change."""
from __future__ import annotations
from collections.abc import Callable

def integer(text: str) -> int:
    negative = text.startswith('-')
    text = text.lstrip('+-')
    value = 0
    for at in range(0, len(text), 9):
        chunk = text[at:at + 9]
        value = value * 10 ** len(chunk) + int(chunk)
    return -value if negative else value

class Exact:
    __slots__ = ('sign', 'digits', 'exponent')
    def __init__(self, sign: int, digits: str, exponent: int) -> None:
        self.sign, self.digits, self.exponent = sign, digits, exponent
    @classmethod
    def parse(cls, token: str) -> Exact:
        negative = token.startswith('-')
        mantissa, _, exponent = token.lstrip('-').lower().partition('e')
        whole, _, fraction = mantissa.partition('.')
        digits = (whole + fraction).lstrip('0')
        if not digits:
            return cls(0, '0', 0)
        trimmed = digits.rstrip('0')
        shift = (integer(exponent) if exponent else 0) - len(fraction) + len(digits) - len(trimmed)
        return cls(-1 if negative else 1, trimmed, shift)
    def compare(self, other: Exact) -> int:
        if self.sign != other.sign:
            return (self.sign > other.sign) - (self.sign < other.sign)
        if self.sign == 0:
            return 0
        a, b = len(self.digits) + self.exponent, len(other.digits) + other.exponent
        if a != b:
            order = (a > b) - (a < b)
        else:
            length = max(len(self.digits), len(other.digits))
            left, right = self.digits.ljust(length, '0'), other.digits.ljust(length, '0')
            order = (left > right) - (left < right)
        return self.sign * order
    def integral(self) -> bool:
        return self.sign == 0 or self.exponent >= 0

def divisible(value: Exact, divisor: Exact, spend: Callable[[int], None]) -> bool:
    if value.sign == 0:
        return True
    shift = value.exponent - divisor.exponent
    # Normalized coefficients have no trailing zero; a negative shift cannot
    # divide exactly, regardless of the divisor's coefficient.
    if shift < 0:
        return False
    a, b = integer(value.digits), integer(divisor.digits)
    spend(len(value.digits) + len(divisor.digits))
    for prime in (2, 5):
        powers = 0
        while b % prime == 0 and powers < shift:
            spend(1)
            b //= prime
            powers += 1
    return a % b == 0
