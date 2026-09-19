package sdk

import (
	"math/big"
	"strings"
)

// Decimal coefficients and exponents are exact and independently bounded by
// token length. An exponent is never expanded into zero padding.
type validationDecimal struct {
	sign     int
	digits   string
	exponent *big.Int
}

func validationNumber(token string) (validationDecimal, error) {
	if _, err := ParseNumber(token); err != nil {
		return validationDecimal{}, err
	}
	sign := 1
	if strings.HasPrefix(token, "-") {
		sign = -1
		token = token[1:]
	}
	parts := strings.FieldsFunc(token, func(c rune) bool { return c == 'e' || c == 'E' })
	exponent := new(big.Int)
	if len(parts) > 1 {
		exponent.SetString(parts[1], 10)
	}
	mantissa := parts[0]
	fraction := 0
	if dot := strings.IndexByte(mantissa, '.'); dot >= 0 {
		fraction = len(mantissa) - dot - 1
		mantissa = mantissa[:dot] + mantissa[dot+1:]
	}
	digits := strings.TrimLeft(mantissa, "0")
	if digits == "" {
		return validationDecimal{0, "0", new(big.Int)}, nil
	}
	trimmed := strings.TrimRight(digits, "0")
	exponent.Add(exponent, big.NewInt(int64(len(digits)-len(trimmed)-fraction)))
	return validationDecimal{sign, trimmed, exponent}, nil
}
func (a validationDecimal) compare(b validationDecimal) int {
	if a.sign != b.sign {
		if a.sign < b.sign {
			return -1
		}
		return 1
	}
	if a.sign == 0 {
		return 0
	}
	left := new(big.Int).Add(a.exponent, big.NewInt(int64(len(a.digits))))
	right := new(big.Int).Add(b.exponent, big.NewInt(int64(len(b.digits))))
	order := left.Cmp(right)
	if order == 0 {
		size := max(len(a.digits), len(b.digits))
		order = strings.Compare(a.digits+strings.Repeat("0", size-len(a.digits)), b.digits+strings.Repeat("0", size-len(b.digits)))
	}
	return a.sign * order
}
func (a validationDecimal) integral() bool { return a.sign == 0 || a.exponent.Sign() >= 0 }
func (a validationDecimal) divisible(b validationDecimal, spend func(int) error) (bool, error) {
	if a.sign == 0 {
		return true, nil
	}
	shift := new(big.Int).Sub(a.exponent, b.exponent)
	if shift.Sign() < 0 {
		return false, nil
	}
	if err := spend(len(a.digits) + len(b.digits)); err != nil {
		return false, err
	}
	numerator, _ := new(big.Int).SetString(a.digits, 10)
	denominator, _ := new(big.Int).SetString(b.digits, 10)
	for _, prime := range []int64{2, 5} {
		divisor := big.NewInt(prime)
		count := int64(0)
		for new(big.Int).Mod(denominator, divisor).Sign() == 0 && shift.Cmp(big.NewInt(count)) > 0 {
			if err := spend(1); err != nil {
				return false, err
			}
			denominator.Div(denominator, divisor)
			count++
		}
	}
	return new(big.Int).Mod(numerator, denominator).Sign() == 0, nil
}
