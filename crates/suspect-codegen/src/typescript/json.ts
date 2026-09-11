/** JSON representation/grammar failure, separate from schema validation. */
export class JsonCodecError extends Error {
    override readonly name = 'JsonCodecError';
    constructor(
        readonly kind: 'syntax' | 'type' | 'limit' | 'duplicate-key' | 'cycle',
        message: string,
        /** UTF-16 offset in parsed text, when available. */
        readonly offset?: number,
        /** RFC 6901 instance pointer during encoding, when available. */
        readonly path?: string,
    ) { super(message); }
}

/** Per-call representation budgets. These do not establish schema validity. */
export interface JsonLimits {
    /** Maximum input/output UTF-16 code units, including JSON punctuation. */
    readonly maxLength?: number;
    /** Maximum simultaneously open arrays/objects; traversal is iterative. */
    readonly maxDepth?: number;
    /** Maximum values visited, including the root and collection members. */
    readonly maxNodes?: number;
    /** Maximum numeric token characters, without expanding exponents. */
    readonly maxNumberLength?: number;
}
/** Encoding policy; undefined omission is restricted to object data properties. */
export interface JsonEncodeOptions extends JsonLimits {
    /** Omit enumerable string-named object properties whose value is undefined.
     * Inspected omitted slots still consume maxNodes. Arrays remain strict. */
    readonly omitUndefinedProperties?: boolean;
}
type Limits = Required<JsonLimits>;
const defaults: Limits = Object.freeze({
    maxLength: 16 * 1024 * 1024, maxDepth: 128, maxNodes: 1_000_000, maxNumberLength: 4096,
});
function limits(options: JsonLimits): Limits {
    const result = { ...defaults, ...options };
    for (const key of Object.keys(defaults) as (keyof Limits)[]) {
        if (!Number.isSafeInteger(result[key]) || result[key] < 0) {
            throw new JsonCodecError('limit', `${key} must be a nonnegative safe integer`);
        }
    }
    return result;
}
function digit(code: number): boolean { return code >= 48 && code <= 57; }
function numberEnd(text: string, start: number): number {
    let at = start;
    if (text[at] === '-') at++;
    if (text[at] === '0') at++;
    else {
        if (!(text.charCodeAt(at) >= 49 && text.charCodeAt(at) <= 57)) {
            throw new JsonCodecError('syntax', 'expected a JSON number', at);
        }
        while (digit(text.charCodeAt(at))) at++;
    }
    if (text[at] === '.') {
        at++;
        if (!digit(text.charCodeAt(at))) throw new JsonCodecError('syntax', 'expected fraction digits', at);
        while (digit(text.charCodeAt(at))) at++;
    }
    if (text[at] === 'e' || text[at] === 'E') {
        at++;
        if (text[at] === '+' || text[at] === '-') at++;
        if (!digit(text.charCodeAt(at))) throw new JsonCodecError('syntax', 'expected exponent digits', at);
        while (digit(text.charCodeAt(at))) at++;
    }
    return at;
}

let hasJsonNumber!: (value: unknown) => value is JsonNumber;
let rawJsonNumber!: (value: JsonNumber) => string;

