<?php
declare(strict_types=1);

namespace __NAMESPACE__;

/** Exact JSON numeric token; coefficient and exponent never pass through a float. */
final readonly class JsonNumber
{
    private function __construct(
        public string $token,
        private int $sign,
        private string $digits,
        private string $exponent,
    ) {}

    /** Parse a JSON number with a symbolic, arbitrarily large signed exponent. */
    public static function fromString(string $token): self
    {
        if (strlen($token) > 65536) {
            throw new JsonError('resource_limit', 'numeric token exceeds 65536 bytes');
        }
        if (preg_match('/\A(-?)(0|[1-9][0-9]*)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?\z/D', $token, $parts) !== 1) {
            throw new JsonError('syntax', 'invalid JSON number');
        }
        $fraction = $parts[3] ?? '';
        $digits = ltrim($parts[2] . $fraction, '0');
        if ($digits === '') {
            return new self($token, 0, '0', '0');
        }
        $trimmed = rtrim($digits, '0');
        $exponent = DecimalMath::add(
            DecimalMath::normalize($parts[4] ?? '0'),
            (string) (strlen($digits) - strlen($trimmed) - strlen($fraction)),
        );
        return new self($token, $parts[1] === '-' ? -1 : 1, $trimmed, $exponent);
    }

    public static function fromInt(int $value): self { return self::fromString((string) $value); }
    public function __toString(): string { return $this->token; }
    public function isInteger(): bool { return $this->sign === 0 || DecimalMath::compare($this->exponent, '0') >= 0; }

    /** Mathematical comparison, independent of number spelling and exponent magnitude. */
    public function compare(self $other): int
    {
        if ($this->sign !== $other->sign) { return $this->sign <=> $other->sign; }
        if ($this->sign === 0) { return 0; }
        $left = DecimalMath::add($this->exponent, (string) strlen($this->digits));
        $right = DecimalMath::add($other->exponent, (string) strlen($other->digits));
        $order = DecimalMath::compare($left, $right);
        if ($order === 0) {
            $length = max(strlen($this->digits), strlen($other->digits));
            $order = strcmp(str_pad($this->digits, $length, '0'), str_pad($other->digits, $length, '0')) <=> 0;
        }
        return $this->sign * $order;
    }

    /** Exact range-checked conversion to the platform's native integer. */
    public function toInt(): int
    {
        if (!$this->isInteger() || $this->compare(self::fromInt(PHP_INT_MIN)) < 0 || $this->compare(self::fromInt(PHP_INT_MAX)) > 0) {
            throw new JsonError('conversion', 'number is not a representable native integer');
        }
        return (int) $this->toDecimalString(32);
    }

    /** Expand only when the exact plain-decimal result fits the caller's byte limit. */
    public function toDecimalString(int $maxBytes = 4096): string
    {
        if ($maxBytes < 1 || $maxBytes > 16777216) {
            throw new JsonError('resource_limit', 'decimal expansion limit must be between 1 and 16777216');
        }
        if ($this->sign === 0) { return '0'; }
        if (DecimalMath::compare($this->exponent, (string) $maxBytes) > 0 || DecimalMath::compare($this->exponent, '-' . $maxBytes) < 0) {
            throw new JsonError('resource_limit', 'decimal expansion exceeds byte limit');
        }
        $shift = (int) $this->exponent; // Proved small by the symbolic comparison above.
        $point = strlen($this->digits) + $shift;
        $sign = $this->sign < 0 ? '-' : '';
        $length = strlen($sign) + ($shift >= 0 ? $point : ($point > 0 ? strlen($this->digits) + 1 : 2 - $point + strlen($this->digits)));
        if ($length > $maxBytes) { throw new JsonError('resource_limit', 'decimal expansion exceeds byte limit'); }
        if ($shift >= 0) { return $sign . $this->digits . str_repeat('0', $shift); }
        if ($point > 0) { return $sign . substr($this->digits, 0, $point) . '.' . substr($this->digits, $point); }
        return $sign . '0.' . str_repeat('0', -$point) . $this->digits;
    }

    /**
     * Exact divisibility without expanding either exponent.
     * @param \Closure(int): void $spend Shared finite numeric-work budget.
     * @internal
     */
    public function multipleOf(self $divisor, \Closure $spend): bool
    {
        if ($divisor->sign <= 0) { throw new JsonError('conversion', 'divisor must be positive'); }
        if ($this->sign === 0) { return true; }
        $shift = DecimalMath::add($this->exponent, DecimalMath::negate($divisor->exponent));
        if (DecimalMath::compare($shift, '0') < 0) { return false; }
        $denominator = $divisor->digits;
        // Coefficients are normalized with no trailing zero. Only powers of
        // two and five in the denominator can cancel a positive decimal shift.
        foreach ([2, 5] as $prime) {
            $powers = 0;
            while (DecimalMath::compare((string) $powers, $shift) < 0) {
                $spend(strlen($denominator));
                [$quotient, $remainder] = DecimalMath::divideSmall($denominator, $prime);
                if ($remainder !== 0) { break; }
                $denominator = $quotient;
                ++$powers;
            }
        }
        return DecimalMath::remainder($this->digits, $denominator, $spend) === '0';
    }
}

