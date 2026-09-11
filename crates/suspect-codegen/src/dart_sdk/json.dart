/// Immutable JSON values. Numeric tokens never pass through `num` or `double`.
sealed class JsonValue {
  const JsonValue();

  /// Encode an exact JSON value under finite output and work budgets.
  String encode({JsonLimits limits = const JsonLimits()}) =>
      writeJson(this, limits: limits);
}

/// JSON null, distinct from a missing model property.
final class JsonNull extends JsonValue {
  const JsonNull();
}

/// An exact JSON boolean.
final class JsonBoolean extends JsonValue {
  const JsonBoolean(this.value);
  final bool value;
}

/// A Unicode-scalar JSON string. Unpaired UTF-16 surrogates are rejected.
final class JsonString extends JsonValue {
  JsonString(this.value) {
    _unicodeLength(value);
  }
  final String value;
}

/// A snapshot of a JSON array. The supplied collection is copied.
final class JsonArray extends JsonValue {
  JsonArray(Iterable<JsonValue> values) : values = List.unmodifiable(values);
  final List<JsonValue> values;
}

/// A snapshot of a JSON object, retaining all decoded member names.
final class JsonObject extends JsonValue {
  JsonObject(Map<String, JsonValue> values)
    : values = Map.unmodifiable(values) {
    for (final key in values.keys) {
      _unicodeLength(key);
    }
  }
  final Map<String, JsonValue> values;
}

/// A JSON numeric token with exact, symbolic-decimal semantics.
///
/// `1.0`, `-0.0` and `1e999999999999999999999` retain their original spellings.
/// Magnitude does not allocate expanded powers of ten. Use [compareTo] for
/// mathematical equality; object equality remains identity equality.
final class JsonNumber extends JsonValue implements Comparable<JsonNumber> {
  JsonNumber._(this.token, this._exact);

  /// Parse one complete JSON number, rejecting whitespace and non-JSON syntax.
  factory JsonNumber.parse(String token, {int maxBytes = 4096}) {
    _numberSyntax(token, maxBytes);
    return JsonNumber._(token, _Exact.parse(token));
  }

  /// Convert an already exact Dart integer without a floating-point step.
  factory JsonNumber.fromInt(int value) => JsonNumber.parse(value.toString());

  /// Convert an arbitrary-precision integer under the numeric token ceiling.
  factory JsonNumber.fromBigInt(BigInt value, {int maxBytes = 4096}) =>
      JsonNumber.parse(value.toString(), maxBytes: maxBytes);

  final String token;
  final _Exact _exact;

  /// Whether this value is a mathematical integer, independently of spelling.
  bool get isInteger => _exact.sign == 0 || _exact.exponent >= BigInt.zero;

  @override
  int compareTo(JsonNumber other) => _exact.compare(other._exact);

  /// Expand a mathematical integer only when its digit count fits [maxDigits].
  BigInt toBigInt({int maxDigits = 4096}) {
    if (maxDigits < 1 || maxDigits > 65536) {
      throw ArgumentError.value(maxDigits, 'maxDigits', 'must be 1..65536');
    }
    if (!isInteger) {
      throw const JsonException('not an exact integer');
    }
    if (_exact.sign == 0) {
      return BigInt.zero;
    }
    if (_exact.exponent + BigInt.from(_exact.digits.length) >
        BigInt.from(maxDigits)) {
      throw const JsonException(
        'integer expansion budget exhausted',
        resourceLimit: true,
      );
    }
    final coefficient = BigInt.parse(_exact.digits, radix: 10);
    final value = coefficient * BigInt.from(10).pow(_exact.exponent.toInt());
    return _exact.sign < 0 ? -value : value;
  }

  @override
  String toString() => token;
}

/// A mathematically integral JSON number, with its original wire token intact.
final class JsonInteger extends JsonNumber {
  JsonInteger._(super.token, super.exact) : super._();

