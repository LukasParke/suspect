<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

function check(bool $condition, string $message): void {
    if (!$condition) { throw new RuntimeException($message); }
}
function nativeType(?ReflectionType $type): string {
    if ($type instanceof ReflectionUnionType) {
        $names = array_map(nativeType(...), $type->getTypes());
    } elseif ($type instanceof ReflectionNamedType) {
        $name = $type->getName();
        $name = substr($name, (strrpos($name, '\\') ?: -1) + 1);
        $names = [$name];
        if ($type->allowsNull() && !in_array($name, ['null', 'mixed'], true)) { $names[] = 'null'; }
    } else { throw new RuntimeException('missing or unsupported reflection type'); }
    sort($names, SORT_STRING);
    return implode('|', $names);
}
function normalized(string $type): string {
    $names = explode('|', $type); sort($names, SORT_STRING); return implode('|', $names);
}
$root = $argv[1] ?? throw new RuntimeException('installed package path required');
$coverage = json_decode(file_get_contents($root . '/docs/coverage.json'), true, flags: JSON_THROW_ON_ERROR);
$html = file_get_contents($root . '/docs/index.html');
check($coverage['version'] === 'suspect.php-native.v2', 'wrong native coverage version');
$namespace = $coverage['namespace'];
foreach ($coverage['protocolTypes'] as $declaration) {
    $name = $declaration['name'];
    $class = new ReflectionClass($namespace . '\\' . $name);
    check($class->isFinal(), $name . ' must be final');
    $constructor = $class->getConstructor();
    check($constructor !== null, $name . ' constructor missing');
    $parameters = $constructor->getParameters();
    check(count($parameters) === count($declaration['parameters']), $name . ' constructor count');
    foreach ($declaration['parameters'] as $index => $expected) {
        $actual = $parameters[$index];
        check($actual->getName() === $expected['name'], $name . ' constructor order/name');
        check(nativeType($actual->getType()) === normalized($expected['native']), $name . ' constructor native type: ' . $expected['name']);
        check($actual->isOptional() === !$expected['required'], $name . ' constructor requiredness');
        if (str_contains($expected['phpdoc'], '<')) {
            check(str_contains($constructor->getDocComment() ?: '', '@param ' . $expected['phpdoc'] . ' $' . $expected['name']), $name . ' generic PHPDoc');
        }
    }
    foreach ($declaration['fields'] as $expected) {
        $property = $class->getProperty($expected['name']);
        check($property->isPublic(), $name . ' field visibility');
        check($property->isReadOnly() === $expected['readonly'], $name . ' mutability');
        check(nativeType($property->getType()) === normalized($expected['native']), $name . ' field native type: ' . $expected['name']);
        check(($property->getDeclaringClass()->getName() !== $class->getName()) === $expected['inherited'], $name . ' field inheritance');
    }
    check(str_contains($html, 'id="type-' . $name . '"'), $name . ' reference anchor');
}
foreach ($coverage['operations'] as $operation) {
    foreach (['Client', 'ClientInterface'] as $owner) {
        $method = new ReflectionMethod($namespace . '\\' . $owner, $operation['method']);
        $comment = $method->getDocComment() ?: '';
        check(str_contains($comment, '@throws SdkError'), $owner . ' classified error documentation');
        check(nativeType($method->getReturnType()) === normalized($operation['resultType']), $owner . ' actual result signature');
        foreach ($operation['responses'] as $response) {
            if (!$response['success']) { check(str_contains($comment, '@throws ' . $response['type']), 'missing concrete API-error documentation'); }
        }
    }
}
foreach ($coverage['models'] as $model) {
    foreach ($model['codecs'] as $name) {
        $method = new ReflectionMethod($namespace . '\\Codecs', $name);
        check($method->isPublic() && $method->isStatic(), 'codec visibility');
    }
    if ($model['constructor'] !== null) {
        $class = new ReflectionClass($namespace . '\\' . $model['name']);
        $actual = array_map(static fn(ReflectionParameter $p): string => $p->getName(), $class->getConstructor()->getParameters());
        $expected = $model['constructor']['parameters'];
        if ($model['constructor']['extras'] !== null) { $expected[] = $model['constructor']['extras']; }
        check($actual === $expected, 'source-native model constructor');
    }
}
preg_match_all('/href="#([^"]+)"/', $html, $links);
foreach ($links[1] as $anchor) { check(str_contains($html, 'id="' . $anchor . '"'), 'broken local reference: ' . $anchor); }
echo 'native protocol reflection/reference checked: ', count($coverage['protocolTypes']), ' declarations, ', count($coverage['operations']), ' operations, ', count($coverage['models']), ' source codecs', PHP_EOL;
