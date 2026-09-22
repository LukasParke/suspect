// Internal exact decimal arithmetic. Included only by runtime.cpp.
#pragma once

namespace @NAMESPACE@::detail {

inline std::string trim_zeroes(std::string value) {
    auto first = value.find_first_not_of('0');
    return first == std::string::npos ? "0" : value.substr(first);
}
inline int magnitude_compare(std::string_view a, std::string_view b) {
    if (a.size() != b.size()) return a.size() < b.size() ? -1 : 1;
    return a == b ? 0 : (a < b ? -1 : 1);
}
inline std::string magnitude_add(std::string_view a, std::string_view b) {
    std::string result;
    auto i = a.size(); auto j = b.size(); int carry = 0;
    while (i || j || carry) {
        int digit = carry;
        if (i) digit += a[--i] - '0';
        if (j) digit += b[--j] - '0';
        result.push_back(static_cast<char>('0' + digit % 10)); carry = digit / 10;
    }
    std::reverse(result.begin(), result.end());
    return trim_zeroes(std::move(result));
}
// Requires a >= b, both canonical nonnegative decimal integers.
inline std::string magnitude_subtract(std::string_view a, std::string_view b) {
    std::string result; auto i = a.size(); auto j = b.size(); int borrow = 0;
    while (i) {
        int digit = a[--i] - '0' - borrow - (j ? b[--j] - '0' : 0);
        borrow = digit < 0 ? 1 : 0;
        result.push_back(static_cast<char>('0' + (digit < 0 ? digit + 10 : digit)));
    }
    std::reverse(result.begin(), result.end()); return trim_zeroes(std::move(result));
}
struct SignedDecimal {
    int sign = 0;
    std::string digits = "0";
    static SignedDecimal parse(std::string_view value) {
        int sign = 1;
        if (!value.empty() && (value.front() == '-' || value.front() == '+')) {
            if (value.front() == '-') sign = -1;
            value.remove_prefix(1);
        }
        auto digits = trim_zeroes(std::string(value));
        return {digits == "0" ? 0 : sign, std::move(digits)};
    }
    static SignedDecimal size(std::size_t value) { return parse(std::to_string(value)); }
    int compare(const SignedDecimal& other) const {
        if (sign != other.sign) return sign < other.sign ? -1 : 1;
        return sign * magnitude_compare(digits, other.digits);
    }
    SignedDecimal plus(const SignedDecimal& other) const {
        if (sign == 0) return other;
        if (other.sign == 0) return *this;
        if (sign == other.sign) return {sign, magnitude_add(digits, other.digits)};
        int order = magnitude_compare(digits, other.digits);
        if (order == 0) return {};
        return order > 0 ? SignedDecimal{sign, magnitude_subtract(digits, other.digits)}
            : SignedDecimal{other.sign, magnitude_subtract(other.digits, digits)};
    }
    SignedDecimal minus(SignedDecimal other) const { other.sign = -other.sign; return plus(other); }
};

struct Decimal {
    int sign = 0;
    std::string digits = "0";
    SignedDecimal exponent;
    static Decimal parse(std::string_view token) {
        int sign = 1;
        if (token.front() == '-') { sign = -1; token.remove_prefix(1); }
        auto e = token.find_first_of("eE");
        auto mantissa = token.substr(0, e);
        auto exponent = e == std::string_view::npos ? SignedDecimal{} : SignedDecimal::parse(token.substr(e + 1));
        auto dot = mantissa.find('.');
        auto fraction = dot == std::string_view::npos ? 0 : mantissa.size() - dot - 1;
        std::string digits;
        for (char c : mantissa) if (c != '.') digits.push_back(c);
        digits = trim_zeroes(std::move(digits));
        if (digits == "0") return {};
        auto size = digits.size();
        while (digits.back() == '0') digits.pop_back();
        exponent = exponent.plus(SignedDecimal::size(size - digits.size())).minus(SignedDecimal::size(fraction));
        return {sign, std::move(digits), std::move(exponent)};
    }
    int compare(const Decimal& other) const {
        if (sign != other.sign) return sign < other.sign ? -1 : 1;
        if (!sign) return 0;
        auto left = exponent.plus(SignedDecimal::size(digits.size()));
        auto right = other.exponent.plus(SignedDecimal::size(other.digits.size()));
        int order = left.compare(right);
        if (!order) {
            auto size = std::max(digits.size(), other.digits.size());
            for (std::size_t i = 0; i < size; ++i) {
                char a = i < digits.size() ? digits[i] : '0';
                char b = i < other.digits.size() ? other.digits[i] : '0';
                if (a != b) { order = a < b ? -1 : 1; break; }
            }
        }
        return sign * order;
    }
    bool integral() const { return !sign || exponent.sign >= 0; }
    template<class Spend> bool divisible(const Decimal& divisor, Spend spend) const {
        if (!sign) return true;
        auto shift = exponent.minus(divisor.exponent);
        if (shift.sign < 0) return false;
        std::string denominator = divisor.digits;
        spend(digits.size() + denominator.size());
        for (unsigned prime : {2u, 5u}) {
            std::size_t count = 0;
            while (denominator != "1" && shift.compare(SignedDecimal::size(count)) > 0) {
                spend(denominator.size());
                std::string quotient; unsigned remainder = 0;
                for (char c : denominator) {
                    auto n = remainder * 10 + static_cast<unsigned>(c - '0');
                    quotient.push_back(static_cast<char>('0' + n / prime)); remainder = n % prime;
                }
                if (remainder) break;
                denominator = trim_zeroes(std::move(quotient)); ++count;
            }
        }
        if (denominator == "1") return true;
        std::string remainder = "0";
        for (char c : digits) {
            spend(remainder.size() + denominator.size());
            remainder = trim_zeroes(remainder + c);
            while (magnitude_compare(remainder, denominator) >= 0) {
                spend(remainder.size());
                remainder = magnitude_subtract(remainder, denominator);
            }
        }
        return remainder == "0";
    }
};
} // namespace @NAMESPACE@::detail