  factory JsonInteger.parse(String token, {int maxBytes = 4096}) {
    final number = JsonNumber.parse(token, maxBytes: maxBytes);
    if (!number.isInteger) {
      throw const JsonException('not an exact integer');
    }
    return JsonInteger._(number.token, number._exact);
  }

  factory JsonInteger.fromInt(int value) => JsonInteger.parse(value.toString());
  factory JsonInteger.fromBigInt(BigInt value, {int maxBytes = 4096}) =>
      JsonInteger.parse(value.toString(), maxBytes: maxBytes);
}

/// Finite resource ceilings for one parser/writer invocation.
final class JsonLimits {
  const JsonLimits({
    this.maxBytes = 8 * 1024 * 1024,
    this.maxDepth = 128,
    this.maxSteps = 32 * 1024 * 1024,
    this.maxNumberBytes = 4096,
  });
  final int maxBytes;
  final int maxDepth;
  final int maxSteps;
  final int maxNumberBytes;

  void _check() {
    if (maxBytes < 0 ||
        maxBytes > 2147483647 ||
        maxDepth < 1 ||
        maxDepth > 128 ||
        maxSteps < 0 ||
        maxSteps > 2147483647 ||
        maxNumberBytes < 0 ||
        maxNumberBytes > 65536) {
      throw ArgumentError('invalid finite JSON resource policy');
    }
  }
}

/// Invalid JSON and finite-budget failures have distinct [resourceLimit] values.
final class JsonException implements Exception {
  const JsonException(this.message, {this.resourceLimit = false, this.offset});
  final String message;
  final bool resourceLimit;
  final int? offset;
  @override
  String toString() => 'JsonException: $message';
}

/// Parse exact JSON. Duplicate decoded keys and invalid Unicode are rejected.
JsonValue parseJson(String text, {JsonLimits limits = const JsonLimits()}) {
  limits._check();
  if (text.length > limits.maxBytes || _unicodeLength(text) > limits.maxBytes) {
    throw const JsonException(
      'JSON input byte budget exhausted',
      resourceLimit: true,
    );
  }
  return _JsonParser(text, limits).parse();
}

/// Parse strict UTF-8 JSON, checking byte length before decoding or allocation.
JsonValue parseJsonBytes(
  List<int> bytes, {
  JsonLimits limits = const JsonLimits(),
}) {
  limits._check();
  if (bytes.length > limits.maxBytes) {
    throw const JsonException(
      'JSON input byte budget exhausted',
      resourceLimit: true,
    );
  }
  try {
    // Invalid integers in an injected byte list must not be truncated to bytes.
    for (final byte in bytes) {
      if (byte < 0 || byte > 255) {
        throw const FormatException();
      }
    }
    return parseJson(utf8.decode(bytes, allowMalformed: false), limits: limits);
  } on FormatException {
    throw const JsonException('invalid UTF-8 JSON input');
  }
}

/// Serialize tokens verbatim, preserving omission decisions made by codecs.
String writeJson(JsonValue value, {JsonLimits limits = const JsonLimits()}) {
  limits._check();
  final writer = _JsonWriter(limits);
  writer.value(value, 0);
  return writer.out.toString();
}

int _unicodeLength(String value) {
  var bytes = 0;
  for (var i = 0; i < value.length; i++) {
    final unit = value.codeUnitAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (++i >= value.length) {
        throw const JsonException('unpaired Unicode surrogate');
      }
      final next = value.codeUnitAt(i);
      if (next < 0xdc00 || next > 0xdfff) {
        throw const JsonException('unpaired Unicode surrogate');
      }
      bytes += 4;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw const JsonException('unpaired Unicode surrogate');
    } else {
      bytes += unit < 0x80
          ? 1
          : unit < 0x800
          ? 2
          : 3;
    }
  }
  return bytes;
}

bool _digit(int unit) => unit >= 48 && unit <= 57;