/** Decimal-string integer arithmetic. No PHP numeric-string comparisons. @internal */
final class DecimalMath
{
    public static function normalize(string $value): string
    {
        $negative = str_starts_with($value, '-');
        $digits = ltrim(ltrim($value, '+-'), '0');
        return $digits === '' ? '0' : ($negative ? '-' : '') . $digits;
    }

    public static function negate(string $value): string
    {
        return $value === '0' ? '0' : (str_starts_with($value, '-') ? substr($value, 1) : '-' . $value);
    }

    public static function compare(string $a, string $b): int
    {
        $an = str_starts_with($a, '-'); $bn = str_starts_with($b, '-');
        if ($an !== $bn) { return $an ? -1 : 1; }
        return ($an ? -1 : 1) * self::unsignedCompare(ltrim($a, '-'), ltrim($b, '-'));
    }

    private static function unsignedCompare(string $a, string $b): int
    {
        return strlen($a) === strlen($b) ? (strcmp($a, $b) <=> 0) : (strlen($a) <=> strlen($b));
    }

    public static function add(string $a, string $b): string
    {
        $an = str_starts_with($a, '-'); $bn = str_starts_with($b, '-');
        $a = ltrim($a, '-'); $b = ltrim($b, '-');
        if ($an === $bn) {
            $result = self::unsignedAdd($a, $b);
            return $an && $result !== '0' ? '-' . $result : $result;
        }
        $order = self::unsignedCompare($a, $b);
        if ($order === 0) { return '0'; }
        $result = $order > 0 ? self::unsignedSubtract($a, $b) : self::unsignedSubtract($b, $a);
        return ($order > 0 ? $an : $bn) ? '-' . $result : $result;
    }

    private static function unsignedAdd(string $a, string $b): string
    {
        $at = strlen($a) - 1; $bt = strlen($b) - 1; $carry = 0; $out = '';
        while ($at >= 0 || $bt >= 0 || $carry !== 0) {
            $sum = ($at >= 0 ? ord($a[$at--]) - 48 : 0) + ($bt >= 0 ? ord($b[$bt--]) - 48 : 0) + $carry;
            $out .= chr(48 + $sum % 10); $carry = intdiv($sum, 10);
        }
        return strrev($out);
    }

    private static function unsignedSubtract(string $a, string $b): string
    {
        $at = strlen($a) - 1; $bt = strlen($b) - 1; $borrow = 0; $out = '';
        while ($at >= 0) {
            $digit = ord($a[$at--]) - 48 - ($bt >= 0 ? ord($b[$bt--]) - 48 : 0) - $borrow;
            $borrow = $digit < 0 ? 1 : 0;
            $out .= chr(48 + ($digit < 0 ? $digit + 10 : $digit));
        }
        $result = ltrim(strrev($out), '0');
        return $result === '' ? '0' : $result;
    }

    /** @return array{string, int} */
    public static function divideSmall(string $digits, int $divisor): array
    {
        $carry = 0; $quotient = '';
        for ($at = 0, $length = strlen($digits); $at < $length; ++$at) {
            $carry = $carry * 10 + ord($digits[$at]) - 48;
            $quotient .= chr(48 + intdiv($carry, $divisor)); $carry %= $divisor;
        }
        $quotient = ltrim($quotient, '0');
        return [$quotient === '' ? '0' : $quotient, $carry];
    }

    /** @param \Closure(int): void $spend */
    public static function remainder(string $digits, string $divisor, \Closure $spend): string
    {
        if ($divisor === '1') { $spend(strlen($digits)); return '0'; }
        $remainder = '0';
        for ($at = 0, $length = strlen($digits); $at < $length; ++$at) {
            $remainder = self::normalize($remainder . $digits[$at]);
            $spend(strlen($remainder) + strlen($divisor));
            while (self::unsignedCompare($remainder, $divisor) >= 0) {
                $spend(strlen($remainder) + strlen($divisor));
                $remainder = self::unsignedSubtract($remainder, $divisor);
            }
        }
        return $remainder;
    }
}
