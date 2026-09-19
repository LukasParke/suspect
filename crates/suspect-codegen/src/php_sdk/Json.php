<?php
declare(strict_types=1);

namespace __NAMESPACE__;

/** Omission only. PHP null represents an explicit JSON null where admitted. */
enum Absent { case Value; }

/** Implemented by native model classes and string enums. */
interface Model { public function toJson(): string; }

/** Cooperative whole-call checkpoint, independent of HTTP or any event loop. @internal */
final readonly class CallControl
{
    /** @param \Closure(): void $checkpoint */
    public function __construct(private \Closure $checkpoint) {}
    public function check(): void { ($this->checkpoint)(); }
}

/** Distinct JSON domains; empty objects never turn into empty arrays. */
enum JsonKind: string
{
    case Null = 'null'; case Boolean = 'boolean'; case Number = 'number';
    case String = 'string'; case Array = 'array'; case Object = 'object';
}

/** JSON syntax, conversion and resource failures are separate from schema mismatches. */
class JsonError extends \RuntimeException
{
    public function __construct(public readonly string $kind, string $message, ?\Throwable $previous = null) { parent::__construct($message, 0, $previous); }
    /** Bounded inert default diagnostics; structured values remain separately available. @internal */
    public static function display(string $text): string
    {
        $truncated = strlen($text) > 512;
        $encoded = json_encode(substr($text, 0, 512), JSON_INVALID_UTF8_SUBSTITUTE | JSON_UNESCAPED_SLASHES);
        return ($encoded === false ? '"[unprintable]"' : $encoded) . ($truncated ? '…' : '');
    }
}

/** Finite JSON policy. Numbers remain exact even when their exponent is enormous. */
final readonly class JsonLimits
{
    public function __construct(
        public int $maxBytes = RuntimeConfig::MAX_JSON_BYTES,
        public int $maxDepth = RuntimeConfig::MAX_DEPTH,
        public int $maxNodes = RuntimeConfig::MAX_NODES,
        public int $maxNumberBytes = 65536,
        public ?CallControl $control = null,
    ) {
        if ($maxBytes < 1 || $maxBytes > RuntimeConfig::MAX_JSON_BYTES || $maxDepth < 1 || $maxDepth > RuntimeConfig::MAX_DEPTH || $maxNodes < 1 || $maxNodes > RuntimeConfig::MAX_NODES || $maxNumberBytes < 1 || $maxNumberBytes > 65536) {
            throw new JsonError('resource_limit', 'JSON limits must be positive and cannot exceed generated ceilings');
        }
    }
}

/** Immutable, lossless arbitrary JSON value. Floats and arbitrary PHP objects are excluded. */
final readonly class JsonValue
{
    /** @param null|bool|string|JsonNumber|array<array-key, self> $value */
    private function __construct(public JsonKind $kind, private null|bool|string|JsonNumber|array $value) {}

    public static function null(): self { return new self(JsonKind::Null, null); }
    public static function fromBool(bool $value): self { return new self(JsonKind::Boolean, $value); }
    public static function fromNumber(JsonNumber $value): self { return new self(JsonKind::Number, $value); }
    public static function fromString(string $value): self
    {
        if (strlen($value) > RuntimeConfig::MAX_JSON_BYTES) { throw new JsonError('resource_limit', 'string exceeds generated JSON ceiling'); }
        if (preg_match('//u', $value) !== 1) { throw new JsonError('syntax', 'string is not valid UTF-8'); }
        return new self(JsonKind::String, $value);
    }

    /** @param list<self> $values */
    public static function fromArray(array $values): self
    {
        if (count($values) > RuntimeConfig::MAX_NODES) { throw new JsonError('resource_limit', 'array exceeds generated node ceiling'); }
        if (!array_is_list($values)) { throw new JsonError('conversion', 'JSON arrays require contiguous zero-based list keys'); }
        return new self(JsonKind::Array, array_map(static fn (self $v): self => $v, $values));
    }

    /** Numeric PHP array keys are serialized as their original decimal string member names.
     * @param array<array-key, self> $members
     */
    public static function fromObject(array $members): self
    {
        if (count($members) > RuntimeConfig::MAX_NODES) { throw new JsonError('resource_limit', 'object exceeds generated node ceiling'); }
        foreach ($members as $key => $_) {
            if (strlen((string) $key) > RuntimeConfig::MAX_JSON_BYTES) { throw new JsonError('resource_limit', 'object name exceeds generated JSON ceiling'); }
            if (preg_match('//u', (string) $key) !== 1) { throw new JsonError('syntax', 'object name is not valid UTF-8'); }
        }
        return new self(JsonKind::Object, array_map(static fn (self $v): self => $v, $members));
    }

    public function asString(): string
    {
        if ($this->kind !== JsonKind::String || !is_string($this->value)) { throw new JsonError('conversion', 'expected JSON string'); }
        return $this->value;
    }
    public function asBool(): bool
    {
        if ($this->kind !== JsonKind::Boolean || !is_bool($this->value)) { throw new JsonError('conversion', 'expected JSON boolean'); }
        return $this->value;
    }
    public function asNumber(): JsonNumber
    {
        if ($this->kind !== JsonKind::Number || !$this->value instanceof JsonNumber) { throw new JsonError('conversion', 'expected JSON number'); }
        return $this->value;
    }
    /** @return list<self> */
    public function asArray(): array
    {
        if ($this->kind !== JsonKind::Array || !is_array($this->value)) { throw new JsonError('conversion', 'expected JSON array'); }
        return array_values($this->value);
    }
    /** @return array<array-key, self> */
    public function asObject(): array
    {
        if ($this->kind !== JsonKind::Object || !is_array($this->value)) { throw new JsonError('conversion', 'expected JSON object'); }
        return $this->value;
    }
    public static function parse(string $json, ?JsonLimits $limits = null): self { return (new JsonParser($json, $limits ?? new JsonLimits()))->parse(); }
    public function toJson(?JsonLimits $limits = null): string { return (new JsonWriter($limits ?? new JsonLimits()))->write($this); }
}