void _numberSyntax(String text, int maxBytes) {
  if (maxBytes < 0 || maxBytes > 65536) {
    throw ArgumentError.value(maxBytes, 'maxBytes');
  }
  if (text.length > maxBytes) {
    throw const JsonException(
      'numeric token budget exhausted',
      resourceLimit: true,
    );
  }
  var i = 0;
  if (i < text.length && text.codeUnitAt(i) == 45) {
    i++;
  }
  if (i >= text.length) {
    throw const JsonException('invalid JSON number');
  }
  if (text.codeUnitAt(i) == 48) {
    i++;
  } else if (text.codeUnitAt(i) >= 49 && text.codeUnitAt(i) <= 57) {
    while (i < text.length && _digit(text.codeUnitAt(i))) {
      i++;
    }
  } else {
    throw const JsonException('invalid JSON number');
  }
  if (i < text.length && text.codeUnitAt(i) == 46) {
    final start = ++i;
    while (i < text.length && _digit(text.codeUnitAt(i))) {
      i++;
    }
    if (i == start) {
      throw const JsonException('invalid JSON fraction');
    }
  }
  if (i < text.length &&
      (text.codeUnitAt(i) == 69 || text.codeUnitAt(i) == 101)) {
    i++;
    if (i < text.length &&
        (text.codeUnitAt(i) == 43 || text.codeUnitAt(i) == 45)) {
      i++;
    }
    final start = i;
    while (i < text.length && _digit(text.codeUnitAt(i))) {
      i++;
    }
    if (i == start) {
      throw const JsonException('invalid JSON exponent');
    }
  }
  if (i != text.length) {
    throw const JsonException('invalid JSON number');
  }
}

final class _Exact {
  _Exact(this.sign, this.digits, this.exponent);
  final int sign;
  final String digits;
  final BigInt exponent;

  factory _Exact.parse(String token) {
    final negative = token.startsWith('-');
    final unsigned = negative ? token.substring(1) : token;
    final e = unsigned.indexOf(RegExp('[eE]'));
    final mantissa = e < 0 ? unsigned : unsigned.substring(0, e);
    final dot = mantissa.indexOf('.');
    final fraction = dot < 0 ? 0 : mantissa.length - dot - 1;
    final digits = mantissa.replaceFirst('.', '');
    var start = 0;
    while (start < digits.length && digits.codeUnitAt(start) == 48) {
      start++;
    }
    if (start == digits.length) {
      return _Exact(0, '0', BigInt.zero);
    }
    var end = digits.length;
    while (digits.codeUnitAt(end - 1) == 48) {
      end--;
    }
    final exponent = e < 0
        ? BigInt.zero
        : BigInt.parse(unsigned.substring(e + 1), radix: 10);
    return _Exact(
      negative ? -1 : 1,
      digits.substring(start, end),
      exponent - BigInt.from(fraction) + BigInt.from(digits.length - end),
    );
  }

  int compare(_Exact other) {
    if (sign != other.sign) {
      return sign.compareTo(other.sign);
    }
    if (sign == 0) {
      return 0;
    }
    final order = (exponent + BigInt.from(digits.length)).compareTo(
      other.exponent + BigInt.from(other.digits.length),
    );
    if (order != 0) {
      return sign * order;
    }
    final length = digits.length > other.digits.length
        ? digits.length
        : other.digits.length;
    for (var i = 0; i < length; i++) {
      final a = i < digits.length ? digits.codeUnitAt(i) : 48;
      final b = i < other.digits.length ? other.digits.codeUnitAt(i) : 48;
      if (a != b) {
        return sign * a.compareTo(b);
      }
    }
    return 0;
  }

  bool multipleOf(_Exact divisor, void Function(int) spend) {
    if (sign == 0) {
      return true;
    }
    final shift = exponent - divisor.exponent;
    if (shift < BigInt.zero) {
      return false;
    }
    spend(
      digits.length +
          divisor.digits.length +
          digits.length * divisor.digits.length,
    );
    final numerator = BigInt.parse(digits, radix: 10);
    var denominator = BigInt.parse(divisor.digits, radix: 10);
    // Remove only the factors supplied by symbolic 10^shift. Loop work is
    // bounded by the written coefficient, never the exponent's magnitude.
    for (final prime in [BigInt.two, BigInt.from(5)]) {
      var powers = BigInt.zero;
      while (powers < shift && denominator % prime == BigInt.zero) {
        spend(divisor.digits.length);
        denominator ~/= prime;
        powers += BigInt.one;
      }
    }
    return numerator % denominator == BigInt.zero;
  }
}