/** Exact JSON numeric token. Parsing never rounds or expands an exponent. */
export class JsonNumber {
    readonly #token: string;
    static {
        hasJsonNumber = (value): value is JsonNumber => typeof value === 'object' && value !== null && #token in value;
        rawJsonNumber = (value) => value.#token;
    }
    private constructor(token: string, maxLength: number) {
        if (typeof token !== 'string') throw new JsonCodecError('type', 'numeric token must be a string');
        if (!Number.isSafeInteger(maxLength) || maxLength < 0 || token.length > maxLength) {
            throw new JsonCodecError('limit', 'numeric token exceeds its character budget');
        }
        if (numberEnd(token, 0) !== token.length) throw new JsonCodecError('syntax', 'invalid JSON numeric token');
        this.#token = token;
        Object.freeze(this);
    }
    /** Checks grammar only. Constraints such as minimum belong to schema codecs. */
    static parse(token: string, maxLength = defaults.maxNumberLength): JsonNumber {
        return new JsonNumber(token, maxLength);
    }
    /** Tests the private runtime brand, without accepting a copied prototype. */
    static is(value: unknown): value is JsonNumber {
        return hasJsonNumber(value);
    }
    /** Preserves original numeric spelling, including negative zero. */
    toString(): string { return this.#token; }
    /** Returns the mathematical integer, or undefined if fractional/over budget. */
    toBigInt(maxDigits = defaults.maxNumberLength): bigint | undefined {
        return this.#integer(maxDigits);
    }
    #integer(maxDigits: number): bigint | undefined {
        if (!Number.isSafeInteger(maxDigits) || maxDigits < 0) {
            throw new JsonCodecError('limit', 'maxDigits must be a nonnegative safe integer');
        }
        const [mantissa = '', exponent = '0'] = this.#token.toLowerCase().split('e');
        const negative = mantissa.startsWith('-');
        const unsigned = negative ? mantissa.slice(1) : mantissa;
        const dot = unsigned.indexOf('.');
        const fractionLength = dot < 0 ? 0 : unsigned.length - dot - 1;
        const digits = unsigned.replace('.', '').replace(/^0+/, '');
        if (digits === '') return maxDigits === 0 ? undefined : 0n;
        let end = digits.length;
        while (end > 0 && digits.charCodeAt(end - 1) === 48) end--;
        const coefficient = digits.slice(0, end);
        const power = BigInt(exponent) + BigInt(digits.length - coefficient.length - fractionLength);
        if (power < 0n || power > BigInt(maxDigits - coefficient.length)) return undefined;
        return BigInt((negative ? '-' : '') + coefficient + '0'.repeat(Number(power)));
    }
    /** Checked conversion; never rounds or overflows a JavaScript safe integer. */
    toSafeInteger(): number | undefined {
        const integer = this.#integer(16);
        return integer === undefined || integer < -9007199254740991n || integer > 9007199254740991n
            ? undefined : Number(integer);
    }
    /** Prevents accidental lossy use through ordinary JSON.stringify. */
    toJSON(): never { throw new JsonCodecError('type', 'use stringifyJson for exact JSON numbers'); }
}

/** Values accepted by representation encoding; schema codecs refine this type. */
export type JsonValue = null | boolean | string | number | bigint | JsonNumber | JsonValue[] | { [key: string]: JsonValue };
/** Parsed numbers always retain exact tokens until a schema codec converts them. */
export type WireJsonValue = null | boolean | string | JsonNumber | WireJsonValue[] | { [key: string]: WireJsonValue };
type ParseFrame =
    | { kind: 'array'; value: WireJsonValue[]; entry: boolean; allowEnd: boolean }
    | { kind: 'object'; value: { [key: string]: WireJsonValue }; entry: boolean; allowEnd: boolean };

