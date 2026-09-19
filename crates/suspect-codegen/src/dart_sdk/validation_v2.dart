// Checked static-applicator v2 executor. The v1 executor stays byte-for-byte
// separate so unused modern applicators do not change v1 output or budgets.
// ignore_for_file: unused_element_parameter

/// Stable original schema/keyword identity, shared by docs, codecs and findings.
final class SchemaSource {
  const SchemaSource(this.document, this.pointer);
  final String document;
  final String pointer;
  @override
  bool operator ==(Object other) =>
      other is SchemaSource &&
      document == other.document &&
      pointer == other.pointer;
  @override
  int get hashCode => Object.hash(document, pointer);
  @override
  String toString() => '$document#$pointer';
}

/// A located schema mismatch or an incomplete evaluation.
final class ValidationFinding {
  const ValidationFinding(this.source, this.instancePath, this.message);
  final SchemaSource source;
  final String instancePath;
  final String message;
}

/// Evaluation failure is distinct from a completed schema rejection.
enum ValidationStatus { valid, invalid, evaluationFailure }

/// Completed validation or a finite-budget/nonproductive-recursion failure.
final class ValidationResult {
  ValidationResult(this.status, Iterable<ValidationFinding> findings)
    : findings = List.unmodifiable(findings);
  final ValidationStatus status;
  final List<ValidationFinding> findings;
  bool get isValid => status == ValidationStatus.valid;
}

/// Evaluate a selected root with fresh, finite, shared evaluation budgets.
///
/// Successful same-instance applicators propagate scoped evaluated locations.
/// Trials suppress only mismatches; resource and recursion failures propagate.
ValidationResult validateJson(SchemaSource source, JsonValue value) {
  final index = _validationRoots[source];
  if (index == null) {
    return ValidationResult(ValidationStatus.evaluationFailure, [
      ValidationFinding(source, '', 'schema was not selected as a root'),
    ]);
  }
  return _ValidationSession().validate(index, value);
}

enum _Op {
  always,
  type,
  ref,
  properties,
  additionalProperties,
  required,
  items,
  prefixItems,
  allOf,
  anyOf,
  oneOf,
  not,
  bound,
  multipleOf,
  count,
  enumValue,
  constValue,
  uniqueItems,
  pattern,
  ifValue,
  dependentRequired,
  dependentSchemas,
  contains,
  patternProperties,
  additionalPropertiesWithPatterns,
  propertyNames,
  unevaluatedProperties,
  unevaluatedItems,
}

enum _JsonType { nullValue, boolean, integer, number, string, array, object }

final class _Property {
  const _Property(this.name, this.target);
  final String name;
  final int target;
}

final class _PatternProperty {
  const _PatternProperty(this.name, this.pattern, this.target);
  final String name;
  final _Pattern pattern;
  final int target;
}

final class _Check {
  const _Check(
    this.source,
    this.op, {
    this.target = 0,
    this.targets = const [],
    this.properties = const [],
    this.names = const [],
    this.types = const [],
    this.value = true,
    this.maximum = false,
    this.exclusive = false,
    this.start = 0,
    this.number,
    this.literal,
    this.literals = const [],
    this.countType,
    this.pattern,
    this.condition = 0,
    this.thenTarget,
    this.elseTarget,
    this.dependencies = const [],
    this.minimumCount,
    this.maximumCount,
    this.patterns = const [],
  });
  final SchemaSource source;
  final _Op op;
  final int target;
  final List<int> targets;
  final List<_Property> properties;
  final List<String> names;
  final List<_JsonType> types;
  final bool value;
  final bool maximum;
  final bool exclusive;
  final int start;
  final JsonNumber? number;
  final JsonValue? literal;
  final List<JsonValue> literals;
  final _JsonType? countType;
  final _Pattern? pattern;
  final int condition;
  final int? thenTarget;
  final int? elseTarget;
  final List<(String, List<String>)> dependencies;
  final JsonNumber? minimumCount;
  final JsonNumber? maximumCount;
  final List<_PatternProperty> patterns;
}

final class _ValidationNode {
  const _ValidationNode(this.source, this.checks);
  final SchemaSource source;
  final List<_Check> checks;
}

final class _ValidationLimits {
  const _ValidationLimits(
    this.maxDepth,
    this.maxErrors,
    this.maxNumberBytes,
    this.maxEqualitySteps,
    this.maxEvaluationSteps,
  );
  final int maxDepth;
  final int maxErrors;
  final int maxNumberBytes;
  final int maxEqualitySteps;
  final int maxEvaluationSteps;
}

