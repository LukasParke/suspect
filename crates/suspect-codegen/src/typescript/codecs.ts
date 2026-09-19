import { JsonCodecError, JsonNumber, parseJson, stringifyJson, type JsonLimits, type WireJsonValue } from './json.js';
import { equalJsonNumberToken, isIntegralJsonNumber, type ValidationFinding, type ValidationOutcome, type ValidationSource, type ValidationTrace } from './validation.js';

/** An admitted language representation plan, separate from schema assertions. */
export type Conversion =
    | { readonly kind: 'any' | 'never' | 'null' | 'boolean' | 'string' | 'safeInteger' | 'integer' | 'number' | 'anyNumber' }
    | { readonly kind: 'literal'; readonly value: boolean | string }
    | { readonly kind: 'integerLiteral'; readonly value: string; readonly safe: boolean }
    | { readonly kind: 'reference'; readonly target: number }
    | { readonly kind: 'source'; readonly source: ValidationSource; readonly expression: Conversion }
    | { readonly kind: 'array'; readonly item: Conversion }
    | { readonly kind: 'object'; readonly fields: readonly { readonly name: string; readonly required: boolean; readonly expression: Conversion }[]; readonly extra: Conversion | null }
    | { readonly kind: 'union'; readonly alternatives: readonly Conversion[] }
    | { readonly kind: 'intersection'; readonly members: readonly Conversion[] };

/** Per-call conversion limits and finite symbolic model definitions. */
export interface ConversionProgram {
    /** Compiler-indexed expressions; generated per-codec slices may contain unreachable holes. */
    readonly symbols: readonly (Conversion | undefined)[];
    readonly maxDepth: number;
    readonly maxSteps: number;
    readonly maxIntegerDigits: number;
    /** V3 source-to-resource indices from the same checked validation program. */
    readonly resourceScopes?: Readonly<Record<string, number>>;
}

/** Typed model decoding and source-validated encoding. */
export interface ModelCodec<T> {
    /**
     * Parses exact JSON, validates the source contract once, then constructs the
     * admitted model representation. Missing members remain missing and null
     * remains null. Successful union branches use declared conversion order.
     * @param text Complete JSON response or stored model text.
     * @param options JSON parser limits; compiled validation and conversion limits also apply.
     * @returns The typed model, with safe integers, bigint integers and exact JsonNumber decimals as planned.
     * @throws ModelCodecError for invalid JSON, schema mismatch, incomplete validation, unsupported representation or exhausted conversion limits.
     */
    decode(text: string, options?: JsonLimits): T;
    /**
     * Serializes model values without invoking getters or toJSON, then validates
     * the resulting wire value against the source contract once. Own undefined
     * object members are omitted; source validation rejects required omissions.
     * Undefined array items and array holes are errors.
     * @param value Typed model value to encode, including bigint and JsonNumber representations.
     * @param options JSON encoder/parser limits; compiled validation limits also apply.
     * @returns JSON text that satisfies the compiled source contract.
     * @throws ModelCodecError for unsafe JSON representation, schema mismatch, incomplete evaluation or exhausted representation limits.
     */
    encode(value: T, options?: JsonLimits): string;
}

/** Stable failure categories; resource failures never become union mismatches. */
export class ModelCodecError extends Error {
    override readonly name = 'ModelCodecError';
    constructor(
        readonly kind: 'json' | 'invalid' | 'evaluation' | 'representation' | 'limit',
        message: string,
        readonly source: ValidationSource | undefined = undefined,
        readonly instancePath: string | undefined = undefined,
        readonly findings: readonly ValidationFinding[] = [],
        cause?: unknown,
    ) { super(message, { cause }); }
}

type Validator = (root: ValidationSource, value: WireJsonValue, trace?: ValidationTrace) => ValidationOutcome;
type ModelValue = null | boolean | string | number | bigint | JsonNumber | ModelValue[] | { [key: string]: ModelValue };
type Tree =
    | { readonly kind: 'pass' | 'atom'; readonly value: ModelValue }
    | { readonly kind: 'array'; readonly value: ModelValue[]; readonly items: readonly Tree[] }
    | { readonly kind: 'object'; readonly value: { [key: string]: ModelValue }; readonly fields: ReadonlyMap<string, Tree> };