/** Parses one complete JSON value, preserving numbers and rejecting duplicate keys. */
export function parseJson(text: string, options: JsonLimits = {}): WireJsonValue {
    const cap = limits(options);
    if (typeof text !== 'string') throw new JsonCodecError('type', 'JSON input must be a string');
    if (text.length > cap.maxLength) throw new JsonCodecError('limit', 'JSON input exceeds its character budget');
    let at = 0;
    let nodes = 0;
    const stack: ParseFrame[] = [];
    const fail = (message: string): never => { throw new JsonCodecError('syntax', message, at); };
    function whitespace(): void {
        while (text[at] === ' ' || text[at] === '\t' || text[at] === '\n' || text[at] === '\r') at++;
    }
    function string(): string {
        if (text[at] !== '"') fail('expected a quoted property name');
        const start = at++;
        while (at < text.length) {
            const code = text.charCodeAt(at++);
            if (code === 34) {
                try { return JSON.parse(text.slice(start, at)) as string; }
                catch { return fail('invalid JSON string escape'); }
            }
            if (code < 32) fail('unescaped control character in JSON string');
            if (code === 92) at++;
        }
        return fail('unterminated JSON string');
    }
    function value(): WireJsonValue {
        whitespace();
        if (++nodes > cap.maxNodes) throw new JsonCodecError('limit', 'JSON value budget exceeded', at);
        const char = text[at];
        if (char === '"') return string();
        if (char === '{' || char === '[') {
            if (stack.length >= cap.maxDepth) throw new JsonCodecError('limit', 'JSON nesting budget exceeded', at);
            at++;
            const frame: ParseFrame = char === '['
                ? { kind: 'array', value: [], entry: true, allowEnd: true }
                : { kind: 'object', value: {}, entry: true, allowEnd: true };
            stack.push(frame);
            return frame.value;
        }
        for (const [token, result] of [['null', null], ['true', true], ['false', false]] as const) {
            if (text.startsWith(token, at)) { at += token.length; return result; }
        }
        const start = at;
        at = numberEnd(text, at);
        return JsonNumber.parse(text.slice(start, at), cap.maxNumberLength);
    }
    const root = value();
    while (stack.length !== 0) {
        const frame = stack[stack.length - 1]!;
        whitespace();
        const end = frame.kind === 'array' ? ']' : '}';
        if (!frame.entry) {
            if (text[at] === end) { at++; stack.pop(); continue; }
            if (text[at++] !== ',') fail('expected comma or end of collection');
            frame.entry = true;
            frame.allowEnd = false;
            whitespace();
        }
        if (text[at] === end && frame.allowEnd) { at++; stack.pop(); continue; }
        frame.entry = false;
        if (frame.kind === 'array') frame.value.push(value());
        else {
            const keyAt = at;
            const key = string();
            if (Object.hasOwn(frame.value, key)) throw new JsonCodecError('duplicate-key', `duplicate JSON property ${JSON.stringify(key)}`, keyAt);
            whitespace();
            if (text[at++] !== ':') fail('expected colon after property name');
            Object.defineProperty(frame.value, key, { value: value(), enumerable: true, writable: true, configurable: true });
        }
    }
    whitespace();
    if (at !== text.length) fail('unexpected trailing JSON input');
    return root;
}

type EncodeFrame = {
    object: object; keys: string[]; descriptors: PropertyDescriptorMap;
    array: boolean; index: number; path: string;
};
function childPath(path: string, key: string): string {
    return `${path}/${key.replace(/~/g, '~0').replace(/\//g, '~1')}`;
}

