<?php
declare(strict_types=1);

namespace __NAMESPACE__;

require_once __DIR__ . '/ValidationV2.php';
require_once __DIR__ . '/ValidationResources.php';

/** Completed invalidity and incomplete evaluation are distinct exception kinds. */
class ValidationError extends \RuntimeException
{
    public function __construct(
        public readonly string $kind,
        public readonly string $source,
        public readonly string $instancePath,
        string $message,
    ) { parent::__construct($kind . ' at ' . JsonError::display($source) . ' ' . JsonError::display($instancePath) . ': ' . JsonError::display($message)); }
}

/** Parse immutable program literals on first use, preserving unvisited-operand semantics. @internal */
final class ValidationLiteral
{
    private ?JsonValue $parsed = null;
    public function __construct(private readonly string $json) {}
    public function value(?CallControl $control): JsonValue
    {
        return $this->parsed ??= JsonValue::parse($this->json, new JsonLimits(control: $control));
    }
}

/** Immutable lowered instruction; only the generator constructs this metadata. @internal */
final readonly class ValidationInstruction
{
    /**
     * @param list<string> $types
     * @param list<int> $targets
     * @param list<array{string, int}> $properties
     * @param list<string> $names
     * @param list<string> $declared
     * @param list<ValidationLiteral> $literals
     * @param list<array{string,list<string>}> $requirements
     * @param list<array{string,int}> $dependencies
     * @param list<PatternProperty> $patterns
     */
    public function __construct(
        public string $op,
        public string $source,
        public bool $truth = false,
        public array $types = [],
        public int $target = -1,
        public array $targets = [],
        public array $properties = [],
        public array $names = [],
        public array $declared = [],
        public int $start = 0,
        public ?JsonNumber $number = null,
        public bool $maximum = false,
        public bool $exclusive = false,
        public string $countTarget = '',
        public ?ValidationLiteral $constant = null,
        public array $literals = [],
        public ?PatternProgram $pattern = null,
        public int $condition = -1,
        public ?int $thenTarget = null,
        public ?int $elseTarget = null,
        public array $requirements = [],
        public array $dependencies = [],
        public ?JsonNumber $containsMinimum = null,
        public ?JsonNumber $containsMaximum = null,
        public array $patterns = [],
        public int $initialResource = -1,
        public ?string $anchor = null,
    ) {}
}

/** @internal */
final readonly class ValidationNode
{
    /** @param list<ValidationInstruction> $checks */
    public function __construct(public string $source, public array $checks) {}
}

/** Portable ECMA-262 Unicode Thompson NFA, not a PHP regex translation. @internal */
final readonly class PatternProgram
{
    /** @param list<PatternState> $states */
    public function __construct(public int $start, public array $states) {}
}

/** @internal */
final readonly class PatternState
{
    /** @param list<array{int, int}> $ranges */
    public function __construct(
        public string $op,
        public int $target = -1,
        public int $first = -1,
        public int $second = -1,
        public array $ranges = [],
    ) {}
}

/** Validate a selected schema root against exact JSON; no defaults or coercions. */
final class Validator
{
    public static function validate(int $root, JsonValue $value): void { (new ValidationSession())->check($root, $value); }
}

/** Shared budgets survive all branch trials and cannot become ordinary invalidity. @internal */
final class ValidationSession
{
    use ScopedValidation;
    use ResourceValidation;
    private readonly bool $scoped;
    /** @var list<ValidationNode> */
    private readonly array $nodes;
    /** @var array<int, true> */
    private readonly array $roots;
    /** @var array<string, true> */
    private array $active = [];
    private int $steps = RuntimeConfig::MAX_EVALUATION_STEPS;
    private int $equalities = RuntimeConfig::MAX_EQUALITY_STEPS;
    private int $numeric = RuntimeConfig::MAX_EVALUATION_STEPS;

    public function __construct(private readonly ?CallControl $control = null)
    {
        $this->scoped=$this->profile(ValidationProgram::VERSION, ValidationProgram::PROFILE);
        $this->resources=$this->resourceMetadata(ValidationProgram::VERSION,ValidationProgram::class);
        $this->nodes = ValidationProgram::nodes();
        $this->roots = array_fill_keys(ValidationProgram::roots(), true);
    }
    private function profile(string $version, string $profile): bool
    {
        if($version==='suspect.validation.experimental.v1'&&$profile==='oas31-jsonschema202012-static-subset'){return false;}
        if($version==='suspect.validation.experimental.v2'&&$profile==='oas31-jsonschema202012-static-applicators'){return true;}
        if($version==='suspect.validation.experimental.v3'&&$profile==='oas31-jsonschema202012-resources-dynamic'){return true;}
        $this->failure('', '', 'unknown validation program version/profile');
    }