final class _EvaluationAbort implements Exception {
  const _EvaluationAbort(this.finding);
  final ValidationFinding finding;
}

String _pointerChild(String parent, String name) =>
    '$parent/${name.replaceAll('~', '~0').replaceAll('/', '~1')}';

SchemaSource _sourceChild(SchemaSource source, String name) =>
    SchemaSource(source.document, _pointerChild(source.pointer, name));

// Dart's String.compareTo orders UTF-16 units. The checked profile specifies
// decoded Unicode scalars, including supplementary keys, without normalization.
int _validationScalarCompare(String a, String b) {
  final left = a.runes.iterator;
  final right = b.runes.iterator;
  while (left.moveNext()) {
    if (!right.moveNext()) return 1;
    final order = left.current.compareTo(right.current);
    if (order != 0) return order;
  }
  return right.moveNext() ? -1 : 0;
}

List<String> _orderedKeys(JsonObject value) =>
    value.values.keys.toList()..sort(_validationScalarCompare);

final class _Annotations {
  final Set<String> properties = {};
  final Set<int> items = {};
}

final class _Evaluated {
  const _Evaluated(this.valid, this.annotations);
  final bool valid;
  final _Annotations annotations;
}

final class _ValidationSession {
  int steps = _validationLimits.maxEvaluationSteps;
  int equalities = _validationLimits.maxEqualitySteps;
  int depth = 0;
  List<ValidationFinding> findings = [];
  final Map<int, Set<JsonValue>> active = {};

  Never fail(SchemaSource source, String path, String message) =>
      throw _EvaluationAbort(ValidationFinding(source, path, message));

  void spend(SchemaSource source, String path) {
    if (steps == 0)
      fail(source, path, 'schema evaluation work budget exhausted');
    steps--;
  }

  bool mismatch(SchemaSource source, String path, String message) {
    if (_validationLimits.maxErrors == 0 ||
        findings.length < _validationLimits.maxErrors) {
      findings.add(ValidationFinding(source, path, message));
    }
    return false;
  }

  _Exact number(JsonNumber value, SchemaSource source, String path) {
    if (value.token.length > _validationLimits.maxNumberBytes) {
      fail(source, path, 'numeric operand budget exhausted');
    }
    // Exact values own their immutable parsed coefficients/exponents. Trials
    // reuse those values but never bypass the caller's numeric-byte ceiling.
    return value._exact;
  }

  int compareNumbers(
    JsonNumber a,
    JsonNumber b,
    SchemaSource source,
    String path,
  ) => number(a, source, path).compare(number(b, source, path));

  void merge(
    _Annotations into,
    _Annotations other,
    SchemaSource source,
    String path,
  ) {
    for (final name in other.properties.toList()..sort(_validationScalarCompare)) {
      spend(source, path); // Charge candidates, including duplicate insertions.
      into.properties.add(name);
    }
    for (final index in other.items.toList()..sort()) {
      spend(source, path);
      into.items.add(index);
    }
  }

  ValidationResult validate(int root, JsonValue value) {
    findings = [];
    try {
      final result = node(root, value, '');
      return ValidationResult(
        result.valid ? ValidationStatus.valid : ValidationStatus.invalid,
        findings,
      );
    } on _EvaluationAbort catch (error) {
      return ValidationResult(ValidationStatus.evaluationFailure, [
        error.finding,
      ]);
    }
  }

  // The codec's branch selection uses the same counters as root validation.
  bool trial(int index, JsonValue value, String path) =>
      _trial(index, value, path).valid;

  _Evaluated _trial(int index, JsonValue value, String path) {
    final parent = findings;
    findings = [];
    try {
      return node(index, value, path);
    } finally {
      findings = parent;
    }
  }

  _Evaluated node(int index, JsonValue value, String path) {
    final owner = _validationNodes[index];
    spend(owner.source, path);
    if (depth >= _validationLimits.maxDepth) {
      fail(owner.source, path, 'schema evaluation depth budget exhausted');
    }
    final instances = active.putIfAbsent(index, HashSet<JsonValue>.identity);
    if (!instances.add(value)) {
      fail(owner.source, path, 'nonproductive recursive schema evaluation');
    }
    depth++;
    try {
      var valid = true;
      final local = _Annotations();
      for (final check in owner.checks) {
        spend(check.source, path);
        final accepted = run(owner, check, value, path, local);
        valid = accepted && valid; // Mismatches do not hide later failures.
      }
      return _Evaluated(valid, valid ? local : _Annotations());
    } finally {
      depth--;
      instances.remove(value);
    }
  }

