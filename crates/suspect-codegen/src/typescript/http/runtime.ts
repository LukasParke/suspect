import { ModelCodecError, type ModelCodec } from './codecs.js';
import { parseJson, JsonNumber, type JsonLimits, type WireJsonValue } from './json.js';
import type { ValidationSource } from './validation.js';

/** The subset of fetch used by generated clients. */
export type Fetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

/** Explicit credentials, transport, server override and response budgets for generated operations. */
export interface ClientOptions<Scheme extends string = string> {
    /** Credentials keyed by the exact selected source security-scheme name. */
    readonly auth: Readonly<Record<Scheme, string>>;
    /** Explicit server override. The source server remains the descriptor default. */
    readonly serverURL?: string;
    readonly fetch?: Fetch;
    readonly maxResponseBytes?: number;
    readonly maxErrorCaptureBytes?: number;
}

/** Per-call controls that must not persist on a reusable client. */
export interface CallOptions { readonly signal?: AbortSignal }

/** One source-declared HTTP response after its body has passed the generated codec. */
export interface ApiResponse<T, S extends number = number, M extends string = string> {
    readonly status: S;
    readonly contentType: M;
    readonly headers: Headers;
    readonly data: T;
}

/** Recursive immutable view used for validated declared-error bodies; exact JSON numbers retain their nominal representation. */
export type ReadonlyResponseData<T> =
    T extends JsonNumber ? T :
    T extends readonly (infer Item)[] ? readonly ReadonlyResponseData<Item>[] :
    T extends object ? { readonly [Key in keyof T]: ReadonlyResponseData<T[Key]> } :
    T;

/** Generated source binding for an admitted path or form query parameter. */
export interface Parameter<I> {
    readonly name: string;
    readonly location: 'path' | 'query';
    readonly required: boolean;
    readonly explode: boolean;
    readonly array: boolean;
    readonly source: ValidationSource;
    readonly codec: ModelCodec<unknown>;
    readonly read: (input: I) => unknown;
}

/** Generated source binding for an admitted JSON request body. */
export interface RequestBody<I, T> {
    readonly source: ValidationSource;
    readonly required: boolean;
    readonly contentType: 'application/json';
    readonly codec: ModelCodec<T>;
    readonly read: (input: I) => unknown;
}

/** Exact status and media mapping for one admitted source response. */
export interface ResponseDescriptor<T, S extends number = number, M extends string = string> {
    readonly status: S;
    readonly contentType: M;
    readonly source: ValidationSource;
    readonly success: boolean;
    readonly codec: ModelCodec<T>;
}

/** Immutable generated execution description for one canonical OpenAPI operation. */
export interface OperationDescriptor<I, S, E = never, Scheme extends string = string> {
    readonly operationId: string;
    readonly source: ValidationSource;
    readonly method: 'GET' | 'POST' | 'PATCH' | 'PUT' | 'DELETE' | 'HEAD' | 'OPTIONS' | 'TRACE';
    readonly pathTemplate: string;
    readonly serverURL: string;
    /** Generated transport ceiling; caller limits may only reduce it. */
    readonly maxResponseBytes?: number;
    readonly security: {
        readonly kind: 'httpBearer';
        readonly schemeName: Scheme;
        readonly source: ValidationSource;
    };
    readonly parameters: readonly Parameter<I>[];
    readonly inputMembers: readonly string[];
    readonly body?: RequestBody<I, unknown>;
    readonly responses: readonly ResponseDescriptor<unknown>[];
    /** Type-only carrier for the generated operation's declared error union. */
    readonly errorType?: (error: E) => void;
    /** Type-only carrier for the generated operation's success union. */
    readonly successType?: (success: S) => void;
}

/** Stable categories for failures that are not validated, declared API responses. */
export type SdkFailureKind =
    | 'request-validation' | 'request-representation' | 'transport' | 'cancelled'
    | 'resource-limit' | 'unexpected-response' | 'response-decoding';

/** Source-linked request, transport, cancellation, resource or response failure created by this runtime. */
export interface SdkError extends Error {
    readonly kind: SdkFailureKind;
    readonly operationSource: ValidationSource;
    readonly source: ValidationSource | undefined;
}