    public function check(int $root, JsonValue $value): void
    {
        if (!isset($this->roots[$root])) { $this->failure('', '', 'schema is not a selected validation root'); }
        $issue = $this->scoped?$this->evaluateScoped($root,$value,'',0)->issue:$this->evaluate($root, $value, '', 0);
        if ($issue !== null) { throw $issue; }
    }

    /** Internal native union selection shares this session's finite budgets. */
    public function matches(int $root, JsonValue $value): bool
    {
        if (!isset($this->roots[$root])) { $this->failure('', '', 'schema is not a selected validation root'); }
        return ($this->scoped?$this->evaluateScoped($root,$value,'',0)->issue:$this->evaluate($root, $value, '', 0)) === null;
    }

    private function failure(string $source, string $path, string $message): never
    {
        throw new ValidationError('evaluation_failure', $source, $path, $message);
    }
    private function spend(string $source, string $path, int $amount = 1): void
    {
        $this->control?->check();
        $this->steps -= $amount;
        if ($this->steps < 0) { $this->failure($source, $path, 'evaluation work budget exhausted'); }
    }
    private function child(string $path, string $key): string { return $path . '/' . str_replace(['~', '/'], ['~0', '~1'], $key); }

    private function evaluate(int $index, JsonValue $value, string $path, int $depth): ?ValidationError
    {
        if (!isset($this->nodes[$index])) { $this->failure('', $path, 'unknown instruction target'); }
        $node = $this->nodes[$index]; $this->spend($node->source, $path);
        $identity = $index . ':' . spl_object_id($value);
        if ($depth >= RuntimeConfig::MAX_SCHEMA_DEPTH || isset($this->active[$identity])) {
            $this->failure($node->source, $path, 'evaluation depth/nonproductive recursion limit');
        }
        $this->active[$identity] = true;
        try {
            $first = null;
            foreach ($node->checks as $check) {
                $this->spend($check->source, $path);
                // Do not short circuit: a later evaluation failure dominates
                // even an already-invalid conjunct or already-valid union.
                $issue = $this->instruction($check, $value, $path, $depth);
                $first ??= $issue;
            }
            return $first;
        } finally { unset($this->active[$identity]); }
    }

    private function mismatch(ValidationInstruction $check, string $path, string $message = ''): ValidationError
    {
        return new ValidationError('invalid', $check->source, $path, $message === '' ? $check->op . ' assertion rejected the value' : $message);
    }
    private function number(JsonNumber $number, ValidationInstruction $check, string $path): JsonNumber
    {
        if (strlen($number->token) > RuntimeConfig::MAX_NUMBER_BYTES) { $this->failure($check->source, $path, 'numeric operand byte budget exhausted'); }
        $this->numeric -= strlen($number->token);
        if ($this->numeric < 0) { $this->failure($check->source, $path, 'numeric work budget exhausted'); }
        return $number;
    }
    private function operand(ValidationInstruction $check, string $path): JsonNumber
    {
        if ($check->number === null) { $this->failure($check->source, $path, 'missing numeric operand'); }
        return $this->number($check->number, $check, $path);
    }

    private function literal(ValidationLiteral $literal, ValidationInstruction $check, string $path): JsonValue
    {
        try { return $literal->value($this->control); }
        catch (JsonError $e) { $this->failure($check->source, $path, 'literal representation exceeds the native JSON profile'); }
    }

