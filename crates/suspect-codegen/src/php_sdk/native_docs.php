<?php
declare(strict_types=1);
require __DIR__ . '/vendor/autoload.php';

use FixtureSdk\JsonValue;
function check(bool $condition, string $message): void { if (!$condition) { throw new RuntimeException($message); } }
$package = $argv[1] ?? throw new RuntimeException('provide installed package path');
$text = file_get_contents($package . '/docs/coverage.json');
if ($text === false) { throw new RuntimeException('missing native coverage'); }
$coverage = JsonValue::parse($text)->asObject();
$namespace = $coverage['namespace']->asString();
$codecs = new ReflectionClass($namespace . '\\Codecs');
foreach ($coverage['models']->asArray() as $entry) {
    $model = $entry->asObject(); $name = $model['name']->asString();
    foreach ($model['codecs']->asObject() as $codec) {
        check($codecs->hasMethod($codec->asString()), 'documented codec not actually exported');
        check($codecs->getMethod($codec->asString())->isPublic(), 'codec not public');
    }
    if ($model['constructor']->kind === FixtureSdk\JsonKind::Null) { continue; }
    $class = new ReflectionClass($namespace . '\\' . $name);
    check($class->getDocComment() !== false, 'missing model PHPDoc');
    $constructor = $class->getConstructor(); check($constructor !== null, 'documented constructor missing');
    if ($constructor === null) { throw new RuntimeException('constructor missing'); }
    $expected = array_map(static fn (JsonValue $v): string => $v->asString(), $model['constructor']->asObject()['parameters']->asArray());
    if ($model['constructor']->asObject()['extras']->kind !== FixtureSdk\JsonKind::Null) { $expected[] = 'extra'; }
    check(array_map(static fn (ReflectionParameter $p): string => $p->getName(), $constructor->getParameters()) === $expected, 'constructor docs do not match native symbols');
}
$client = new ReflectionClass($namespace . '\\Client');
foreach ($coverage['operations']->asArray() as $entry) {
    $operation = $entry->asObject(); $method = $client->getMethod($operation['method']->asString());
    check($method->isPublic() && $method->getDocComment() !== false, 'missing native operation PHPDoc');
    check(str_contains((string) $method->getDocComment(), '@throws SdkError'), 'missing typed failure PHPDoc');
    foreach ($operation['responses']->asArray() as $entry) {
        $response = $entry->asObject(); $class = new ReflectionClass($namespace . '\\' . $response['type']->asString());
        check($class->hasProperty('body') && $class->hasProperty('response'), 'response reference names missing');
    }
}
$html = file_get_contents($package . '/docs/index.html');
if ($html === false) { throw new RuntimeException('missing browsable reference'); }
libxml_use_internal_errors(true);
$dom = new DOMDocument(); check($dom->loadHTML($html), 'invalid reference HTML');
check($dom->getElementsByTagName('script')->length === 0, 'source prose became executable HTML');
$ids = [];
foreach ($dom->getElementsByTagName('*') as $element) {
    $id = $element->getAttribute('id');
    if ($id !== '') { check(!isset($ids[$id]), 'duplicate reference anchor'); $ids[$id] = true; }
}
foreach ($dom->getElementsByTagName('a') as $link) {
    $href = $link->getAttribute('href');
    if (str_starts_with($href, '#')) { check(isset($ids[substr($href, 1)]), 'unresolved native reference link'); }
}
echo 'native reflection/PHPDoc and browsable reference links passed', PHP_EOL;