/** Branded wrapper for a source-declared non-success response whose body passed its generated codec. */
export interface DeclaredApiError<T = unknown, S extends number = number, M extends string = string> extends Error {
    readonly kind: 'api-error';
    readonly operationSource: ValidationSource;
    readonly responseSource: ValidationSource;
    readonly response: ApiResponse<ReadonlyResponseData<T>, S, M>;
}

/** Undeclared status or media response with bounded diagnostic capture. */
export interface UnexpectedResponseError extends SdkError {
    readonly kind: 'unexpected-response';
    readonly status: number;
    readonly contentType: string | null;
    readonly headers: Headers;
    readonly rawCapture: string;
    readonly truncated: boolean;
}

/** Declared status/media response whose bytes, JSON or schema could not be decoded. */
export interface ResponseDecodingError extends SdkError {
    readonly kind: 'response-decoding';
    readonly status: number;
    readonly contentType: string;
    readonly headers: Headers;
    readonly rawCapture: string;
    readonly truncated: boolean;
}

const sdkErrorOperations = new WeakMap<object, string>();
const apiErrorOperations = new WeakMap<object, string>();
const sourceKey = (source: ValidationSource): string => JSON.stringify([source.document, source.pointer]);

class RuntimeError<K extends SdkFailureKind = SdkFailureKind> extends Error implements SdkError {
    override readonly name: string = 'SdkError';
    constructor(
        readonly kind: K,
        message: string,
        readonly operationSource: ValidationSource,
        readonly source: ValidationSource | undefined,
        cause?: unknown,
    ) {
        super(message, cause === undefined ? undefined : { cause });
        sdkErrorOperations.set(this, sourceKey(operationSource));
    }
}

class RuntimeUnexpectedResponseError extends RuntimeError<'unexpected-response'> implements UnexpectedResponseError {
    override readonly name: string = 'UnexpectedResponseError';
    constructor(operation: ValidationSource, readonly status: number, readonly contentType: string | null,
        readonly headers: Headers, readonly rawCapture: string, readonly truncated: boolean, source?: ValidationSource, cause?: unknown) {
        super('unexpected-response', `operation received undeclared response ${status}${contentType === null ? '' : ` ${contentType}`}`, operation, source, cause);
    }
}

class RuntimeResponseDecodingError extends RuntimeError<'response-decoding'> implements ResponseDecodingError {
    override readonly name: string = 'ResponseDecodingError';
    constructor(operation: ValidationSource, source: ValidationSource, readonly status: number, readonly contentType: string,
        readonly headers: Headers, readonly rawCapture: string, readonly truncated: boolean, cause: unknown) {
        super('response-decoding', `declared ${status} ${contentType} response did not satisfy its schema`, operation, source, cause);
    }
}

class RuntimeApiError<T, S extends number, M extends string> extends Error implements DeclaredApiError<T, S, M> {
    override readonly name = 'ApiError';
    readonly kind = 'api-error' as const;
    constructor(readonly operationSource: ValidationSource, readonly responseSource: ValidationSource,
        readonly response: ApiResponse<ReadonlyResponseData<T>, S, M>) {
        super(`operation returned declared API error ${response.status} ${response.contentType}`);
        apiErrorOperations.set(this, sourceKey(operationSource));
    }
}

/** Accepts only errors created by this runtime for the exact generated operation. */
export function isDeclaredApiError(error: unknown, operationSource: ValidationSource): error is DeclaredApiError {
    return typeof error === 'object' && error !== null && apiErrorOperations.get(error) === sourceKey(operationSource);
}

/** Accepts only non-API failures created by this runtime for the exact operation. */
export function isSdkError(error: unknown, operationSource?: ValidationSource): error is SdkError {
    return typeof error === 'object' && error !== null && sdkErrorOperations.has(error) &&
        (operationSource === undefined || sdkErrorOperations.get(error) === sourceKey(operationSource));
}

const DEFAULT_RESPONSE_BYTES = 16 * 1024 * 1024;
const DEFAULT_CAPTURE_BYTES = 64 * 1024;
const isAborted = (signal: AbortSignal | undefined): boolean => signal?.aborted === true;

