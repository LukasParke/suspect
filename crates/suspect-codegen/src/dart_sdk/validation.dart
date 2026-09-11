// A selected program need not instantiate every optional opcode operand.
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

/// Evaluate a source-selected root of the emitted, checked portable program.
///
/// This executes compiled instructions, not JSON Schema keywords. Unknown roots
/// and exhausted limits return [ValidationStatus.evaluationFailure].
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
}

enum _JsonType { nullValue, boolean, integer, number, string, array, object }

final class _Property {
  const _Property(this.name, this.target);
  final String name;
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

final class _ValidationSession {
  int steps = _validationLimits.maxEvaluationSteps;
  int equalities = _validationLimits.maxEqualitySteps;
  int numeric = _validationLimits.maxEvaluationSteps;
  int depth = 0;
  List<ValidationFinding> findings = [];
  final Map<int, Set<JsonValue>> active = {};

  Never fail(SchemaSource source, String path, String message) =>
      throw _EvaluationAbort(ValidationFinding(source, path, message));
  void spend(SchemaSource source, String path, [int amount = 1]) {
    steps -= amount;
    if (steps < 0) {
      fail(source, path, 'schema evaluation work budget exhausted');
    }
  }

  bool mismatch(SchemaSource source, String path, String message) {
    if (findings.length < _validationLimits.maxErrors) {
      findings.add(ValidationFinding(source, path, message));
    }
    return false;
  }

  _Exact number(JsonNumber value, SchemaSource source, String path) {
    if (value.token.length > _validationLimits.maxNumberBytes) {
      fail(source, path, 'numeric operand budget exhausted');
    }
    return value._exact;
  }

  void numericSpend(int amount, SchemaSource source, String path) {
    numeric -= amount;
    if (numeric < 0) {
      fail(source, path, 'exact arithmetic work budget exhausted');
    }
  }

  int compareNumbers(
    JsonNumber left,
    JsonNumber right,
    SchemaSource source,
    String path,
  ) {
    final a = number(left, source, path);
    final b = number(right, source, path);
    numericSpend(left.token.length + right.token.length, source, path);
    return a.compare(b);
  }

  ValidationResult validate(int root, JsonValue value) {
    findings = [];
    try {
      final valid = node(root, value, '');
      if (!valid && findings.isEmpty) {
        mismatch(
          _validationNodes[root].source,
          '',
          'source schema rejected the value',
        );
      }
      return ValidationResult(
        valid ? ValidationStatus.valid : ValidationStatus.invalid,
        findings,
      );
    } on _EvaluationAbort catch (error) {
      return ValidationResult(ValidationStatus.evaluationFailure, [
        error.finding,
      ]);
    }
  }

  bool trial(int index, JsonValue value, String path) {
    final parent = findings;
    findings = [];
    try {
      return node(index, value, path);
    } finally {
      findings = parent;
    }
  }

  bool node(int index, JsonValue value, String path) {
    final node = _validationNodes[index];
    spend(node.source, path);
    if (depth >= _validationLimits.maxDepth) {
      fail(node.source, path, 'schema evaluation depth budget exhausted');
    }
    final instances = active.putIfAbsent(index, HashSet<JsonValue>.identity);
    if (!instances.add(value)) {
      fail(node.source, path, 'nonproductive recursive schema evaluation');
    }
    depth++;
    try {
      var valid = true;
      for (final check in node.checks) {
        spend(check.source, path);
        // Keep evaluating after a mismatch. Failures in later logical branches
        // must neither disappear nor be inverted into a valid outcome.
        final accepted = run(check, value, path);
        valid = accepted && valid;
      }
      return valid;
    } finally {
      depth--;
      instances.remove(value);
    }
  }