/** Token parser. json_decode is used ONLY for isolated string tokens. @internal */
final class JsonParser
{
    private int $at = 0;
    private int $nodes = 0;
    private readonly int $length;
    public function __construct(private readonly string $text, private readonly JsonLimits $limits)
    {
        $this->length = strlen($text);
        if ($this->length > $limits->maxBytes) { throw new JsonError('resource_limit', 'JSON input exceeds byte ceiling'); }
    }
    public function parse(): JsonValue
    {
        $value = $this->value(0); $this->space();
        if ($this->at !== $this->length) { $this->bad('trailing data'); }
        return $value;
    }
    private function bad(string $message): never { throw new JsonError('syntax', $message . ' at byte ' . $this->at); }
    private function space(): void
    {
        while ($this->at < $this->length && str_contains(" \t\r\n", $this->text[$this->at])) {
            ++$this->at;
            if (($this->at & 1023) === 0) { $this->limits->control?->check(); }
        }
    }
    private function value(int $depth): JsonValue
    {
        $this->limits->control?->check();
        if ($depth >= $this->limits->maxDepth || ++$this->nodes > $this->limits->maxNodes) { throw new JsonError('resource_limit', 'JSON depth/node budget exhausted'); }
        $this->space();
        if ($this->at >= $this->length) { $this->bad('expected JSON value'); }
        $head = $this->text[$this->at];
        if ($head === '"') { return JsonValue::fromString($this->string()); }
        if ($head === '{') {
            ++$this->at; $this->space(); $members = [];
            if ($this->take('}')) { return JsonValue::fromObject([]); }
            while (true) {
                $this->space();
                if (($this->text[$this->at] ?? '') !== '"') { $this->bad('expected object member name'); }
                $key = $this->string(); $this->space();
                if (array_key_exists($key, $members)) { $this->bad('duplicate object member'); }
                if (!$this->take(':')) { $this->bad('expected colon'); }
                $members[$key] = $this->value($depth + 1); $this->space();
                if ($this->take('}')) { break; }
                if (!$this->take(',')) { $this->bad('expected comma or object end'); }
            }
            return JsonValue::fromObject($members);
        }
        if ($head === '[') {
            ++$this->at; $this->space(); $values = [];
            if ($this->take(']')) { return JsonValue::fromArray([]); }
            while (true) {
                $values[] = $this->value($depth + 1); $this->space();
                if ($this->take(']')) { break; }
                if (!$this->take(',')) { $this->bad('expected comma or array end'); }
            }
            return JsonValue::fromArray($values);
        }
        foreach (['null', 'true', 'false'] as $literal) {
            if (substr_compare($this->text, $literal, $this->at, strlen($literal)) === 0) {
                $this->at += strlen($literal);
                return $literal === 'null' ? JsonValue::null() : JsonValue::fromBool($literal === 'true');
            }
        }
        if ($head === '-' || ($head >= '0' && $head <= '9')) {
            $start = $this->at;
            while ($this->at < $this->length && str_contains('0123456789.eE+-', $this->text[$this->at])) {
                ++$this->at;
                if ($this->at - $start > $this->limits->maxNumberBytes) { throw new JsonError('resource_limit', 'JSON numeric token exceeds byte ceiling'); }
                if (($this->at & 1023) === 0) { $this->limits->control?->check(); }
            }
            if ($this->at - $start > $this->limits->maxNumberBytes) { throw new JsonError('resource_limit', 'JSON numeric token exceeds byte ceiling'); }
            return JsonValue::fromNumber(JsonNumber::fromString(substr($this->text, $start, $this->at - $start)));
        }
        $this->bad('unexpected token');
    }
    private function take(string $byte): bool
    {
        if (($this->text[$this->at] ?? '') !== $byte) { return false; }
        ++$this->at; return true;
    }
    private function string(): string
    {
        $start = $this->at++;
        while ($this->at < $this->length) {
            if (($this->at & 1023) === 0) { $this->limits->control?->check(); }
            $byte = $this->text[$this->at++];
            if ($byte === '"') {
                $token = substr($this->text, $start, $this->at - $start);
                try { $value = json_decode($token, true, 2, JSON_THROW_ON_ERROR); }
                catch (\JsonException $e) { throw new JsonError('syntax', $e->getMessage()); }
                if (!is_string($value)) { $this->bad('invalid string token'); }
                return $value;
            }
            if ($byte === '\\') { ++$this->at; }
            elseif (ord($byte) < 32) { $this->bad('unescaped control byte'); }
        }
        $this->bad('unterminated string');
    }
}