function budget(value: number | undefined, fallback: number, name: string, operation: ValidationSource): number {
    const result = value ?? fallback;
    if (!Number.isSafeInteger(result) || result < 0) throw new RuntimeError('request-representation', `${name} must be a nonnegative safe integer`, operation, operation);
    return result;
}

function cancelled(operation: ValidationSource, source: ValidationSource, signal: AbortSignal, cause?: unknown): RuntimeError {
    return new RuntimeError('cancelled', 'operation was cancelled', operation, source, cause ?? signal.reason);
}

function requestCodecFailure(error: unknown, operation: ValidationSource, source: ValidationSource, subject: string): RuntimeError {
    if (error instanceof ModelCodecError) {
        if (error.kind === 'invalid') return new RuntimeError('request-validation', `${subject} violates its schema`, operation, error.source ?? source, error);
        if (error.kind === 'limit' || error.kind === 'evaluation') return new RuntimeError('resource-limit', `${subject} validation did not complete within its budget`, operation, error.source ?? source, error);
    }
    return new RuntimeError('request-representation', `${subject} cannot be represented as source JSON`, operation, source, error);
}

function pathValue(value: unknown, parameter: Parameter<unknown>, operation: ValidationSource): string {
    if (typeof value !== 'string') throw new RuntimeError('request-validation', `path parameter ${parameter.name} must be a string`, operation, parameter.source);
    try { parameter.codec.encode(value); }
    catch (error) { throw requestCodecFailure(error, operation, parameter.source, `path parameter ${parameter.name}`); }
    if (value === '.' || value === '..') {
        throw new RuntimeError('request-representation', `path parameter ${parameter.name} would be normalized as a dot segment`, operation, parameter.source);
    }
    try { return encodeURIComponent(value).replace(/[!'()*]/g, character => `%${character.charCodeAt(0).toString(16).toUpperCase()}`); }
    catch (error) { throw new RuntimeError('request-representation', `path parameter ${parameter.name} cannot be represented as UTF-8`, operation, parameter.source, error); }
}

function queryPairs(value: unknown, parameter: Parameter<unknown>, operation: ValidationSource): string[] {
    let wire: WireJsonValue;
    // Serialize once through the source codec, then use only this validated snapshot.
    // No coercion hooks or caller-owned array getters run during URI construction.
    try { wire = parseJson(parameter.codec.encode(value)); }
    catch (error) { throw requestCodecFailure(error, operation, parameter.source, `query parameter ${parameter.name}`); }
    const encode = (text: string): string => encodeURIComponent(text).replace(/[!'()*]/g, character => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);
    const scalar = (item: WireJsonValue): string => {
        if (typeof item === 'string') return encode(item);
        if (typeof item === 'boolean') return item ? 'true' : 'false';
        if (JsonNumber.is(item)) return encode(JsonNumber.prototype.toString.call(item));
        throw new TypeError('query values must have an admitted non-null scalar representation');
    };
    try {
        const name = encode(parameter.name);
        if (!parameter.array) return [`${name}=${scalar(wire)}`];
        if (!Array.isArray(wire)) throw new TypeError('query array representation required');
        if (wire.length === 0) {
            if (parameter.required) throw new TypeError('a required query array cannot be represented by zero form pairs');
            return [];
        }
        const values = wire.map(scalar);
        return parameter.explode ? values.map(item => `${name}=${item}`) : [`${name}=${values.join(',')}`];
    } catch (error) {
        throw new RuntimeError('request-representation', `query parameter ${parameter.name} cannot be represented as form UTF-8`, operation, parameter.source, error);
    }
}

function requestURL<I>(descriptor: OperationDescriptor<I, unknown, unknown>, input: I, serverURL: string): URL {
    let path = descriptor.pathTemplate;
    const query: string[] = [];
    for (const parameter of descriptor.parameters) {
        const value = parameter.read(input);
        if (value === undefined) {
            if (parameter.required) throw new RuntimeError('request-validation', `required ${parameter.location} parameter ${parameter.name} is absent`, descriptor.source, parameter.source);
            continue;
        }
        if (parameter.location === 'query') {
            for (const pair of queryPairs(value, parameter as Parameter<unknown>, descriptor.source)) query.push(pair);
            continue;
        }
        const marker = `{${parameter.name}}`;
        if (!path.includes(marker)) throw new RuntimeError('request-representation', `path template does not contain ${marker}`, descriptor.source, parameter.source);
        path = path.replaceAll(marker, pathValue(value, parameter as Parameter<unknown>, descriptor.source));
    }
    if (/\{[^}]+\}/.test(path)) throw new RuntimeError('request-representation', 'path template contains an unbound parameter', descriptor.source, descriptor.source);
    if (path.includes('\\')) throw new RuntimeError('request-representation', 'operation path contains an unsupported backslash', descriptor.source, descriptor.source);
    try {
        const base = new URL(serverURL);
        if (base.protocol !== 'https:' && base.protocol !== 'http:') throw new TypeError('server URL must use HTTP or HTTPS');
        if (base.username !== '' || base.password !== '') throw new TypeError('server URL must not contain credentials');
        if (base.hash !== '' || base.search !== '') throw new TypeError('server URL must not contain a query or fragment');
        const wanted = `${base.pathname.replace(/\/$/, '')}/${path.replace(/^\//, '')}`;
        base.pathname = wanted;
        if (base.pathname !== wanted) throw new TypeError('operation route would be normalized to a different path');
        if (query.length !== 0) base.search = `?${query.join('&')}`;
        return base;
    } catch (error) {
        throw new RuntimeError('request-representation', 'server URL or operation path is invalid', descriptor.source, descriptor.source, error);
    }
}

async function readBounded(response: Response, maximum: number, signal: AbortSignal | undefined,
    operation: ValidationSource, source: ValidationSource): Promise<Uint8Array> {
    if (isAborted(signal)) throw cancelled(operation, source, signal!);
    if (response.body === null) return new Uint8Array();
    const reader = response.body.getReader();
    let releaseDeferred = false;
    const chunks: Uint8Array[] = [];
    let size = 0;
    let abort: (() => void) | undefined;
    const aborted = signal === undefined ? undefined : new Promise<never>((_, reject) => {
        abort = () => reject(cancelled(operation, source, signal));
        if (signal.aborted) { abort(); return; }
        signal.addEventListener('abort', abort, { once: true });
        if (signal.aborted) abort();
    });
    try {
        for (;;) {
            if (isAborted(signal)) throw cancelled(operation, source, signal!);
            const item = await (aborted === undefined ? reader.read() : Promise.race([reader.read(), aborted]));
            if (item.done) break;
            size += item.value.byteLength;
            if (size > maximum) {
                throw new RuntimeError('resource-limit', `response body exceeds ${maximum} bytes`, operation, source);
            }
            chunks.push(item.value);
        }
        const bytes = new Uint8Array(size);
        let at = 0;
        for (const chunk of chunks) { bytes.set(chunk, at); at += chunk.byteLength; }
        return bytes;
    } catch (error) {
        releaseDeferred = true;
        void reader.cancel(error)
            .catch(() => { /* the original failure is authoritative */ })
            .finally(() => { try { reader.releaseLock(); } catch { /* an uncooperative stream retains its own lock */ } });
        if (isSdkError(error)) throw error;
        if (isAborted(signal)) throw cancelled(operation, source, signal!, error);
        throw new RuntimeError('transport', 'response body stream failed', operation, source, error);
    } finally {
        if (abort !== undefined) signal!.removeEventListener('abort', abort);
        if (!releaseDeferred) reader.releaseLock();
    }
}

function utf8(bytes: Uint8Array, operation: ValidationSource, source: ValidationSource): string {
    try { return new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
    catch (error) { throw new RuntimeError('response-decoding', 'response body is not valid UTF-8', operation, source, error); }
}

function capture(bytes: Uint8Array, maximum: number): { rawCapture: string; truncated: boolean } {
    const selected = bytes.subarray(0, maximum);
    return { rawCapture: new TextDecoder('utf-8').decode(selected), truncated: bytes.byteLength > selected.byteLength };
}

function mediaType(value: string | null): string | null {
    if (value === null) return null;
    const token = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+/;
    let at = 0;
    const part = (): string | undefined => {
        const match = token.exec(value.slice(at));
        if (match === null) return undefined;
        at += match[0].length;
        return match[0];
    };
    const type = part();
    if (type === undefined || value[at++] !== '/') return null;
    const subtype = part();
    if (subtype === undefined) return null;
    const parameters = new Set<string>();
    for (;;) {
        while (value[at] === ' ' || value[at] === '\t') at++;
        if (at === value.length) return `${type}/${subtype}`.toLowerCase();
        if (value[at++] !== ';') return null;
        while (value[at] === ' ' || value[at] === '\t') at++;
        const name = part()?.toLowerCase();
        if (name === undefined || parameters.has(name)) return null;
        parameters.add(name);
        if (value[at++] !== '=') return null;
        if (value[at] === '"') {
            at++;
            let closed = false;
            while (at < value.length) {
                const code = value.charCodeAt(at++);
                if (code === 34) { closed = true; break; }
                if (code === 92) { if (at >= value.length || value.charCodeAt(at++) > 127) return null; continue; }
                if (code === 9 || (code >= 32 && code !== 127)) continue;
                return null;
            }
            if (!closed) return null;
        } else if (part() === undefined) return null;
    }
}

function freezeDecoded(value: unknown): void {
    if ((typeof value !== 'object' && typeof value !== 'function') || value === null) return;
    const pending: object[] = [value];
    const seen = new WeakSet<object>();
    while (pending.length !== 0) {
        const object = pending.pop()!;
        if (seen.has(object)) continue;
        seen.add(object);
        for (const descriptor of Object.values(Object.getOwnPropertyDescriptors(object))) {
            if ('value' in descriptor && ((typeof descriptor.value === 'object' && descriptor.value !== null) || typeof descriptor.value === 'function')) {
                pending.push(descriptor.value as object);
            }
        }
        Object.freeze(object);
    }
}

/** Executes exactly one generated operation. It performs no retry or redirect following. */
export async function executeOperation<I, S, E = never, Scheme extends string = string>(descriptor: OperationDescriptor<I, S, E, Scheme>, input: I,
    client: ClientOptions<Scheme>, call: CallOptions = {}): Promise<S> {
    const signal = call.signal;
    if (isAborted(signal)) throw cancelled(descriptor.source, descriptor.source, signal!);
    if (typeof input !== 'object' || input === null || (Object.getPrototypeOf(input) !== Object.prototype && Object.getPrototypeOf(input) !== null)) {
        throw new RuntimeError('request-validation', 'operation input must be a plain object with own data members', descriptor.source, descriptor.source);
    }
    for (const key of Reflect.ownKeys(input)) {
        const property = Object.getOwnPropertyDescriptor(input, key)!;
        if (typeof key !== 'string' || !descriptor.inputMembers.includes(key) || !('value' in property)) {
            throw new RuntimeError('request-validation', 'operation input contains an undeclared or accessor member', descriptor.source, descriptor.source);
        }
    }
    const ceiling = budget(descriptor.maxResponseBytes, DEFAULT_RESPONSE_BYTES, 'descriptor.maxResponseBytes', descriptor.source);
    const maximum = budget(client.maxResponseBytes, ceiling, 'maxResponseBytes', descriptor.source);
    if (maximum > ceiling) throw new RuntimeError('request-representation', `maxResponseBytes exceeds the generated ${ceiling} byte ceiling`, descriptor.source, descriptor.source);
    const captureMaximum = budget(client.maxErrorCaptureBytes, DEFAULT_CAPTURE_BYTES, 'maxErrorCaptureBytes', descriptor.source);
    const url = requestURL(descriptor as OperationDescriptor<I, unknown, unknown>, input, client.serverURL ?? descriptor.serverURL);
    const schemeName = descriptor.security.schemeName;
    if (typeof client.auth !== 'object' || client.auth === null || !Object.hasOwn(client.auth, schemeName) || typeof client.auth[schemeName] !== 'string') {
        throw new RuntimeError('request-validation', `auth must contain an own string credential for ${schemeName}`, descriptor.source, descriptor.security.source);
    }
    if (!/^[A-Za-z0-9\-._~+/]+={0,}$/.test(client.auth[schemeName])) {
        throw new RuntimeError('request-validation', `auth credential for ${schemeName} is not an RFC 6750 bearer token`, descriptor.source, descriptor.security.source);
    }
    const headers = new Headers();
    try {
        headers.set('accept', [...new Set(descriptor.responses.map(response => response.contentType))].join(', '));
        headers.set('authorization', `Bearer ${client.auth[schemeName]}`);
    } catch (error) {
        throw new RuntimeError('request-representation', 'request headers cannot be represented', descriptor.source, descriptor.security.source, error);
    }
    let body: string | undefined;
    if (descriptor.body !== undefined) {
        const value = descriptor.body.read(input);
        if (value === undefined && descriptor.body.required) throw new RuntimeError('request-validation', 'required request body is absent', descriptor.source, descriptor.body.source);
        if (value !== undefined) {
            try { body = descriptor.body.codec.encode(value); }
            catch (error) { throw requestCodecFailure(error, descriptor.source, descriptor.body.source, 'request body'); }
            headers.set('content-type', descriptor.body.contentType);
        }
    }
    const transport = client.fetch ?? globalThis.fetch;
    if (typeof transport !== 'function') throw new RuntimeError('transport', 'no fetch implementation is available', descriptor.source, descriptor.source);
    let response: Response;
    try {
        response = await transport(url, { method: descriptor.method, headers, ...(body === undefined ? {} : { body }),
            ...(signal === undefined ? {} : { signal }), redirect: 'error', credentials: 'omit' });
    } catch (error) {
        if (isAborted(signal)) throw cancelled(descriptor.source, descriptor.source, signal!, error);
        throw new RuntimeError('transport', 'request transport failed', descriptor.source, descriptor.source, error);
    }
    if (isAborted(signal)) throw cancelled(descriptor.source, descriptor.source, signal!);
    const contentType = mediaType(response.headers.get('content-type'));
    const statusCandidate = descriptor.responses.find(candidate => candidate.status === response.status);
    const declared = descriptor.responses.find(candidate => candidate.status === response.status && candidate.contentType.toLowerCase() === contentType);
    const source = declared?.source ?? statusCandidate?.source ?? descriptor.source;
    const bytes = await readBounded(response, maximum, signal, descriptor.source, source);
    const raw = capture(bytes, captureMaximum);
    if (declared === undefined || contentType === null) {
        throw new RuntimeUnexpectedResponseError(descriptor.source, response.status, contentType, response.headers, raw.rawCapture, raw.truncated, source);
    }
    let data: unknown;
    try {
        const text = utf8(bytes, descriptor.source, declared.source);
        const limits: JsonLimits = { maxLength: text.length };
        data = declared.codec.decode(text, limits);
    } catch (error) {
        if (error instanceof ModelCodecError && (error.kind === 'limit' || error.kind === 'evaluation')) {
            throw new RuntimeError('resource-limit', 'response validation did not complete within its budget', descriptor.source, error.source ?? declared.source, error);
        }
        if (isSdkError(error) && error.kind === 'response-decoding') {
            throw new RuntimeResponseDecodingError(descriptor.source, declared.source, response.status, contentType, response.headers, raw.rawCapture, raw.truncated, error);
        }
        throw new RuntimeResponseDecodingError(descriptor.source, declared.source, response.status, contentType, response.headers, raw.rawCapture, raw.truncated, error);
    }
    const result: ApiResponse<unknown> = Object.freeze({ status: response.status, contentType, headers: response.headers, data });
    if (!declared.success) { freezeDecoded(data); throw new RuntimeApiError(descriptor.source, declared.source, result); }
    return result as S;
}

/** Freezes a reusable option object without discovering credentials or transport state. */
export function createRuntimeClient<Scheme extends string>(options: ClientOptions<Scheme>): Readonly<ClientOptions<Scheme>> {
    return Object.freeze({ ...options, auth: Object.freeze({ ...options.auth }) });
}
