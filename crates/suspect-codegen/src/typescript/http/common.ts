import { ModelCodecError, type ModelCodec } from '../codecs.js';
import { JsonCodecError, JsonNumber, parseJson, stringifyJson, type WireJsonValue } from '../json.js';
import type { ValidationSource } from '../validation.js';
import type { ApiResponse, CodecRef, DeclaredApiError, OperationDescriptor, SdkError, SdkFailureKind } from './types.js';

export const sourceKey = (source: ValidationSource): string => JSON.stringify([source.document, source.pointer]);
const sdkErrors = new WeakMap<object, string>();
const apiErrors = new WeakMap<object, string>();

export class RuntimeError<K extends SdkFailureKind = SdkFailureKind> extends Error implements SdkError {
    override readonly name: string = 'SdkError';
    constructor(readonly kind: K, message: string, readonly operationSource: ValidationSource,
        readonly source: ValidationSource | undefined, cause?: unknown) {
        super(message, cause === undefined ? undefined : { cause });
        sdkErrors.set(this, sourceKey(operationSource));
    }
}
export class ApiError extends Error {
    override readonly name = 'ApiError'; readonly kind = 'api-error' as const;
    constructor(readonly operationSource: ValidationSource, readonly responseSource: ValidationSource,
        readonly response: ApiResponse<unknown, number, string | null, object, object>) {
        super(`operation returned declared API error ${response.status}${response.contentType === null ? '' : ` ${response.contentType}`}`);
        apiErrors.set(this, sourceKey(operationSource));
    }
}

/** Accepts only failures created by this runtime; shape lookalikes do not match. */
export function isSdkError(error: unknown, operationSource?: ValidationSource): error is SdkError {
    return typeof error === 'object' && error !== null && sdkErrors.has(error) &&
        (operationSource === undefined || sdkErrors.get(error) === sourceKey(operationSource));
}
export function isDeclaredApiError(error: unknown, source: ValidationSource): error is DeclaredApiError {
    return typeof error === 'object' && error !== null && apiErrors.get(error) === sourceKey(source);
}

export function codecFor(descriptor: OperationDescriptor<unknown>, codec: CodecRef, direction: 'request' | 'response'): ModelCodec<unknown> {
    const result = (direction === 'request' ? descriptor.requestCodecs : descriptor.responseCodecs)[sourceKey(codec.schema.id)];
    if (result === undefined) throw new RuntimeError(direction === 'request' ? 'request-representation' : 'response-decoding',
        'generated codec binding is missing', descriptor.source, codec.schema.id);
    return result;
}
export function requestFailure(error: unknown, operation: ValidationSource, source: ValidationSource, subject: string): RuntimeError {
    if (isSdkError(error)) return error as RuntimeError;
    if (error instanceof ModelCodecError) {
        if (error.kind === 'invalid') return new RuntimeError('request-validation', `${subject} violates its schema`, operation, error.source ?? source, error);
        if (error.kind === 'limit' || error.kind === 'evaluation') return new RuntimeError('resource-limit', `${subject} exceeded its validation budget`, operation, error.source ?? source, error);
    }
    if (error instanceof JsonCodecError && error.kind === 'limit') return new RuntimeError('resource-limit', `${subject} exceeded its JSON budget`, operation, source, error);
    return new RuntimeError('request-representation', `${subject} cannot be represented`, operation, source, error);
}
export function responseFailure(error: unknown, operation: ValidationSource, source: ValidationSource): RuntimeError {
    if (isSdkError(error)) return error as RuntimeError;
    if (error instanceof ModelCodecError && (error.kind === 'limit' || error.kind === 'evaluation') || error instanceof JsonCodecError && error.kind === 'limit') {
        return new RuntimeError('resource-limit', 'response validation exceeded its budget', operation,
            error instanceof ModelCodecError ? error.source ?? source : source, error);
    }
    return new RuntimeError('response-decoding', 'response did not satisfy its declared representation and schema', operation,
        error instanceof ModelCodecError ? error.source ?? source : source, error);
}
export function encodeJson(value: unknown, descriptor: OperationDescriptor<unknown>, codec: CodecRef | null, source: ValidationSource, maximum: number): string {
    try { return codec === null ? stringifyJson(value, { maxLength: maximum }) : codecFor(descriptor, codec, 'request').encode(value, { maxLength: maximum }); }
    catch (error) { throw requestFailure(error, descriptor.source, source, 'request value'); }
}
export function validatedWire(value: unknown, descriptor: OperationDescriptor<unknown>, codec: CodecRef, maximum: number): WireJsonValue {
    return parseJson(encodeJson(value, descriptor, codec, codec.schema.id, maximum), { maxLength: maximum });
}
export function decodeJson(text: string, descriptor: OperationDescriptor<unknown>, codec: CodecRef | null, source: ValidationSource): unknown {
    try { return codec === null ? parseJson(text, { maxLength: text.length }) : codecFor(descriptor, codec, 'response').decode(text, { maxLength: text.length }); }
    catch (error) { throw responseFailure(error, descriptor.source, source); }
}
export function budget(value: number | undefined, ceiling: number, name: string, source: ValidationSource): number {
    const result = value ?? ceiling;
    if (!Number.isSafeInteger(result) || result < 0 || result > ceiling) {
        throw new RuntimeError('request-representation', `${name} must be a nonnegative safe integer at most ${ceiling}`, source, source);
    }
    return result;
}
export function plainObject(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null);
}
/** Copy own data members without calling getters, iterators, or coercion hooks. */
export function objectData(value: unknown): Record<string, unknown> {
    if (!plainObject(value)) throw new TypeError('a plain object with own data members is required');
    const result: Record<string, unknown> = Object.create(null);
    for (const key of Reflect.ownKeys(value)) {
        const property = Object.getOwnPropertyDescriptor(value, key)!;
        if (typeof key !== 'string' || !('value' in property) || !property.enumerable) throw new TypeError('symbol, accessor, or hidden input member is not supported');
        if (property.value !== undefined) Object.defineProperty(result, key, { value: property.value, enumerable: true });
    }
    return result;
}
export function arrayData(value: unknown): unknown[] {
    if (!Array.isArray(value)) throw new TypeError('an array is required');
    for (const key of Reflect.ownKeys(value)) {
        if (key === 'length') continue;
        if (typeof key !== 'string' || !/^(0|[1-9][0-9]*)$/.test(key) || Number(key) >= value.length || !('value' in Object.getOwnPropertyDescriptor(value, key)!)) {
            throw new TypeError('array has an accessor or extra member');
        }
    }
    const result: unknown[] = [];
    for (let i = 0; i < value.length; i++) {
        const property = Object.getOwnPropertyDescriptor(value, String(i));
        if (property === undefined || !('value' in property) || property.value === undefined) throw new TypeError('array holes and undefined items have no wire representation');
        result.push(property.value);
    }
    return result;
}