/** Deterministic bounded writer. Numeric tokens are emitted verbatim. @internal */
final class JsonWriter
{
    private string $output = '';
    private int $nodes = 0;
    public function __construct(private readonly JsonLimits $limits) {}
    public function write(JsonValue $value): string { $this->value($value, 0); return $this->output; }
    private function append(string $text): void
    {
        if (strlen($text) > $this->limits->maxBytes - strlen($this->output)) { throw new JsonError('resource_limit', 'JSON output exceeds byte ceiling'); }
        $this->output .= $text;
    }
    private function quote(string $value): void
    {
        if (strlen($value) + 2 > $this->limits->maxBytes - strlen($this->output)) { throw new JsonError('resource_limit', 'JSON string exceeds remaining output ceiling'); }
        // Scan once and append bounded runs. Escaping never materializes a 6x
        // temporary string before discovering that the output ceiling was exceeded.
        $this->append('"'); $start = 0;
        for ($at = 0, $length = strlen($value); $at < $length; ++$at) {
            if (($at & 1023) === 0) { $this->limits->control?->check(); }
            $byte = $value[$at];
            if ($byte !== '"' && $byte !== '\\' && ord($byte) >= 32) { continue; }
            $this->append(substr($value, $start, $at - $start));
            $this->append(match ($byte) {
                '"' => '\\"', '\\' => '\\\\', "\n" => '\\n', "\r" => '\\r', "\t" => '\\t',
                "\x08" => '\\b', "\x0c" => '\\f', default => sprintf('\\u%04x', ord($byte)),
            });
            $start = $at + 1;
        }
        $this->append(substr($value, $start)); $this->append('"');
    }
    private function value(JsonValue $value, int $depth): void
    {
        $this->limits->control?->check();
        if ($depth >= $this->limits->maxDepth || ++$this->nodes > $this->limits->maxNodes) { throw new JsonError('resource_limit', 'JSON depth/node budget exhausted'); }
        switch ($value->kind) {
            case JsonKind::Null: $this->append('null'); break;
            case JsonKind::Boolean: $this->append($value->asBool() ? 'true' : 'false'); break;
            case JsonKind::String: $this->quote($value->asString()); break;
            case JsonKind::Number:
                $token = $value->asNumber()->token;
                if (strlen($token) > $this->limits->maxNumberBytes) { throw new JsonError('resource_limit', 'JSON numeric token exceeds byte ceiling'); }
                $this->append($token); break;
            case JsonKind::Array:
                $items = $value->asArray();
                if (count($items) > $this->limits->maxNodes - $this->nodes) { throw new JsonError('resource_limit', 'JSON array exceeds remaining node ceiling'); }
                $this->append('['); $comma = false;
                foreach ($items as $item) {
                    if ($comma) { $this->append(','); } $comma = true;
                    $this->value($item, $depth + 1);
                }
                $this->append(']'); break;
            case JsonKind::Object:
                $members = $value->asObject();
                if (count($members) > $this->limits->maxNodes - $this->nodes) { throw new JsonError('resource_limit', 'JSON object exceeds remaining node ceiling'); }
                // Reject an oversized key set before sorting it. This lower
                // bound includes punctuation and a one-byte minimum per value.
                $minimum = 2 + 4 * count($members) + max(0, count($members) - 1);
                foreach ($members as $key => $_) {
                    $this->limits->control?->check(); $minimum += strlen((string) $key);
                    if ($minimum > $this->limits->maxBytes - strlen($this->output)) { throw new JsonError('resource_limit', 'JSON members exceed remaining output ceiling'); }
                }
                ksort($members, SORT_STRING);
                $this->append('{'); $comma = false;
                foreach ($members as $key => $item) {
                    if ($comma) { $this->append(','); } $comma = true;
                    $this->quote((string) $key); $this->append(':');
                    $this->value($item, $depth + 1);
                }
                $this->append('}'); break;
        }
    }
}