final class _JsonParser {
  _JsonParser(this.text, this.limits) : left = limits.maxSteps;
  final String text;
  final JsonLimits limits;
  int at = 0;
  int left;
  void spend([int n = 1]) {
    left -= n;
    if (left < 0) {
      fail('JSON parser work budget exhausted', limit: true);
    }
  }

  Never fail(String message, {bool limit = false}) =>
      throw JsonException(message, resourceLimit: limit, offset: at);
  int get peek => at < text.length ? text.codeUnitAt(at) : -1;
  void whitespace() {
    while (peek == 32 || peek == 9 || peek == 10 || peek == 13) {
      spend();
      at++;
    }
  }

  JsonValue parse() {
    whitespace();
    final result = value(0);
    whitespace();
    if (at != text.length) {
      fail('trailing JSON input');
    }
    return result;
  }

  JsonValue value(int depth) {
    spend();
    if (depth >= limits.maxDepth) {
      fail('JSON nesting budget exhausted', limit: true);
    }
    switch (peek) {
      case 110:
        literal('null');
        return const JsonNull();
      case 116:
        literal('true');
        return const JsonBoolean(true);
      case 102:
        literal('false');
        return const JsonBoolean(false);
      case 34:
        return JsonString(string());
      case 91:
        at++;
        whitespace();
        final values = <JsonValue>[];
        if (peek == 93) {
          at++;
          return JsonArray(values);
        }
        while (true) {
          values.add(value(depth + 1));
          whitespace();
          if (peek == 93) {
            at++;
            return JsonArray(values);
          }
          if (peek != 44) {
            fail('expected array comma');
          }
          at++;
          whitespace();
        }
      case 123:
        at++;
        whitespace();
        final values = <String, JsonValue>{};
        if (peek == 125) {
          at++;
          return JsonObject(values);
        }
        while (true) {
          if (peek != 34) {
            fail('expected object key');
          }
          final key = string();
          whitespace();
          if (values.containsKey(key)) {
            fail('duplicate decoded object key');
          }
          if (peek != 58) {
            fail('expected object colon');
          }
          at++;
          whitespace();
          values[key] = value(depth + 1);
          whitespace();
          if (peek == 125) {
            at++;
            return JsonObject(values);
          }
          if (peek != 44) {
            fail('expected object comma');
          }
          at++;
          whitespace();
        }
      default:
        final start = at;
        while (_digit(peek) ||
            peek == 45 ||
            peek == 43 ||
            peek == 46 ||
            peek == 101 ||
            peek == 69) {
          spend();
          at++;
          if (at - start > limits.maxNumberBytes) {
            fail('numeric token budget exhausted', limit: true);
          }
        }
        if (at == start) {
          fail('expected JSON value');
        }
        return JsonNumber.parse(
          text.substring(start, at),
          maxBytes: limits.maxNumberBytes,
        );
    }
  }

  void literal(String expected) {
    spend(expected.length);
    if (!text.startsWith(expected, at)) {
      fail('invalid JSON literal');
    }
    at += expected.length;
  }

  int hex() {
    var result = 0;
    for (var i = 0; i < 4; i++) {
      spend();
      final c = peek;
      final digit = c >= 48 && c <= 57
          ? c - 48
          : c >= 65 && c <= 70
          ? c - 55
          : c >= 97 && c <= 102
          ? c - 87
          : -1;
      if (digit < 0) {
        fail('invalid Unicode escape');
      }
      at++;
      result = result * 16 + digit;
    }
    return result;
  }