export function utf8Length(text: string): number {
    let length = 0;
    for (let i = 0; i < text.length; i++) {
        const code = text.charCodeAt(i);
        if (code < 0x80) length++;
        else if (code < 0x800) length += 2;
        else if (code >= 0xd800 && code <= 0xdbff) {
            const low = text.charCodeAt(++i);
            if (!(low >= 0xdc00 && low <= 0xdfff)) throw new TypeError('unpaired UTF-16 surrogate');
            length += 4;
        } else {
            if (code >= 0xdc00 && code <= 0xdfff) throw new TypeError('unpaired UTF-16 surrogate');
            length += 3;
        }
    }
    return length;
}
export function limitBytes(length: number, maximum: number, operation: ValidationSource, source: ValidationSource, label: string): void {
    if (length > maximum) throw new RuntimeError('resource-limit', `${label} exceeds ${maximum} bytes`, operation, source);
}
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype) as object;
const typedArrayName = Object.getOwnPropertyDescriptor(typedArrayPrototype, Symbol.toStringTag)!.get!;
const typedArraySize = Object.getOwnPropertyDescriptor(typedArrayPrototype, 'byteLength')!.get!;
/** Intrinsic brand/length checks cannot be shadowed by caller properties. */
export function isBytes(value: unknown): value is Uint8Array { return typedArrayName.call(value) === 'Uint8Array'; }
export function byteLength(value: Uint8Array): number { return typedArraySize.call(value) as number; }
export function copyBytes(value: Uint8Array): Uint8Array {
    const result = new Uint8Array(byteLength(value));
    Uint8Array.prototype.set.call(result, value);
    return result;
}
export function encodeUtf8(text: string, maximum: number, operation: ValidationSource, source: ValidationSource): Uint8Array {
    limitBytes(utf8Length(text), maximum, operation, source, 'encoded value');
    return new TextEncoder().encode(text);
}
export function decodeUtf8(bytes: Uint8Array): string { return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes); }
export function capture(bytes: Uint8Array, maximum: number): { rawCapture: string; truncated: boolean } {
    return { rawCapture: new TextDecoder().decode(bytes.subarray(0, maximum)), truncated: bytes.byteLength > maximum };
}
export function cancelError(operation: ValidationSource, source: ValidationSource, signal: AbortSignal): RuntimeError<'cancelled'> {
    return new RuntimeError('cancelled', 'operation was cancelled', operation, source, signal.reason);
}
export function cancelBody(response: Response, reason?: unknown): void {
    if (response.body !== null && !response.body.locked) void response.body.cancel(reason).catch(() => {});
}