  bool run(
    _ValidationNode owner,
    _Check check,
    JsonValue value,
    String path,
    _Annotations local,
  ) {
    final source = check.source;
    var produced = _Annotations();
    var result = true;
    var report = false;
    switch (check.op) {
      case _Op.always:
        result = check.value;
        report = true;
      case _Op.type:
        result = check.types.any(
          (type) => switch (type) {
            _JsonType.nullValue => value is JsonNull,
            _JsonType.boolean => value is JsonBoolean,
            _JsonType.string => value is JsonString,
            _JsonType.array => value is JsonArray,
            _JsonType.object => value is JsonObject,
            _JsonType.number => value is JsonNumber,
            _JsonType.integer =>
              value is JsonNumber &&
                  (check.types.contains(_JsonType.number) ||
                      number(value, source, path).sign == 0 ||
                      number(value, source, path).exponent >= BigInt.zero),
          },
        );
        report = true;
      case _Op.ref:
        final child = node(check.target, value, path);
        result = child.valid;
        produced = child.annotations; // Moving is not merging.
      case _Op.properties:
        if (value is JsonObject) {
          for (final property in check.properties) {
            spend(source, path);
            final child = value.values[property.name];
            if (child != null) {
              final accepted = node(
                property.target,
                child,
                _pointerChild(path, property.name),
              );
              result = accepted.valid && result;
              produced.properties.add(property.name);
            }
          }
        }
      case _Op.additionalProperties:
      case _Op.additionalPropertiesWithPatterns:
        if (value is JsonObject) {
          final patterns = check.op == _Op.additionalPropertiesWithPatterns
              ? owner.checks
                    .firstWhere((c) => c.op == _Op.patternProperties)
                    .patterns
              : const <_PatternProperty>[];
          members:
          for (final name in _orderedKeys(value)) {
            spend(source, path);
            if (check.names.contains(name)) continue;
            for (final p in patterns) {
              spend(source, path);
              if (pattern(p.pattern, name, source, path)) continue members;
            }
            final accepted = node(
              check.target,
              value.values[name]!,
              _pointerChild(path, name),
            );
            result = accepted.valid && result;
            produced.properties.add(name);
          }
        }
      case _Op.required:
        if (value is JsonObject) {
          for (final name in check.names) {
            spend(source, path);
            if (!value.values.containsKey(name)) {
              result = mismatch(source, path, 'required property is absent');
            }
          }
        }
      case _Op.items:
        if (value is JsonArray) {
          for (var i = check.start; i < value.values.length; i++) {
            spend(source, path);
            final accepted = node(
              check.target,
              value.values[i],
              _pointerChild(path, '$i'),
            );
            result = accepted.valid && result;
            produced.items.add(i);
          }
        }
      case _Op.prefixItems:
        if (value is JsonArray) {
          for (
            var i = 0;
            i < check.targets.length && i < value.values.length;
            i++
          ) {
            spend(source, path);
            final accepted = node(
              check.targets[i],
              value.values[i],
              _pointerChild(path, '$i'),
            );
            result = accepted.valid && result;
            produced.items.add(i);
          }
        }
      case _Op.allOf:
      case _Op.anyOf:
      case _Op.oneOf:
        final passing = <_Annotations>[];
        for (final target in check.targets) {
          spend(source, path);
          final child = check.op == _Op.allOf
              ? node(target, value, path)
              : _trial(target, value, path);
          if (child.valid) passing.add(child.annotations);
        }
        result = check.op == _Op.allOf
            ? passing.length == check.targets.length
            : check.op == _Op.anyOf
            ? passing.isNotEmpty
            : passing.length == 1;
        if (result) {
          for (final annotations in passing) {
            merge(produced, annotations, source, path);
          }
        }
        report = check.op != _Op.allOf;
      case _Op.not:
        result = !trial(check.target, value, path);
        report = true;
      case _Op.ifValue:
        final condition = _trial(check.condition, value, path);
        final target = condition.valid ? check.thenTarget : check.elseTarget;
        if (condition.valid) merge(local, condition.annotations, source, path);
        if (target != null) {
          final child = node(target, value, path);
          result = child.valid;
          produced = child.annotations;
        }
      case _Op.dependentRequired:
        if (value is JsonObject) {
          for (final (trigger, names) in check.dependencies) {
            spend(source, path);
            if (value.values.containsKey(trigger)) {
              for (final name in names) {
                spend(source, path);
                if (!value.values.containsKey(name)) {
                  result = mismatch(
                    _sourceChild(source, trigger),
                    path,
                    'dependent required property is absent',
                  );
                }
              }
            }
          }
        }
      case _Op.dependentSchemas:
        final passing = <_Annotations>[];
        if (value is JsonObject) {
          for (final dependency in check.properties) {
            spend(source, path);
            if (value.values.containsKey(dependency.name)) {
              final child = node(dependency.target, value, path);
              result = child.valid && result;
              if (child.valid) passing.add(child.annotations);
            }
          }
        }
        if (result) {
          for (final annotations in passing)
            merge(produced, annotations, source, path);
        }
      case _Op.contains:
        if (value is JsonArray) {
          for (var i = 0; i < value.values.length; i++) {
            spend(source, path);
            if (trial(
              check.target,
              value.values[i],
              _pointerChild(path, '$i'),
            )) {
              produced.items.add(i);
            }
          }
          final count = produced.items.length;
          final minimum = check.minimumCount;
          final maximum = check.maximumCount;
          final minSource = minimum == null
              ? source
              : _sourceChild(owner.source, 'minContains');
          final maxSource = _sourceChild(owner.source, 'maxContains');
          // The default 1 has no source numeric operand, even under a zero
          // numeric-byte limit. Explicit -0 and huge exponents remain exact.
          final lower = minimum == null
              ? count >= 1
              : _Exact.parse(
                      '$count',
                    ).compare(number(minimum, minSource, path)) >=
                    0;
          final upper =
              maximum == null ||
              _Exact.parse(
                    '$count',
                  ).compare(number(maximum, maxSource, path)) <=
                  0;
          if (count != 0 ||
              (minimum != null && number(minimum, minSource, path).sign == 0)) {
            merge(local, produced, source, path);
          }
          produced = _Annotations();
          if (!lower) mismatch(minSource, path, 'too few contains matches');
          if (!upper) mismatch(maxSource, path, 'too many contains matches');
          result = lower && upper;
        }
      case _Op.patternProperties:
        if (value is JsonObject) {
          for (final name in _orderedKeys(value)) {
            spend(source, path);
            for (final p in check.patterns) {
              spend(source, path);
              if (pattern(p.pattern, name, source, path)) {
                final accepted = node(
                  p.target,
                  value.values[name]!,
                  _pointerChild(path, name),
                );
                result = accepted.valid && result;
                produced.properties.add(name);
              }
            }
          }
        }
      case _Op.propertyNames:
        if (value is JsonObject) {
          for (final name in _orderedKeys(value)) {
            spend(source, path);
            // This fresh key object keeps its identity throughout the child
            // evaluation, including refs. It never aliases a member value.
            final accepted = node(
              check.target,
              JsonString(name),
              _pointerChild(path, name),
            );
            result = accepted.valid && result;
          }
        }
      case _Op.unevaluatedProperties:
        if (value is JsonObject) {
          for (final name in _orderedKeys(value)) {
            spend(source, path);
            if (!local.properties.contains(name)) {
              final accepted = node(
                check.target,
                value.values[name]!,
                _pointerChild(path, name),
              );
              result = accepted.valid && result;
              produced.properties.add(name);
            }
          }
        }
      case _Op.unevaluatedItems:
        if (value is JsonArray) {
          for (var i = 0; i < value.values.length; i++) {
            spend(source, path);
            if (!local.items.contains(i)) {
              final accepted = node(
                check.target,
                value.values[i],
                _pointerChild(path, '$i'),
              );
              result = accepted.valid && result;
              produced.items.add(i);
            }
          }
        }
      case _Op.bound:
        if (value is JsonNumber) {
          final order = compareNumbers(value, check.number!, source, path);
          result = check.maximum
              ? order < 0 || (!check.exclusive && order == 0)
              : order > 0 || (!check.exclusive && order == 0);
        }
        report = true;
      case _Op.multipleOf:
        if (value is JsonNumber) {
          // Coefficient/token lengths bound arithmetic; symbolic exponents
          // never expand. V2 visit costs do not meter numeric string copying.
          result = number(
            value,
            source,
            path,
          ).multipleOf(number(check.number!, source, path), (_) {});
        }
        report = true;
      case _Op.count:
        final int? count = switch ((check.countType, value)) {
          (_JsonType.string, JsonString(value: final text)) =>
            text.runes.length,
          (_JsonType.array, JsonArray(values: final items)) => items.length,
          (_JsonType.object, JsonObject(values: final members)) =>
            members.length,
          _ => null,
        };
        if (count != null) {
          final order = _Exact.parse(
            '$count',
          ).compare(number(check.number!, source, path));
          result = check.maximum ? order <= 0 : order >= 0;
        }
        report = true;
      case _Op.constValue:
        result = equal(value, check.literal!, source, path);
        report = true;
      case _Op.enumValue:
        result = false;
        for (final literal in check.literals) {
          spend(source, path);
          if (equal(value, literal, source, path)) {
            result = true;
            break;
          }
        }
        report = true;
      case _Op.uniqueItems:
        if (value is JsonArray) {
          pairs:
          for (var i = 0; i < value.values.length; i++) {
            spend(source, path);
            for (var j = 0; j < i; j++) {
              spend(source, path);
              if (equal(value.values[i], value.values[j], source, path)) {
                result = false;
                break pairs;
              }
            }
          }
        }
        report = true;
      case _Op.pattern:
        if (value is JsonString)
          result = pattern(check.pattern!, value.value, source, path);
        report = true;
    }
    if (result) {
      merge(local, produced, source, path);
    } else if (report) {
      mismatch(source, path, '${check.op.name} assertion rejected the value');
    }
    return result;
  }