const numberBrand = JsonNumber.is;
const integerValue = JsonNumber.prototype.toBigInt;
const safeIntegerValue = JsonNumber.prototype.toSafeInteger;
const own = Object.prototype.hasOwnProperty;
const sourceKey = (source: ValidationSource): string => JSON.stringify([source.document, source.pointer]);
const traceKey = (source: ValidationSource, path: string, context?: string): string => JSON.stringify(context === undefined ? [source.document, source.pointer, path] : [source.document, source.pointer, path, context]);
const childPath = (path: string, key: string): string => `${path}/${key.replace(/~/g, '~0').replace(/\//g, '~1')}`;
const sourcesByProgram = new WeakMap<ConversionProgram, ReadonlySet<string>>();

class Mismatch extends Error {
    constructor(readonly source: ValidationSource, readonly path: string, message: string) { super(message); }
}
function mismatch(source: ValidationSource, path: string, message: string): never { throw new Mismatch(source, path, message); }
function definedSources(program: ConversionProgram): ReadonlySet<string> {
    const cached = sourcesByProgram.get(program);
    if (cached !== undefined) return cached;
    const sources = new Set<string>();
    const pending = [...program.symbols];
    while (pending.length !== 0) {
        const expression = pending.pop()!;
        if (expression === undefined) continue;
        switch (expression.kind) {
            case 'source': sources.add(sourceKey(expression.source)); pending.push(expression.expression); break;
            case 'array': pending.push(expression.item); break;
            case 'object': for (const field of expression.fields) pending.push(field.expression); if (expression.extra !== null) pending.push(expression.extra); break;
            case 'union': for (const alternative of expression.alternatives) pending.push(alternative); break;
            case 'intersection': for (const member of expression.members) pending.push(member); break;
        }
    }
    sourcesByProgram.set(program, sources);
    return sources;
}
function assertValid(outcome: ValidationOutcome): void {
    if (outcome.kind === 'invalid') {
        const first = outcome.findings[0];
        throw new ModelCodecError('invalid', first?.message ?? 'value violates its source schema', first?.source, first?.instancePath, outcome.findings);
    }
    if (outcome.kind === 'evaluationFailure') {
        throw new ModelCodecError('evaluation', outcome.finding.message, outcome.finding.source, outcome.finding.instancePath, [outcome.finding]);
    }
}
function codecFailure(error: unknown, source: ValidationSource): never {
    if (error instanceof ModelCodecError) throw error;
    if (error instanceof Mismatch) throw new ModelCodecError('representation', error.message, error.source, error.path, [], error);
    if (error instanceof JsonCodecError) throw new ModelCodecError(error.kind === 'limit' ? 'limit' : 'json', error.message, source, error.path, [], error);
    throw new ModelCodecError('evaluation', error instanceof Error ? error.message : 'model conversion did not complete', source, undefined, [], error);
}

/**
 * Creates a codec from admitted generated modules. Decode validates once, then
 * uses only completed schema traces from a valid root. When several union
 * branches can represent a value, declared plan order determines the result.
 * Type/enum alternatives use representation tests without invented traces.
 * Tracing and conversion do not restart validation budgets. No input data is
 * changed; missing members, nulls and arbitrary wire names stay distinct.
 */
export function createCodec<T>(source: ValidationSource, symbolIndex: number, program: ConversionProgram, validate: Validator): ModelCodec<T> {
    for (const [name, limit] of [['maxDepth', program.maxDepth], ['maxSteps', program.maxSteps], ['maxIntegerDigits', program.maxIntegerDigits]] as const) {
        if (!Number.isSafeInteger(limit) || limit < 0) throw new ModelCodecError('representation', `${name} must be a nonnegative safe integer`, source);
    }
    if (!Number.isSafeInteger(symbolIndex) || symbolIndex < 0 || symbolIndex >= program.symbols.length || program.symbols[symbolIndex] === undefined) throw new ModelCodecError('representation', 'model symbol is outside the admitted conversion program', source);
    const wanted = definedSources(program);
    return Object.freeze({
        decode(text: string, options: JsonLimits = {}): T {
            try {
                const wire = parseJson(text, options);
                const trace = new Map<string, boolean>();
                assertValid(validate(source, wire, (at, path, valid, context) => { if (wanted.has(sourceKey(at))) trace.set(traceKey(at, path, context), valid); }));
                const state = new ConversionState(program, trace);
                return state.convert(program.symbols[symbolIndex]!, wire, source, '').value as T;
            } catch (error) { return codecFailure(error, source); }
        },
        encode(value: T, options: JsonLimits = {}): string {
            try {
                // Undefined object members are explicit omission candidates;
                // the source validator still rejects required omissions.
                const text = stringifyJson(value, { ...options, omitUndefinedProperties: true });
                const wire = parseJson(text, options);
                assertValid(validate(source, wire));
                return text;
            } catch (error) { return codecFailure(error, source); }
        },
    });
}