/** Native conversion depth/work guards and active-model cycle detection. @internal */
final class CodecContext
{
    private int $depth = 0;
    private int $remaining = RuntimeConfig::MAX_NODES;
    private int $bytes = RuntimeConfig::MAX_CONVERSION_BYTES;
    /** @var array<int, true> */
    private array $active = [];
    public readonly ValidationSession $validation;
    public function __construct(public readonly ?CallControl $control = null) { $this->validation = new ValidationSession($control); }
    public function enter(?object $value = null): void
    {
        $this->control?->check();
        if ($this->depth >= RuntimeConfig::MAX_DEPTH || --$this->remaining < 0) { throw new JsonError('resource_limit', 'native conversion depth/node budget exhausted'); }
        if ($value !== null) {
            $id = spl_object_id($value);
            if (isset($this->active[$id])) { throw new JsonError('conversion', 'cyclic native model'); }
            $this->active[$id] = true;
        }
        ++$this->depth;
    }
    public function leave(?object $value = null): void
    {
        --$this->depth;
        if ($value !== null) { unset($this->active[spl_object_id($value)]); }
    }
    public function bytes(int $amount): void
    {
        $this->control?->check();
        if ($amount < 0 || $amount > $this->bytes) { throw new JsonError('resource_limit', 'native conversion byte budget exhausted'); }
        $this->bytes -= $amount;
    }
    public function string(string $value): string { $this->bytes(strlen($value)); return $value; }
    public function number(JsonNumber $value): JsonNumber { $this->bytes(strlen($value->token)); return $value; }
    /** @param array<array-key, mixed> $values */
    public function array(array $values): JsonValue
    {
        if (!array_is_list($values)) { throw new JsonError('conversion', 'expected native JSON list'); }
        $this->enter();
        try {
            $items = [];
            foreach ($values as $value) {
                if (!$value instanceof JsonValue) { throw new JsonError('conversion', 'expected native JsonValue list member'); }
                $items[] = $this->json($value);
            }
            return JsonValue::fromArray($items);
        } finally { $this->leave(); }
    }
    /** Visit immutable arbitrary JSON under the SAME conversion budget as named fields. */
    public function json(JsonValue $value): JsonValue
    {
        $this->enter();
        try {
            switch ($value->kind) {
                case JsonKind::String: $this->bytes(strlen($value->asString())); break;
                case JsonKind::Number: $this->bytes(strlen($value->asNumber()->token)); break;
                case JsonKind::Array:
                    foreach ($value->asArray() as $item) { $this->json($item); } break;
                case JsonKind::Object:
                    foreach ($value->asObject() as $key => $item) { $this->bytes(strlen((string) $key)); $this->json($item); } break;
                default: break;
            }
            return $value;
        } finally { $this->leave(); }
    }
}