  String string() {
    at++;
    final result = StringBuffer();
    while (at < text.length) {
      spend();
      var unit = text.codeUnitAt(at++);
      if (unit == 34) {
        return result.toString();
      }
      if (unit < 32) {
        fail('unescaped control character');
      }
      if (unit == 92) {
        if (at >= text.length) {
          fail('incomplete string escape');
        }
        spend();
        switch (text.codeUnitAt(at++)) {
          case 34:
            unit = 34;
          case 92:
            unit = 92;
          case 47:
            unit = 47;
          case 98:
            unit = 8;
          case 102:
            unit = 12;
          case 110:
            unit = 10;
          case 114:
            unit = 13;
          case 116:
            unit = 9;
          case 117:
            unit = hex();
            if (unit >= 0xd800 && unit <= 0xdbff) {
              if (!text.startsWith('\\u', at)) {
                fail('unpaired Unicode surrogate');
              }
              at += 2;
              final next = hex();
              if (next < 0xdc00 || next > 0xdfff) {
                fail('unpaired Unicode surrogate');
              }
              result.writeCharCode(unit);
              result.writeCharCode(next);
              continue;
            }
            if (unit >= 0xdc00 && unit <= 0xdfff) {
              fail('unpaired Unicode surrogate');
            }
          default:
            fail('invalid string escape');
        }
      }
      result.writeCharCode(unit);
    }
    fail('unterminated JSON string');
  }
}

final class _JsonWriter {
  _JsonWriter(this.limits) : left = limits.maxSteps;
  final JsonLimits limits;
  final StringBuffer out = StringBuffer();
  int bytes = 0;
  int left;
  void spend([int n = 1]) {
    left -= n;
    if (left < 0) {
      throw const JsonException(
        'JSON writer work budget exhausted',
        resourceLimit: true,
      );
    }
  }

  void add(String text) {
    spend(text.length);
    bytes += _unicodeLength(text);
    if (bytes > limits.maxBytes) {
      throw const JsonException(
        'JSON output byte budget exhausted',
        resourceLimit: true,
      );
    }
    out.write(text);
  }

  void string(String text) {
    spend(text.length);
    if (text.length > limits.maxBytes - bytes) {
      throw const JsonException(
        'JSON output byte budget exhausted',
        resourceLimit: true,
      );
    }
    _unicodeLength(text);
    add('"');
    // Write bounded slices; escaped strings are never built in a separate
    // unbounded buffer before checking the output ceiling.
    for (var i = 0; i < text.length; i++) {
      final unit = text.codeUnitAt(i);
      if (unit == 34) {
        add('\\"');
      } else if (unit == 92) {
        add('\\\\');
      } else if (unit < 32) {
        add('\\u${unit.toRadixString(16).padLeft(4, '0')}');
      } else if (unit >= 0xd800 && unit <= 0xdbff) {
        add(text.substring(i, i + 2));
        i++;
      } else {
        add(text[i]);
      }
    }
    add('"');
  }

  void value(JsonValue value, int depth) {
    spend();
    if (depth >= limits.maxDepth) {
      throw const JsonException(
        'JSON nesting budget exhausted',
        resourceLimit: true,
      );
    }
    switch (value) {
      case JsonNull():
        add('null');
      case JsonBoolean():
        add(value.value ? 'true' : 'false');
      case JsonString():
        string(value.value);
      case JsonNumber():
        if (value.token.length > limits.maxNumberBytes) {
          throw const JsonException(
            'numeric token budget exhausted',
            resourceLimit: true,
          );
        }
        add(value.token);
      case JsonArray():
        add('[');
        for (var i = 0; i < value.values.length; i++) {
          if (i != 0) {
            add(',');
          }
          this.value(value.values[i], depth + 1);
        }
        add(']');
      case JsonObject():
        add('{');
        var first = true;
        for (final entry in value.values.entries) {
          if (!first) {
            add(',');
          }
          first = false;
          string(entry.key);
          add(':');
          this.value(entry.value, depth + 1);
        }
        add('}');
    }
  }
}