/** @internal Defers trusted generated conversion and validator tables until a codec is first called. */
export function createLazyCodec<T>(factory: () => ModelCodec<T>): ModelCodec<T> {
    let codec: ModelCodec<T> | undefined;
    const get = (): ModelCodec<T> => codec ??= factory();
    return Object.freeze({
        decode(text: string, options?: JsonLimits): T { return get().decode(text, options); },
        encode(value: T, options?: JsonLimits): string { return get().encode(value, options); },
    });
}

class ConversionState {
    private remaining: number;
    private depth = 0;
    private readonly resources: number[] = [];
    private readonly entered = new Set<number>();
    constructor(private readonly program: ConversionProgram, private readonly trace: ReadonlyMap<string, boolean>) { this.remaining = program.maxSteps; }
    private step(source: ValidationSource, path: string): void {
        if (this.remaining === 0) throw new ModelCodecError('limit', `model conversion exceeds ${this.program.maxSteps} visits`, source, path);
        this.remaining--;
    }
    private arithmetic<T>(source: ValidationSource, path: string, run: () => T): T {
        try { return run(); }
        catch (error) { throw new ModelCodecError('limit', error instanceof Error ? error.message : 'exact numeric conversion did not complete', source, path, [], error); }
    }
    private integer(value: JsonNumber, safe: boolean, source: ValidationSource, path: string): ModelValue {
        if (!this.arithmetic(source, path, () => isIntegralJsonNumber(value))) return mismatch(source, path, 'value is not a mathematical integer');
        if (safe) {
            const result = this.arithmetic(source, path, () => safeIntegerValue.call(value));
            if (result === undefined) return mismatch(source, path, 'integer is outside the safe native number range');
            if (String(Math.abs(result)).length > this.program.maxIntegerDigits) throw new ModelCodecError('limit', `integer representation exceeds ${this.program.maxIntegerDigits} decimal digits`, source, path);
            return result;
        }
        const result = this.arithmetic(source, path, () => integerValue.call(value, this.program.maxIntegerDigits));
        if (result === undefined) throw new ModelCodecError('limit', `integer representation exceeds ${this.program.maxIntegerDigits} decimal digits`, source, path);
        return result;
    }
    convert(expression: Conversion, value: WireJsonValue, source: ValidationSource, path: string): Tree {
        this.step(source, path);
        if (this.depth >= this.program.maxDepth) throw new ModelCodecError('limit', `model conversion depth exceeds ${this.program.maxDepth}`, source, path);
        this.depth++;
        try { return this.run(expression, value, source, path); }
        finally { this.depth--; }
    }
    private run(expression: Conversion, value: WireJsonValue, source: ValidationSource, path: string): Tree {
        switch (expression.kind) {
            case 'any': return { kind: 'pass', value };
            case 'never': return mismatch(source, path, 'value has no representation in an uninhabited model');
            case 'null': if (value === null) return { kind: 'atom', value }; break;
            case 'boolean': if (typeof value === 'boolean') return { kind: 'atom', value }; break;
            case 'string': if (typeof value === 'string') return { kind: 'atom', value }; break;
            case 'safeInteger': case 'integer':
                if (numberBrand(value)) return { kind: 'atom', value: this.integer(value, expression.kind === 'safeInteger', source, path) };
                break;
            case 'number': if (numberBrand(value)) return { kind: 'atom', value }; break;
            case 'anyNumber': if (numberBrand(value)) return { kind: 'pass', value }; break;
            case 'literal': if (value === expression.value) return { kind: 'atom', value }; break;
            case 'integerLiteral':
                if (numberBrand(value) && this.arithmetic(source, path, () => equalJsonNumberToken(value, expression.value))) {
                    return { kind: 'atom', value: this.integer(value, expression.safe, source, path) };
                }
                break;
            case 'reference': {
                const target = this.program.symbols[expression.target];
                if (target === undefined) throw new ModelCodecError('representation', 'reference is outside the admitted conversion program', source, path);
                return this.convert(target, value, source, path);
            }
            case 'source': {
                const scopes = this.program.resourceScopes, resource = scopes?.[sourceKey(expression.source)];
                if (scopes !== undefined && (resource === undefined || !Number.isSafeInteger(resource) || resource < 0)) throw new ModelCodecError('representation', 'source has no checked resource scope', expression.source, path);
                const fresh = resource !== undefined && !this.entered.has(resource);
                if (fresh) { this.resources.push(resource); this.entered.add(resource); }
                try {
                    const context = scopes === undefined ? undefined : this.resources.join(',');
                    if (this.trace.get(traceKey(expression.source, path, context)) !== true) return mismatch(expression.source, path, scopes === undefined ? 'schema alternative did not validate' : 'schema alternative did not validate in this resource context');
                    return this.convert(expression.expression, value, expression.source, path);
                } finally { if (fresh) { this.resources.pop(); this.entered.delete(resource); } }
            }
            case 'array': {
                if (!Array.isArray(value)) break;
                const items: Tree[] = [];
                for (let index = 0; index < value.length; index++) {
                    this.step(source, path);
                    items.push(this.convert(expression.item, value[index]!, source, childPath(path, String(index))));
                }
                return this.array(items, source, path);
            }
            case 'object': {
                if (value === null || typeof value !== 'object' || Array.isArray(value) || numberBrand(value)) break;
                const fields = new Map<string, Tree>();
                const known = new Map<string, (typeof expression.fields)[number]>();
                for (const field of expression.fields) {
                    this.step(source, path);
                    known.set(field.name, field);
                    if (field.required && !own.call(value, field.name)) return mismatch(source, path, `required model property ${JSON.stringify(field.name)} is absent`);
                }
                for (const name of Object.keys(value)) {
                    this.step(source, path);
                    const field = known.get(name);
                    const child = field?.expression ?? expression.extra;
                    if (child === null) return mismatch(source, childPath(path, name), 'undeclared property has no model representation');
                    fields.set(name, this.convert(child, value[name]!, source, childPath(path, name)));
                }
                return this.object(fields, source, path);
            }
            case 'union': {
                let last: Mismatch | undefined;
                for (const alternative of expression.alternatives) {
                    this.step(source, path);
                    try { return this.convert(alternative, value, source, path); }
                    catch (error) { if (!(error instanceof Mismatch)) throw error; last = error; }
                }
                throw last ?? new Mismatch(source, path, 'no model alternative can represent the validated value');
            }
            case 'intersection': {
                let merged: Tree = { kind: 'pass', value };
                for (const member of expression.members) {
                    this.step(source, path);
                    merged = this.merge(merged, this.convert(member, value, source, path), source, path);
                }
                return merged;
            }
            default: throw new ModelCodecError('representation', 'unknown admitted model conversion instruction', source, path);
        }
        return mismatch(source, path, `value does not fit the ${expression.kind} representation`);
    }
    private array(items: readonly Tree[], source: ValidationSource, path: string): Tree {
        const value: ModelValue[] = [];
        for (const item of items) { this.step(source, path); value.push(item.value); }
        return { kind: 'array', value, items };
    }
    private object(fields: ReadonlyMap<string, Tree>, source: ValidationSource, path: string): Tree {
        const value: { [key: string]: ModelValue } = Object.create(null);
        for (const [name, field] of fields) {
            this.step(source, path);
            Object.defineProperty(value, name, { value: field.value, enumerable: true, writable: true, configurable: true });
        }
        return { kind: 'object', value, fields };
    }
    private merge(left: Tree, right: Tree, source: ValidationSource, path: string): Tree {
        this.step(source, path);
        if (left.kind === 'pass') return right;
        if (right.kind === 'pass') return left;
        if (left.kind === 'array' && right.kind === 'array') {
            if (left.items.length !== right.items.length) return mismatch(source, path, 'intersection array representations have different lengths');
            const items: Tree[] = [];
            for (let index = 0; index < left.items.length; index++) items.push(this.merge(left.items[index]!, right.items[index]!, source, childPath(path, String(index))));
            return this.array(items, source, path);
        }
        if (left.kind === 'object' && right.kind === 'object') {
            const fields = new Map<string, Tree>();
            for (const [name, field] of left.fields) { this.step(source, path); fields.set(name, field); }
            for (const [name, field] of right.fields) {
                this.step(source, path);
                const previous = fields.get(name);
                fields.set(name, previous === undefined ? field : this.merge(previous, field, source, childPath(path, name)));
            }
            return this.object(fields, source, path);
        }
        if (left.kind === 'atom' && right.kind === 'atom' && Object.is(left.value, right.value)) return left;
        return mismatch(source, path, 'intersection members require incompatible model representations');
    }
}
