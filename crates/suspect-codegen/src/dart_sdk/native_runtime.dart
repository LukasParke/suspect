import 'package:generated_sdk/generated_sdk.dart';
import 'support.dart';

void sharedVectors() {
  __VECTORS__
}

void resourceAndReferenceCases() {
  // All alternatives must be evaluated, even following success or a mismatch.
  final expensive = JsonArray([JsonNumber.parse('1' * 65)]);
  for (final result in [anyFailureCodec.validate(expensive), oneFailureCodec.validate(expensive),
      allFailureCodec.validate(expensive), negatedFailureCodec.validate(expensive)]) {
    check(result.status == ValidationStatus.evaluationFailure, 'logical composition suppressed/inverted a numeric failure');
    check(result.findings.single.instancePath == '/0', 'numeric failure retains instance path');
  }
  check(fails<CodecException>(() => anyFailureCodec.fromJson(expensive)).kind == CodecFailureKind.evaluationFailure, 'codec preserves evaluation failure');
  check(fails<CodecException>(() => vector01Codec.decode('1' * 65)).kind == CodecFailureKind.resourceLimit, 'parser token failure remains distinct');

  final branchValue = JsonArray(List.generate(9, (_) => JsonString('a')));
  check(branchPickCodec.validate(branchValue).isValid, 'standalone union validation fits declared budget');
  check(fails<CodecException>(() => branchPickCodec.fromJson(branchValue)).kind == CodecFailureKind.evaluationFailure,
      'native branch selection must share the root validation session');
  final repeated = JsonString('x' * 50);
  convertedArrayCodec.toJson([repeated]);
  check(fails<CodecException>(() => convertedArrayCodec.toJson([repeated, repeated, repeated])).kind == CodecFailureKind.resourceLimit,
      'one conversion budget charges shared/copied JSON again, not once per identity');
  check(fails<CodecException>(() => convertedArrayCodec.decode('["${'x' * 50}","${'x' * 50}","${'x' * 50}"]')).kind == CodecFailureKind.resourceLimit,
      'decode conversion shares copied string work too');
  final unique = JsonArray(List.generate(9, (i) => JsonNumber.fromInt(i)));
  check(uniqueBudgetCodec.validate(unique).status == ValidationStatus.evaluationFailure, 'equality trials share a budget');
  final unknown = validateJson(const SchemaSource('https://unknown.test/schema', '/not-selected'), const JsonNull());
  check(unknown.status == ValidationStatus.evaluationFailure, 'unknown root cannot be ordinary invalidity');

  final nullable = presenceCodec.decode('{"requiredNullable":null,"requiredValue":"x"}');
  check(nullable.requiredNullable == null && nullable.optionalNullable is Absent<String?> && nullable.optionalValue is Absent<String>, 'all four presence states');
  nullable.optionalNullable = const Present(null);
  check(presenceCodec.encode(nullable).contains('"optionalNullable":null'), 'explicit nullable presence encodes');
  for (final invalid in ['{"requiredValue":"x"}', '{"requiredNullable":null,"requiredValue":null}',
      '{"requiredNullable":null,"requiredValue":"x","optionalValue":null}']) {
    check(fails<CodecException>(() => presenceCodec.decode(invalid)).kind == CodecFailureKind.invalid, 'presence schema requiredness');
  }
  final extra = typedExtrasCodec.decode('{"id":"x","count":1.0}');
  check(extra.extraFields['count']!.toBigInt() == BigInt.one && extra.extraFields['count']!.token == '1.0', 'typed exact extras');
  extra.extraFields['id'] = JsonInteger.fromInt(2);
  check(fails<CodecException>(() => typedExtrasCodec.encode(extra)).kind == CodecFailureKind.conversion, 'typed extras collision');

  final recursive = externalNodeCodec.decode('{"label":"root","child":{"label":"leaf"}}');
  check(externalNodeCodec.encode(recursive).contains('"label":"leaf"'), 'external recursive identity');
  final sourceError = fails<CodecException>(() => externalNodeCodec.decode('{"label":"ok","child":{"label":""}}'));
  check(sourceError.source.document.endsWith('/external.json') && sourceError.source.pointer == '/components/schemas/ExternalNode/properties/label/minLength', 'external keyword provenance');
  check(sourceError.instancePath == '/child/label', 'external recursive instance pointer');
  check(narrowCodec.decode('{"label":"ok"}').label == 'ok', 'ref siblings retain native target');
  check(fails<CodecException>(() => narrowCodec.decode('{"label":"x"}')).source.pointer == '/components/schemas/Narrow/properties/label/minLength', 'ref siblings are not dropped');
  final escaped = escapedCodec.decode('{"a/b~c":"x","😀":"y"}');
  check(escapedCodec.encode(escaped).contains('"a/b~c":"x"'), 'decoded source and property keys preserved');
  final escapedError = fails<CodecException>(() => escapedCodec.decode('{"a/b~c":false}'));
  check(escapedError.instancePath == '/a~1b~0c', 'escaped instance pointer');

  final mixed = mixedLiteralCodec.decode('[1.0,true,null]');
  check(mixedLiteralCodec.encode(mixed) == '[1.0,true,null]', 'heterogeneous structural literal wrapper retains exact spelling');
  final intersection = intersectionCodec.decode('{"left":"x","right":1.00}');
  check(intersectionCodec.encode(intersection).contains('1.00'), 'allOf exact wrapper retains both domains');
  check(fails<CodecException>(() => intersectionCodec.decode('{"left":"x"}')).kind == CodecFailureKind.invalid, 'allOf requires every branch');
}

void main() {
  sharedVectors();
  resourceAndReferenceCases();
  print('DART_SHARED_17_RUNTIME_VECTORS_AND_RESOURCE_REFS_OK');
}
