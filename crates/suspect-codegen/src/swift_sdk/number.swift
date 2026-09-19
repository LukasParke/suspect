// Exact decimal arithmetic. Exponents remain symbolic signed decimal integers;
// neither token magnitude nor exponent magnitude allocates zero padding.

struct BigSigned: Sendable {
    let sign: Int
    let digits: [UInt8]
    init(_ value: Int) { self.init(text: String(value)) }
    init(text: String) {
        let bytes = Array(text.utf8)
        let negative = bytes.first == 45
        let start = bytes.first == 43 || negative ? 1 : 0
        let values = bytes.dropFirst(start).map { $0 - 48 }
        let normalized = DigitMath.trim(values)
        self.sign = normalized == [0] ? 0 : negative ? -1 : 1
        self.digits = normalized
    }
    init(sign: Int, digits: [UInt8]) {
        self.digits = DigitMath.trim(digits)
        self.sign = self.digits == [0] ? 0 : sign
    }
    func compare(_ other: Self) -> Int {
        if sign != other.sign { return sign < other.sign ? -1 : 1 }
        return sign * DigitMath.compare(digits, other.digits)
    }
    func adding(_ other: Self) -> Self {
        if sign == 0 { return other }
        if other.sign == 0 { return self }
        if sign == other.sign { return Self(sign: sign, digits: DigitMath.add(digits, other.digits)) }
        let order = DigitMath.compare(digits, other.digits)
        if order == 0 { return Self(0) }
        return order > 0
            ? Self(sign: sign, digits: DigitMath.subtract(digits, other.digits))
            : Self(sign: other.sign, digits: DigitMath.subtract(other.digits, digits))
    }
    func subtracting(_ other: Self) -> Self { adding(Self(sign: -other.sign, digits: other.digits)) }
    func smallInt() -> Int? {
        guard digits.count <= 19 else { return nil }
        return Int((sign < 0 ? "-" : "") + String(decoding: digits.map { $0 + 48 }, as: UTF8.self))
    }
}

struct ExactDecimal: Sendable {
    let sign: Int
    let digits: [UInt8]
    let exponent: BigSigned

    init(_ token: String) {
        let bytes = Array(token.utf8)
        let negative = bytes.first == 45
        let start = negative ? 1 : 0
        let exponentAt = bytes.firstIndex(where: { $0 == 101 || $0 == 69 }) ?? bytes.count
        let mantissa = bytes[start..<exponentAt]
        let dot = mantissa.firstIndex(of: 46)
        let fraction = dot.map { exponentAt - $0 - 1 } ?? 0
        let coefficient = DigitMath.trim(mantissa.filter { $0 != 46 }.map { $0 - 48 })
        if coefficient == [0] {
            sign = 0; digits = [0]; exponent = BigSigned(0); return
        }
        var end = coefficient.count
        while end > 1 && coefficient[end - 1] == 0 { end -= 1 }
        let exp = exponentAt < bytes.count ? BigSigned(text: String(decoding: bytes[(exponentAt + 1)...], as: UTF8.self)) : BigSigned(0)
        sign = negative ? -1 : 1
        digits = Array(coefficient[..<end])
        exponent = exp.adding(BigSigned(coefficient.count - end - fraction))
    }
    var isIntegral: Bool { sign == 0 || exponent.sign >= 0 }
    func compare(_ other: Self) -> Int {
        if sign != other.sign { return sign < other.sign ? -1 : 1 }
        if sign == 0 { return 0 }
        let order = exponent.adding(BigSigned(digits.count)).compare(other.exponent.adding(BigSigned(other.digits.count)))
        if order != 0 { return sign * order }
        for index in 0..<max(digits.count, other.digits.count) {
            let a = index < digits.count ? digits[index] : 0
            let b = index < other.digits.count ? other.digits[index] : 0
            if a != b { return sign * (a < b ? -1 : 1) }
        }
        return 0
    }
    func divisible(by other: Self, spend: (Int) throws -> Void) throws -> Bool {
        if sign == 0 { return true }
        let shift = exponent.subtracting(other.exponent)
        if shift.sign < 0 { return false }
        try spend(digits.count + other.digits.count)
        var denominator = other.digits
        for prime in [UInt8(2), UInt8(5)] {
            var count = 0
            while denominator.last! % prime == 0 && shift.compare(BigSigned(count)) > 0 {
                try spend(denominator.count)
                denominator = DigitMath.divideSmall(denominator, by: prime)
                count += 1
            }
        }
        if denominator == [1] { return true }
        var remainder: [UInt8] = [0]
        for digit in digits {
            try spend(remainder.count + denominator.count)
            remainder = DigitMath.trim(remainder + [digit])
            while DigitMath.compare(remainder, denominator) >= 0 {
                try spend(remainder.count + denominator.count)
                remainder = DigitMath.subtract(remainder, denominator)
            }
        }
        return remainder == [0]
    }
}

enum DigitMath {
    static func trim(_ digits: [UInt8]) -> [UInt8] {
        let start = digits.firstIndex(where: { $0 != 0 }) ?? digits.count
        return start == digits.count ? [0] : Array(digits[start...])
    }
    static func compare(_ a: [UInt8], _ b: [UInt8]) -> Int {
        if a.count != b.count { return a.count < b.count ? -1 : 1 }
        for (x, y) in zip(a, b) { if x != y { return x < y ? -1 : 1 } }
        return 0
    }
    static func add(_ a: [UInt8], _ b: [UInt8]) -> [UInt8] {
        var out: [UInt8] = []; var carry = 0
        for offset in 0..<max(a.count, b.count) {
            let x = offset < a.count ? Int(a[a.count - offset - 1]) : 0
            let y = offset < b.count ? Int(b[b.count - offset - 1]) : 0
            let sum = x + y + carry
            out.append(UInt8(sum % 10)); carry = sum / 10
        }
        if carry > 0 { out.append(UInt8(carry)) }
        return Array(out.reversed())
    }
    // a >= b, both positive normalized coefficients.
    static func subtract(_ a: [UInt8], _ b: [UInt8]) -> [UInt8] {
        var out: [UInt8] = []; var borrow = 0
        for offset in 0..<a.count {
            let x = Int(a[a.count - offset - 1]) - borrow
            let y = offset < b.count ? Int(b[b.count - offset - 1]) : 0
            var difference = x - y
            borrow = difference < 0 ? 1 : 0
            if difference < 0 { difference += 10 }
            out.append(UInt8(difference))
        }
        return trim(Array(out.reversed()))
    }
    static func divideSmall(_ a: [UInt8], by divisor: UInt8) -> [UInt8] {
        var out: [UInt8] = []; var remainder: UInt8 = 0
        for digit in a {
            let value = remainder * 10 + digit
            out.append(value / divisor); remainder = value % divisor
        }
        return trim(out)
    }
}