  bool equal(
    JsonValue left,
    JsonValue right,
    SchemaSource source,
    String path,
  ) {
    final pending = <(JsonValue, JsonValue, int)>[(left, right, 0)];
    while (pending.isNotEmpty) {
      final (a, b, level) = pending.removeLast();
      if (equalities == 0 || level > _validationLimits.maxDepth) {
        fail(source, path, 'structural equality budget exhausted');
      }
      equalities--;
      switch ((a, b)) {
        case (JsonNull(), JsonNull()):
          break;
        case (JsonBoolean(value: final av), JsonBoolean(value: final bv)):
          if (av != bv) return false;
        case (JsonString(value: final av), JsonString(value: final bv)):
          if (av != bv) return false;
        case (JsonNumber av, JsonNumber bv):
          if (compareNumbers(av, bv, source, path) != 0) return false;
        case (JsonArray(values: final av), JsonArray(values: final bv)):
          if (av.length != bv.length) return false;
          if (av.length + pending.length > equalities)
            fail(source, path, 'structural equality budget exhausted');
          for (var i = av.length - 1; i >= 0; i--)
            pending.add((av[i], bv[i], level + 1));
        case (JsonObject av, JsonObject bv):
          if (av.values.length != bv.values.length) return false;
          if (av.values.length + pending.length > equalities)
            fail(source, path, 'structural equality budget exhausted');
          for (final key in _orderedKeys(av).reversed) {
            final other = bv.values[key];
            if (other == null) return false;
            pending.add((av.values[key]!, other, level + 1));
          }
        default:
          return false;
      }
    }
    return true;
  }