/** Encodes JSON values without invoking getters/toJSON or silently erasing data. */
export function stringifyJson(value: unknown, options: JsonEncodeOptions = {}): string {
    const cap = limits(options);
    const chunks: string[] = [];
    const stack: EncodeFrame[] = [];
    const active = new Set<object>();
    let length = 0;
    let nodes = 0;
    function append(text: string): void {
        length += text.length;
        if (length > cap.maxLength) throw new JsonCodecError('limit', 'JSON output exceeds its character budget');
        chunks.push(text);
    }
    function string(text: string): void {
        if (text.length > cap.maxLength - length) throw new JsonCodecError('limit', 'JSON string exceeds output budget');
        append(JSON.stringify(text));
    }
    function emit(current: unknown, path: string): void {
        if (++nodes > cap.maxNodes) throw new JsonCodecError('limit', 'JSON value budget exceeded', undefined, path);
        if (current === null) { append('null'); return; }
        if (typeof current === 'string') { string(current); return; }
        if (typeof current === 'boolean') { append(current ? 'true' : 'false'); return; }
        if (typeof current === 'number' || typeof current === 'bigint' || hasJsonNumber(current)) {
            if (typeof current === 'number' && (!Number.isFinite(current) || Number.isInteger(current) && !Number.isSafeInteger(current))) {
                throw new JsonCodecError('type', 'native numbers must be finite, with safe integers; use bigint or JsonNumber', undefined, path);
            }
            const token = hasJsonNumber(current) ? rawJsonNumber(current)
                : typeof current === 'number' && Object.is(current, -0) ? '-0' : current.toString();
            if (token.length > cap.maxNumberLength) throw new JsonCodecError('limit', 'numeric token exceeds its character budget', undefined, path);
            append(token);
            return;
        }
        if (typeof current !== 'object') throw new JsonCodecError('type', 'value is not JSON-representable', undefined, path);
        if (active.has(current)) throw new JsonCodecError('cycle', 'cyclic JSON value', undefined, path);
        if (stack.length >= cap.maxDepth) throw new JsonCodecError('limit', 'JSON nesting budget exceeded', undefined, path);
        if (cap.maxLength - length < 2) throw new JsonCodecError('limit', 'JSON collection exceeds output budget', undefined, path);
        const array = Array.isArray(current);
        const prototype = Object.getPrototypeOf(current);
        if (!array && prototype !== null && prototype !== Object.prototype) {
            throw new JsonCodecError('type', 'JSON objects must have a plain or null prototype', undefined, path);
        }
        // Enumerating keys is unavoidable for arbitrary JS objects. Admit the
        // immediate collection before materializing its property descriptors.
        const ownKeys = Reflect.ownKeys(current);
        const count = ownKeys.length - (array ? 1 : 0);
        if (count > cap.maxNodes - nodes) throw new JsonCodecError('limit', 'JSON collection exceeds value budget', undefined, path);
        const omitUndefined = !array && options.omitUndefinedProperties === true;
        let minimumLength = omitUndefined ? 2 : 2 + Math.max(0, 2 * count - 1);
        for (const key of ownKeys) {
            if (array && key === 'length') continue;
            if (typeof key !== 'string') throw new JsonCodecError('type', 'JSON properties must have string names', undefined, path);
            if (!array && !omitUndefined) minimumLength += key.length + 3;
        }
        if (minimumLength > cap.maxLength - length) throw new JsonCodecError('limit', 'JSON collection exceeds output budget', undefined, path);
        const descriptors: PropertyDescriptorMap = Object.create(null);
        const keys: string[] = [];
        const arrayLength = array ? Object.getOwnPropertyDescriptor(current, 'length')?.value : undefined;
        for (const key of ownKeys) {
            if (array && key === 'length') continue;
            const descriptor = Object.getOwnPropertyDescriptor(current, key);
            if (!descriptor || !descriptor.enumerable || typeof key !== 'string' || !Object.hasOwn(descriptor, 'value')) {
                throw new JsonCodecError('type', 'JSON properties must be enumerable string-named data properties', undefined, path);
            }
            if (omitUndefined && descriptor.value === undefined) {
                nodes++;
                continue;
            }
            descriptors[key] = descriptor;
            keys.push(key);
        }
        if (array && (keys.length !== arrayLength || keys.some((key, index) => key !== String(index)))) {
            throw new JsonCodecError('type', 'JSON arrays cannot contain holes or extra enumerable properties', undefined, path);
        }
        active.add(current);
        stack.push({ object: current, keys, descriptors, array, index: 0, path });
        append(array ? '[' : '{');
    }
    emit(value, '');
    while (stack.length !== 0) {
        const frame = stack[stack.length - 1]!;
        if (frame.index === frame.keys.length) {
            append(frame.array ? ']' : '}');
            active.delete(frame.object);
            stack.pop();
            continue;
        }
        if (frame.index !== 0) append(',');
        const key = frame.keys[frame.index++]!;
        if (!frame.array) { string(key); append(':'); }
        emit(frame.descriptors[key]!.value, childPath(frame.path, key));
    }
    return chunks.join('');
}