/** Owns the per-exchange controller and all caller-signal listener cleanup. */
export class Control {
    readonly controller = new AbortController();
    readonly signal = this.controller.signal;
    readonly aborted: Promise<never>;
    private readonly parentAbort: (() => void) | undefined;
    private readonly rejectAbort: () => void;
    private disposed = false;
    constructor(readonly operation: ValidationSource, private readonly parent?: AbortSignal) {
        let reject!: (error: unknown) => void;
        this.aborted = new Promise<never>((_, rejecter) => { reject = rejecter; });
        // An idle streaming result still owns an abort listener, but no unhandled rejection.
        void this.aborted.catch(() => {});
        this.rejectAbort = () => reject(cancelError(operation, operation, this.signal));
        this.signal.addEventListener('abort', this.rejectAbort, { once: true });
        this.parentAbort = parent === undefined ? undefined : () => this.controller.abort(parent.reason);
        if (this.parentAbort !== undefined) {
            parent!.addEventListener('abort', this.parentAbort, { once: true });
            if (parent!.aborted) this.parentAbort();
        }
    }
    check(source: ValidationSource = this.operation): void {
        if (this.signal.aborted) throw cancelError(this.operation, source, this.signal);
    }
    race<T>(promise: Promise<T>): Promise<T> { this.check(); return Promise.race([promise, this.aborted]); }
    dispose(): void {
        if (this.disposed) return;
        this.disposed = true;
        if (this.parentAbort !== undefined) this.parent!.removeEventListener('abort', this.parentAbort);
        this.signal.removeEventListener('abort', this.rejectAbort);
    }
}

/** One bounded reader, cancelled and unlocked on every completion path. */
export class BodyReader {
    private readonly reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    private size = 0;
    private closed = false;
    private readonly abort: () => void;
    constructor(response: Response, readonly control: Control, readonly source: ValidationSource,
        private readonly maximum: number, private readonly chunkMaximum = maximum) {
        this.reader = response.body?.getReader();
        this.abort = () => this.cancel(cancelError(control.operation, source, control.signal));
        control.signal.addEventListener('abort', this.abort, { once: true });
        if (control.signal.aborted) this.abort();
    }
    async read(): Promise<Uint8Array | undefined> {
        this.control.check(this.source);
        if (this.closed || this.reader === undefined) { this.finish(); return undefined; }
        try {
            const item = await this.control.race(this.reader.read());
            this.control.check(this.source);
            if (item.done) { this.finish(); return undefined; }
            if (!isBytes(item.value)) throw new TypeError('Fetch body chunks must be Uint8Array values');
            const length = byteLength(item.value);
            limitBytes(length, this.chunkMaximum, this.control.operation, this.source, 'transport chunk');
            limitBytes(this.size + length, this.maximum, this.control.operation, this.source, 'response body');
            this.size += length;
            return copyBytes(item.value);
        } catch (error) {
            this.cancel(error);
            if (isSdkError(error)) throw error;
            this.control.check(this.source);
            throw new RuntimeError('transport', 'response body stream failed', this.control.operation, this.source, error);
        }
    }
    finish(): void {
        if (this.closed) return;
        this.closed = true;
        this.control.signal.removeEventListener('abort', this.abort);
        this.reader?.releaseLock();
    }
    cancel(reason?: unknown): void {
        if (this.closed) return;
        this.closed = true;
        this.control.signal.removeEventListener('abort', this.abort);
        if (this.reader !== undefined) {
            const release = () => { try { this.reader!.releaseLock(); } catch { /* retry after cancellation settles */ } };
            void this.reader.cancel(reason).then(release, release);
            release();
        }
    }
    async all(): Promise<Uint8Array> {
        const chunks: Uint8Array[] = [];
        let length = 0;
        for (;;) {
            const chunk = await this.read();
            if (chunk === undefined) break;
            length += byteLength(chunk);
            chunks.push(chunk);
        }
        const result = new Uint8Array(length);
        let offset = 0;
        for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.byteLength; }
        return result;
    }
}

/** Freeze source metadata and JSON error data without executing accessors. */
export function freezeMetadata<T>(value: T): T {
    if (typeof value !== 'object' || value === null || value instanceof Uint8Array || JsonNumber.is(value)) return value;
    for (const property of Object.values(Object.getOwnPropertyDescriptors(value))) if ('value' in property) freezeMetadata(property.value);
    return Object.freeze(value);
}
/** Detached snapshots preserve byte immutability without pretending TypedArrays can be frozen. */
export function errorSnapshot(value: unknown): unknown {
    if (value instanceof Uint8Array) return value.slice();
    if (typeof value !== 'object' || value === null || JsonNumber.is(value) || Symbol.asyncIterator in value) return value;
    if (Array.isArray(value)) return Object.freeze(value.map(errorSnapshot));
    const result = Object.create(null) as Record<string, unknown>;
    for (const [key, property] of Object.entries(Object.getOwnPropertyDescriptors(value))) {
        if ('value' in property) Object.defineProperty(result, key, { value: errorSnapshot(property.value), enumerable: true });
    }
    return Object.freeze(result);
}