  bool pattern(
    _Pattern program,
    String text,
    SchemaSource source,
    String path,
  ) {
    // Portable Thompson NFA. Every loop, enqueue (even a duplicate), state pop
    // and range visit has the checked profile's exact shared step cost.
    final scalars = text.runes.iterator;
    var seeds = <int>[];
    var position = 0;
    var hasScalar = scalars.moveNext();
    while (true) {
      spend(source, path);
      final seen = <int>{};
      final pending = <int>[];
      final consuming = <int>[];
      void enqueue(int index) {
        spend(source, path);
        if (seen.add(index)) pending.add(index);
      }

      enqueue(program.start);
      for (final index in seeds) enqueue(index);
      while (pending.isNotEmpty) {
        spend(source, path);
        final index = pending.removeLast();
        final state = program.states[index];
        switch (state.op) {
          case _PatternOp.match:
            return true;
          case _PatternOp.char:
            consuming.add(index);
          case _PatternOp.split:
            enqueue(state.second);
            enqueue(state.target);
          case _PatternOp.jump:
            enqueue(state.target);
          case _PatternOp.start:
            if (position == 0) enqueue(state.target);
          case _PatternOp.end:
            if (!hasScalar) enqueue(state.target);
        }
      }
      if (!hasScalar) return false;
      seeds = [];
      for (final index in consuming) {
        final state = program.states[index];
        for (final (low, high) in state.ranges) {
          spend(source, path);
          if (scalars.current < low) break;
          if (scalars.current <= high) {
            seeds.add(state.target);
            break;
          }
        }
      }
      position++;
      hasScalar = scalars.moveNext();
    }
  }
}

enum _PatternOp { match, char, split, jump, start, end }

final class _PatternState {
  const _PatternState(
    this.op, {
    this.target = 0,
    this.second = 0,
    this.ranges = const [],
  });
  final _PatternOp op;
  final int target;
  final int second;
  final List<(int, int)> ranges;
}

final class _Pattern {
  const _Pattern(this.start, this.states);
  final int start;
  final List<_PatternState> states;
}
