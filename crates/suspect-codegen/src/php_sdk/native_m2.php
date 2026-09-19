<?php
declare(strict_types=1);

require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\{Absent, Codecs, JsonNumber, JsonValue, JsonError, ValidationError, Widget,
    WidgetInput, WidgetPatch, StandardPayload, SecurePayload, WidgetNode};

function check(bool $condition, string $message): void
{
    if (!$condition) { throw new RuntimeException($message); }
}

/** @param Closure(): mixed $action */
function invalid(Closure $action): void
{
    try { $action(); }
    catch (JsonError|ValidationError|TypeError $e) { return; }
    throw new RuntimeException('invalid value accepted');
}

$widget = new Widget(amount: JsonNumber::fromString('9007199254740993.000000000000000001'), id: 'w1', payload: new StandardPayload(text: 'plain'), meta: null, child: new WidgetNode(label: 'root'));
$encoded = $widget->toJson();
check(str_contains($encoded, '9007199254740993.000000000000000001'), 'decimal lost');
$decoded = Widget::fromJson($encoded);
check($decoded->meta === null, 'explicit null lost');
check($decoded->payload instanceof StandardPayload && $decoded->payload->text === 'plain', 'native union lost');
check($decoded->child instanceof WidgetNode && $decoded->child->child === Absent::Value, 'recursive omission lost');
$decoded->payload = new SecurePayload(vault: 'v1');
check(str_contains($decoded->toJson(), '"kind":"secure"'), 'constant tag lost');
$decoded->meta = Absent::Value;
check(!str_contains($decoded->toJson(), '"meta"'), 'omission lost');
$decoded->extra['0'] = JsonValue::fromNumber(JsonNumber::fromString('1e-400'));
$decoded->extra['01'] = JsonValue::fromBool(true);
check(str_contains($decoded->toJson(), '"0":1e-400'), 'numeric member name lost');
invalid(static fn () => Widget::fromJson('{"id":"w","amount":1,"payload":{"kind":"invalid"}}'));
invalid(static fn () => Widget::fromJson('{"id":"w","amount":true,"payload":{"kind":"standard","text":"x"}}'));
invalid(static fn () => WidgetPatch::fromJson('{"amount":null}'));
$mutated = new WidgetInput(name: 'valid'); $mutated->name = '';
invalid(static fn () => $mutated->toJson());
$mutated->name = 'valid'; $mutated->extra['name'] = JsonValue::fromString('collision');
invalid(static fn () => $mutated->toJson());
$cycle = new WidgetNode(label: 'cycle'); $cycle->child = $cycle;
invalid(static fn () => $cycle->toJson());
foreach (['-', '+1', '01', '1.', '1e', '1e+', 'NaN', 'Infinity'] as $token) { invalid(static fn () => JsonNumber::fromString($token)); }
foreach (['{}true', '[1,]', '{"a":1,"\u0061":2}', '"\ud800"', 'true false', "\xC0\xAF"] as $text) { invalid(static fn () => JsonValue::parse($text)); }
foreach (['1.0', '1e3', '1e' . str_repeat('0', 50), '10e-' . str_repeat('0', 50) . '1'] as $token) { check(JsonNumber::fromString($token)->isInteger(), 'mathematical integer lost'); }
check(!JsonNumber::fromString('0.1e' . str_repeat('0', 50))->isInteger(), 'padded exponent coerced');
check(JsonNumber::fromString('1e' . str_repeat('0', 50))->toInt() === 1, 'padded conversion');
check(JsonNumber::fromString('100e-2')->compare(JsonNumber::fromString('1.00')) === 0, 'exact equality');
$huge = JsonNumber::fromString('1e999999999999999999999999999999999');
check($huge->isInteger() && $huge->compare(JsonNumber::fromString('9e999999999999999999999999999999998')) > 0, 'symbolic comparison');
invalid(static fn () => $huge->toInt());
check(JsonValue::parse('{"a":{},"b":[],"c":true,"d":1}')->toJson() === '{"a":{},"b":[],"c":true,"d":1}', 'JSON domains');
echo 'native typed M2 models, exact JSON, presence and mutable codec checks passed', PHP_EOL;