  bool run(_Check check, JsonValue value, String path) {
    final source = check.source;
    var result = true;
    switch (check.op) {
      case _Op.always:
        result = check.value;
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
      case _Op.ref:
        result = node(check.target, value, path);
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
              result = accepted && result;
            }
          }
        }
      case _Op.additionalProperties:
        if (value is JsonObject) {
          for (final entry in value.values.entries) {
            spend(source, path);
            if (!check.names.contains(entry.key)) {
              final accepted = node(
                check.target,
                entry.value,
                _pointerChild(path, entry.key),
              );
              result = accepted && result;
            }
          }
        }
      case _Op.required:
        if (value is JsonObject) {
          for (final name in check.names) {
            spend(source, path);
            if (!value.values.containsKey(name)) {
              result = mismatch(
                source,
                _pointerChild(path, name),
                'required property is absent',
              );
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
            result = accepted && result;
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
            result = accepted && result;
          }
        }
      case _Op.allOf:
      case _Op.anyOf:
      case _Op.oneOf:
        var count = 0;
        for (final target in check.targets) {
          spend(source, path);
          final accepted = check.op == _Op.allOf
              ? node(target, value, path)
              : trial(target, value, path);
          if (accepted) {
            count++;
          }
        }
        result = check.op == _Op.allOf
            ? count == check.targets.length
            : check.op == _Op.anyOf
            ? count > 0
            : count == 1;
      case _Op.not:
        result = !trial(check.target, value, path);
      case _Op.bound:
        if (value is JsonNumber) {
          final order = compareNumbers(value, check.number!, source, path);
          result = check.maximum
              ? order < 0 || (!check.exclusive && order == 0)
              : order > 0 || (!check.exclusive && order == 0);
        }
      case _Op.multipleOf:
        if (value is JsonNumber) {
          result = number(value, source, path).multipleOf(
            number(check.number!, source, path),
            (cost) {
              numericSpend(cost, source, path);
            },
          );
        }
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
      case _Op.constValue:
        result = equal(value, check.literal!, source, path);
      case _Op.enumValue:
        result = false;
        for (final literal in check.literals) {
          spend(source, path);
          if (equal(value, literal, source, path)) {
            result = true;
            break;
          }
        }
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
      case _Op.pattern:
        if (value is JsonString) {
          result = pattern(check.pattern!, value.value, source, path);
        }
    }
    if (!result) {
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
      final (a, b, depth) = pending.removeLast();
      if (--equalities < 0 || depth > _validationLimits.maxDepth) {
        fail(source, path, 'structural equality budget exhausted');
      }
      switch ((a, b)) {
        case (JsonNull(), JsonNull()):
          break;
        case (JsonBoolean(value: final av), JsonBoolean(value: final bv)):
          if (av != bv) {
            return false;
          }
        case (JsonString(value: final av), JsonString(value: final bv)):
          if (av != bv) {
            return false;
          }
        case (JsonNumber av, JsonNumber bv):
          if (compareNumbers(av, bv, source, path) != 0) {
            return false;
          }
        case (JsonArray(values: final av), JsonArray(values: final bv)):
          if (av.length != bv.length) {
            return false;
          }
          if (av.length + pending.length > equalities) {
            fail(source, path, 'structural equality budget exhausted');
          }
          for (var i = av.length - 1; i >= 0; i--) {
            pending.add((av[i], bv[i], depth + 1));
          }
        case (JsonObject(values: final av), JsonObject(values: final bv)):
          if (av.length != bv.length) {
            return false;
          }
          if (av.length + pending.length > equalities) {
            fail(source, path, 'structural equality budget exhausted');
          }
          for (final entry in av.entries) {
            final other = bv[entry.key];
            if (other == null) {
              return false;
            }
            pending.add((entry.value, other, depth + 1));
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
    // This is the compiler's Thompson NFA, never a host RegExp translation.
    // One active-state set is advanced for each Unicode scalar; new matches
    // may begin at every offset. All epsilon/range visits share schema work.
    final scalars = text.runes.iterator;
    var current = <int>{};
    var position = 0;
    var hasScalar = scalars.moveNext();
    while (true) {
      current.add(program.start);
      final pending = current.toList();
      final seen = <int>{};
      final consuming = <int>[];
      while (pending.isNotEmpty) {
        spend(source, path);
        final index = pending.removeLast();
        if (!seen.add(index)) {
          continue;
        }
        final state = program.states[index];
        switch (state.op) {
          case _PatternOp.match:
            return true;
          case _PatternOp.char:
            consuming.add(index);
          case _PatternOp.split:
            pending.add(state.target);
            pending.add(state.second);
          case _PatternOp.jump:
            pending.add(state.target);
          case _PatternOp.start:
            if (position == 0) {
              pending.add(state.target);
            }
          case _PatternOp.end:
            if (!hasScalar) {
              pending.add(state.target);
            }
        }
      }
      if (!hasScalar) {
        return false;
      }
      current = {};
      for (final index in consuming) {
        final state = program.states[index];
        for (final (low, high) in state.ranges) {
          spend(source, path);
          if (low <= scalars.current && scalars.current <= high) {
            current.add(state.target);
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