    private function instruction(ValidationInstruction $check, JsonValue $value, string $path, int $depth): ?ValidationError
    {
        $kind = $value->kind; $result = true;
        switch ($check->op) {
            case 'always': $result = $check->truth; break;
            case 'type':
                $result = in_array($kind->value, $check->types, true);
                if (!$result && $kind === JsonKind::Number && in_array('integer', $check->types, true)) {
                    $result = $this->number($value->asNumber(), $check, $path)->isInteger();
                }
                break;
            case 'ref': return $this->evaluate($check->target, $value, $path, $depth + 1);
            case 'properties':
            case 'additionalProperties':
                if ($kind !== JsonKind::Object) { break; }
                $members = $value->asObject(); $first = null;
                if ($check->op === 'properties') {
                    foreach ($check->properties as [$name, $target]) {
                        $this->spend($check->source, $path);
                        if (array_key_exists($name, $members)) {
                            $issue = $this->evaluate($target, $members[$name], $this->child($path, $name), $depth + 1);
                            $first ??= $issue;
                        }
                    }
                } else {
                    $declared = array_fill_keys($check->declared, true);
                    foreach ($members as $name => $item) {
                        $this->spend($check->source, $path);
                        if (!isset($declared[$name])) {
                            $issue = $this->evaluate($check->target, $item, $this->child($path, (string) $name), $depth + 1);
                            $first ??= $issue;
                        }
                    }
                }
                return $first;
            case 'required':
                if ($kind === JsonKind::Object) {
                    $members = $value->asObject(); $first = null;
                    foreach ($check->names as $name) {
                        $this->spend($check->source, $path);
                        if (!array_key_exists($name, $members)) { $first ??= $this->mismatch($check, $path, 'required member is absent: ' . $name); }
                    }
                    return $first;
                }
                break;
            case 'items':
            case 'prefixItems':
                if ($kind === JsonKind::Array) {
                    $first = null;
                    foreach ($value->asArray() as $index => $item) {
                        if ($check->op === 'items' && $index < $check->start) { continue; }
                        if ($check->op === 'prefixItems' && !isset($check->targets[$index])) { break; }
                        $this->spend($check->source, $path);
                        $target = $check->op === 'items' ? $check->target : $check->targets[$index];
                        $issue = $this->evaluate($target, $item, $this->child($path, (string) $index), $depth + 1);
                        $first ??= $issue;
                    }
                    return $first;
                }
                break;
            case 'allOf':
            case 'anyOf':
            case 'oneOf':
                $matches = 0;
                foreach ($check->targets as $target) {
                    $this->spend($check->source, $path);
                    if ($this->evaluate($target, $value, $path, $depth + 1) === null) { ++$matches; }
                }
                $result = match ($check->op) { 'allOf' => $matches === count($check->targets), 'anyOf' => $matches > 0, default => $matches === 1 };
                break;
            case 'not': $result = $this->evaluate($check->target, $value, $path, $depth + 1) !== null; break;
            case 'bound':
                if ($kind === JsonKind::Number) {
                    $order = $this->number($value->asNumber(), $check, $path)->compare($this->operand($check, $path));
                    $result = ($check->maximum ? $order <= 0 : $order >= 0) && (!$check->exclusive || $order !== 0);
                }
                break;
            case 'multipleOf':
                if ($kind === JsonKind::Number) {
                    $result = $this->number($value->asNumber(), $check, $path)->multipleOf($this->operand($check, $path), function (int $cost) use ($check, $path): void {
                        $this->numeric -= $cost;
                        if ($this->numeric < 0) { $this->failure($check->source, $path, 'numeric work budget exhausted'); }
                    });
                }
                break;
            case 'count':
                if ($kind->value === $check->countTarget) {
                    $count = match ($kind) {
                        JsonKind::Array => count($value->asArray()), JsonKind::Object => count($value->asObject()),
                        JsonKind::String => self::scalarCount($value->asString()), default => 0,
                    };
                    $order = JsonNumber::fromInt($count)->compare($this->operand($check, $path));
                    $result = $check->maximum ? $order <= 0 : $order >= 0;
                }
                break;
            case 'const':
                if ($check->constant === null) { $this->failure($check->source, $path, 'missing constant operand'); }
                $result = $this->equal($value, $this->literal($check->constant, $check, $path), $check, $path, 0); break;
            case 'enum':
                $result = false;
                foreach ($check->literals as $literal) {
                    $this->spend($check->source, $path);
                    if ($this->equal($value, $this->literal($literal, $check, $path), $check, $path, 0)) { $result = true; break; }
                }
                break;
            case 'uniqueItems':
                if ($kind === JsonKind::Array) {
                    $items = $value->asArray();
                    for ($i = 0, $length = count($items); $i < $length; ++$i) {
                        $this->spend($check->source, $path);
                        for ($j = 0; $j < $i; ++$j) {
                            $this->spend($check->source, $path);
                            if ($this->equal($items[$i], $items[$j], $check, $path, 0)) { return $this->mismatch($check, $path); }
                        }
                    }
                }
                break;
            case 'pattern':
                if ($kind === JsonKind::String) {
                    if ($check->pattern === null) { $this->failure($check->source, $path, 'missing pattern program'); }
                    $result = $this->pattern($check->pattern, $value->asString(), $check, $path);
                }
                break;
            default: $this->failure($check->source, $path, 'unknown instruction: ' . $check->op);
        }
        return $result ? null : $this->mismatch($check, $path);
    }

