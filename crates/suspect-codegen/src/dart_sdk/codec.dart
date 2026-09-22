/// Property absence is separate from a present nullable value.
sealed class Presence<T> {
  const Presence();
  bool get isPresent => this is Present<T>;
}

/// A property that is omitted from the JSON object.
final class Absent<T> extends Presence<T> {
  const Absent();
}

/// A present property; `Present<String?>(null)` emits an explicit JSON null.
final class Present<T> extends Presence<T> {
  const Present(this.value);
  final T value;
}

/// Codecs distinguish malformed input, mismatch, conversion and budget failure.
enum CodecFailureKind {
  json,
  invalid,
  evaluationFailure,
  conversion,
  resourceLimit,
}

/// A native typed codec failure; source findings retain their original identity.
final class CodecException implements Exception {
  CodecException(
    this.kind,
    this.source,
    this.instancePath,
    this.message, {
    Iterable<ValidationFinding> findings = const [],
  }) : findings = List.unmodifiable(findings);
  final CodecFailureKind kind;
  final SchemaSource source;
  final String instancePath;
  final String message;
  final List<ValidationFinding> findings;
  @override
  String toString() => 'CodecException(${kind.name}) at $source';
}

/// Source-bound typed JSON codec. Every encode revalidates the current model.
final class ModelCodec<T> {
  const ModelCodec._(this.source, this._root, this._decode, this._encode);
  final SchemaSource source;
  final int _root;
  final T Function(JsonValue, _Conversion) _decode;
  final JsonValue Function(T, _Conversion) _encode;

  /// Validate exact JSON with fresh shared logical/equality/numeric budgets.
  ValidationResult validate(JsonValue value) =>
      _ValidationSession().validate(_root, value);

  /// Decode strict JSON text, preserving all numeric tokens and extra members.
  T decode(String text) =>
      _jsonCall(() => fromJson(parseJson(text, limits: _decodeLimits)));

  /// Decode bounded strict UTF-8 without `jsonDecode` or floating-point values.
  T decodeBytes(List<int> bytes) =>
      _jsonCall(() => fromJson(parseJsonBytes(bytes, limits: _decodeLimits)));

  /// Validate before exposing a typed native value. No defaults are inserted.
  T fromJson(JsonValue value) {
    final conversion = _Conversion(source);
    conversion.requireValid(_root, value);
    return _jsonCall(() => _decode(value, conversion));
  }

  /// Revalidate mutable fields, collections, recursion and unknown-key values.
  JsonValue toJson(T value) => _toJsonWith(value, _Conversion(source));

  // Parts share one conversion/evaluation allowance for the complete request.
  JsonValue _toJsonWith(T value, _Conversion conversion) => _jsonCall(() {
    final json = _encode(value, conversion);
    conversion.requireValid(_root, json);
    return json;
  });

  /// Encode the current value, validating it again on every invocation.
  String encode(T value) =>
      _jsonCall(() => writeJson(toJson(value), limits: _encodeLimits));

  /// Exact, bounded UTF-8 request bytes.
  Uint8List encodeBytes(T value) =>
      Uint8List.fromList(utf8.encode(encode(value)));

  R _jsonCall<R>(R Function() call) {
    try {
      return call();
    } on JsonException catch (error) {
      throw CodecException(
        error.resourceLimit
            ? CodecFailureKind.resourceLimit
            : CodecFailureKind.json,
        source,
        '',
        error.message,
      );
    }
  }
}

final class _Conversion {
  _Conversion(this.source);
  final SchemaSource source;
  final _ValidationSession validation = _ValidationSession();
  int left = _maxConversionSteps;
  int depth = 0;
  String path = '';
  final Set<Object> active = HashSet<Object>.identity();

  Never fail(String message, {bool limit = false}) => throw CodecException(
    limit ? CodecFailureKind.resourceLimit : CodecFailureKind.conversion,
    source,
    path,
    message,
  );
  void spend([int amount = 1]) {
    left -= amount;
    if (left < 0) {
      fail('native conversion work budget exhausted', limit: true);
    }
  }

  R nest<R>(R Function() call) {
    spend();
    if (depth >= _maxConversionDepth) {
      fail('native conversion depth budget exhausted', limit: true);
    }
    depth++;
    try {
      return call();
    } finally {
      depth--;
    }
  }

  R object<R>(Object value, R Function() call) {
    if (!active.add(value)) {
      fail('cyclic mutable model or collection');
    }
    try {
      return call();
    } finally {
      active.remove(value);
    }
  }

  R child<R>(String key, R Function() call) {
    // Include the copied prefix and the worst-case escaped member spelling.
    spend(path.length + key.length * 2 + 1);
    final parent = path;
    path = _pointerChild(path, key);
    try {
      return call();
    } finally {
      path = parent;
    }
  }

  String string(String value) {
    spend(value.length);
    try {
      _unicodeLength(value);
    } on JsonException {
      fail('native string contains an unpaired Unicode surrogate');
    }
    return value;
  }

  JsonValue json(JsonValue value) {
    // A model may carry arbitrary immutable JSON. Bound its entire graph on
    // encode/fromJson as well as when a text parser happened to produce it.
    final pending = <(JsonValue, int)>[(value, depth)];
    while (pending.isNotEmpty) {
      final (item, level) = pending.removeLast();
      spend();
      if (level >= _maxConversionDepth) {
        fail('JSON conversion depth budget exhausted', limit: true);
      }
      switch (item) {
        case JsonString():
          string(item.value);
        case JsonNumber():
          spend(item.token.length);
          if (item.token.length > _decodeLimits.maxNumberBytes) {
            fail('numeric conversion budget exhausted', limit: true);
          }
        case JsonArray():
          if (item.values.length + pending.length > left) {
            fail('JSON conversion work budget exhausted', limit: true);
          }
          pending.addAll(item.values.map((v) => (v, level + 1)));
        case JsonObject():
          if (item.values.length + pending.length > left) {
            fail('JSON conversion work budget exhausted', limit: true);
          }
          for (final entry in item.values.entries) {
            string(entry.key);
            pending.add((entry.value, level + 1));
          }
        case JsonNull():
          break;
        case JsonBoolean():
          break;
      }
    }
    return value;
  }

  void requireValid(int root, JsonValue value) {
    final result = validation.validate(root, value);
    if (result.isValid) {
      return;
    }
    final finding = result.findings.first;
    throw CodecException(
      result.status == ValidationStatus.invalid
          ? CodecFailureKind.invalid
          : CodecFailureKind.evaluationFailure,
      finding.source,
      finding.instancePath,
      finding.message,
      findings: result.findings,
    );
  }

  bool matches(int root, JsonValue value) {
    try {
      return validation.trial(root, value, path);
    } on _EvaluationAbort catch (error) {
      throw CodecException(
        CodecFailureKind.evaluationFailure,
        error.finding.source,
        error.finding.instancePath,
        error.finding.message,
        findings: [error.finding],
      );
    }
  }
}
