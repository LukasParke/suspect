<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\{Absent, AdverseRoot, AdverseRootTypedMap, Codecs, JsonNumber, JsonValue, JsonError, ValidationError, Long, Short};
function check(bool $condition, string $message): void { if (!$condition) { throw new RuntimeException($message); } }
/** @param Closure(): mixed $action */
function invalid(Closure $action): void
{
    try { $action(); }
    catch (JsonError|ValidationError|TypeError $error) { return; }
    throw new RuntimeException('invalid model accepted');
}

$root = new AdverseRoot(requiredValue: null);
check($root->toJson() === '{"required_value":null}', 'required nullable must be present');
invalid(static fn () => AdverseRoot::fromJson('{}'));
invalid(static fn () => AdverseRoot::fromJson('{"required_value":null,"optional_nonnull":null}'));
$root->optionalNullable = null; $root->optionalNonnull = 'x';
check(str_contains($root->toJson(), '"optional_nullable":null'), 'explicit optional null');
$root->optionalNullable = Absent::Value;
check(!str_contains($root->toJson(), '"optional_nullable"'), 'omission distinct from null');
$root->literal = true; $root->scalarUnion = false;
$root->nullableArray = null;
check(str_contains($root->toJson(), '"nullable_array":null'), 'outer array nullability');
$root->nullableArray = [true, null, 'x'];
check(str_contains($root->toJson(), '"nullable_array":[true,null,"x"]'), 'inner union nullability');
$decoded = AdverseRoot::fromJson($root->toJson());
check($decoded->literal === true && $decoded->scalarUnion === false, 'boolean/numeric confusion');
$root->literal = JsonNumber::fromString('1.00'); $root->scalarUnion = JsonNumber::fromString('10e-000001');
check(str_contains($root->toJson(), '"literal":1.00'), 'mixed literal exact number');
$root->literal = 'invalid'; invalid(static fn () => $root->toJson()); $root->literal = 'one';
$root->scalarUnion = JsonNumber::fromString('0.1'); invalid(static fn () => $root->toJson()); $root->scalarUnion = 'x';
$root->typedMap = new AdverseRootTypedMap(extra: ['0' => JsonNumber::fromString('1.0'), '01' => JsonNumber::fromString('1e3')]);
check(str_contains($root->toJson(), '"typed_map":{"0":1.0,"01":1e3}'), 'typed map exact values/names');
$root->typedMap->extra['bad'] = JsonNumber::fromString('0.5'); invalid(static fn () => $root->toJson()); unset($root->typedMap->extra['bad']);
$long = new Long(name: 'long'); $root->union = $long;
check(AdverseRoot::fromJson($root->toJson())->union instanceof Long, 'selected native branch');
$long->name = 'x'; invalid(static fn () => $root->toJson());
// Although the serialized value would satisfy Short, a Long instance cannot
// silently change to the other public native carrier during encoding.
$root->union = new Short(name: 'x'); $long->name = 'long';
$root->mixedValue = Codecs::decodeMixedValue('{"name":"long"}');
check($root->mixedValue instanceof Long && str_contains($root->toJson(), '"mixed":{"name":"long"}'), 'object after literal union arm');
$root->parentValue = new Long(name: 'long'); invalid(static fn () => $root->toJson());
$root->parentValue->extra['second'] = JsonValue::null();
check(str_contains($root->toJson(), '"second":null'), 'parent and branch must both validate');
$root->stringUnion = 'x'; $root->arrayUnion = ['a'];
check(str_contains($root->toJson(), '"array_union":["a"]'), 'native collection union');
$root->arrayUnion = [JsonNumber::fromString('1.0')];
check(str_contains($root->toJson(), '"array_union":[1.0]'), 'numeric collection union');
$root->arrayUnion = []; // Exactly one branch accepts an empty list.
$root->fooBar = 'dash'; $root->fooBar2 = 'underscore'; $root->value0 = 'zero'; $root->thisValue = 'this';
$encoded = $root->toJson();
check(str_contains($encoded, '"foo-bar":"dash"') && str_contains($encoded, '"foo_bar":"underscore"') && str_contains($encoded, '"0":"zero"'), 'collision-safe source names');
invalid(static fn () => Codecs::decodeClosed('{"name":"x","unexpected":1}'));
foreach (['true', 'false', '1', '1.0', '"one"', 'null'] as $text) {
    if ($text === 'false') { invalid(static fn () => Codecs::decodeAdverseRootLiteral($text)); }
    else { check(Codecs::encodeAdverseRootLiteral(Codecs::decodeAdverseRootLiteral($text)) === $text, 'literal round trip'); }
}
echo 'adversarial native presence, mixed literals/unions, branch+parent validation, maps and source names passed', PHP_EOL;