    private function equal(JsonValue $a, JsonValue $b, ValidationInstruction $check, string $path, int $depth): bool
    {
        $this->control?->check();
        if (--$this->equalities < 0 || $depth > RuntimeConfig::MAX_SCHEMA_DEPTH) { $this->failure($check->source, $path, 'equality budget exhausted'); }
        if ($a->kind !== $b->kind) { return false; }
        switch ($a->kind) {
            case JsonKind::Null: return true;
            case JsonKind::Boolean: return $a->asBool() === $b->asBool();
            case JsonKind::String: return $a->asString() === $b->asString();
            case JsonKind::Number: return $this->number($a->asNumber(), $check, $path)->compare($this->number($b->asNumber(), $check, $path)) === 0;
            case JsonKind::Array:
                $left = $a->asArray(); $right = $b->asArray();
                if (count($left) !== count($right)) { return false; }
                foreach ($left as $i => $item) { if (!$this->equal($item, $right[$i], $check, $path, $depth + 1)) { return false; } }
                return true;
            case JsonKind::Object:
                $left = $a->asObject(); $right = $b->asObject();
                if (count($left) !== count($right)) { return false; }
                foreach ($left as $key => $item) {
                    if (!array_key_exists($key, $right) || !$this->equal($item, $right[$key], $check, $path, $depth + 1)) { return false; }
                }
                return true;
        }
    }

    /** JSON construction has already verified Unicode scalar encodings. */
    private static function scalarCount(string $text): int
    {
        $count = 0;
        for ($i = 0, $length = strlen($text); $i < $length; ++$i) { if ((ord($text[$i]) & 0xC0) !== 0x80) { ++$count; } }
        return $count;
    }

    private function pattern(PatternProgram $program, string $text, ValidationInstruction $check, string $path): bool
    {
        $this->spend($check->source, $path, strlen($text));
        $chars = preg_split('//u', $text, -1, PREG_SPLIT_NO_EMPTY);
        if ($chars === false) { $this->failure($check->source, $path, 'invalid Unicode instance'); }
        $length = count($chars);
        for ($offset = 0; $offset <= $length; ++$offset) {
            $current = [$program->start => true];
            for ($position = $offset; $position <= $length; ++$position) {
                $pending = array_keys($current); $seen = []; $consuming = [];
                while ($pending !== []) {
                    $this->spend($check->source, $path);
                    $index = array_pop($pending);
                    if (isset($seen[$index])) { continue; } $seen[$index] = true;
                    if (!isset($program->states[$index])) { $this->failure($check->source, $path, 'unknown pattern target'); }
                    $state = $program->states[$index];
                    switch ($state->op) {
                        case 'match': return true;
                        case 'split': $pending[] = $state->first; $pending[] = $state->second; break;
                        case 'jump': $pending[] = $state->target; break;
                        case 'start': if ($position === 0) { $pending[] = $state->target; } break;
                        case 'end': if ($position === $length) { $pending[] = $state->target; } break;
                        case 'char': $consuming[] = $state; break;
                        default: $this->failure($check->source, $path, 'unknown pattern instruction');
                    }
                }
                if ($position === $length) { break; }
                $point = self::codepoint($chars[$position]); $current = [];
                foreach ($consuming as $state) {
                    foreach ($state->ranges as [$low, $high]) {
                        $this->spend($check->source, $path);
                        if ($point >= $low && $point <= $high) { $current[$state->target] = true; break; }
                    }
                }
                if ($current === []) { break; }
            }
        }
        return false;
    }

    private static function codepoint(string $char): int
    {
        $head = ord($char[0]);
        return match (strlen($char)) {
            1 => $head,
            2 => (($head & 31) << 6) | (ord($char[1]) & 63),
            3 => (($head & 15) << 12) | ((ord($char[1]) & 63) << 6) | (ord($char[2]) & 63),
            default => (($head & 7) << 18) | ((ord($char[1]) & 63) << 12) | ((ord($char[2]) & 63) << 6) | (ord($char[3]) & 63),
        };
    }
}
