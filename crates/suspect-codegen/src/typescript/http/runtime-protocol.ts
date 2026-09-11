import { stringifyJson } from './json.js';
import type { ApiResponse, CallOptions, ClientOptions as TransportOptions, CredentialValue, MediaPlan, OperationDescriptor, RuntimeLimits } from './http/types.js';
import { ApiError, BodyReader, Control, RuntimeError, budget, byteLength, copyBytes, isBytes, cancelBody, capture, decodeJson, decodeUtf8, encodeJson, utf8Length, errorSnapshot, freezeMetadata, isSdkError, limitBytes, objectData, plainObject, requestFailure, responseFailure, validatedWire } from './http/common.js';
import { canonicalMedia, forbiddenBody, parseMedia, selectMedia, selectResponse } from './http/media.js';
import { decodeForm, decodeMultipart, encodeForm, encodeMultipart } from './http/parts.js';
import { credentials, serverURL, setHeader } from './http/security.js';
import { encodeItems, streamItems } from './http/streams.js';
import { decodeHeaders, percentEncode, scalarText, scalarWire, serialize } from './http/wire.js';
import { parseUriReference } from './uri.js';

export type { ApiResponse, AuthorizationCredential, BasicCredential, BinaryPart, CallOptions, Credential, CredentialContext, CredentialProvider, CredentialRequirement, DeclaredApiError, Fetch, LinkMetadata, Located, MediaBody, OAuthFlow, Part, Provenance, ReadonlyBytes, ReadonlyResponseData, ResourceContext, ResponseDecodingError, SdkError, SdkFailureKind, ServerChoice, ServerPlan, SourceLocation, UnexpectedResponseError } from './http/types.js';
export { freezeMetadata, isDeclaredApiError, isSdkError } from './http/common.js';
export type { CredentialValue, Location } from './http/types.js';

/** Runtime options; generated clients retain literal scheme names and narrower credential types. */
export interface ClientOptions<Scheme extends string = string, Auth extends object = Readonly<Record<Scheme, CredentialValue>>> extends TransportOptions<Auth> {}

function limitsFor(descriptor: OperationDescriptor<unknown>, client: TransportOptions<object>): RuntimeLimits {
    const limits = descriptor.limits, source = descriptor.source;
    return {
        response: budget(client.maxResponseBytes, limits.response, 'maxResponseBytes', source),
        request: budget(client.maxRequestBytes, limits.request, 'maxRequestBytes', source),
        part: budget(client.maxPartBytes, limits.part, 'maxPartBytes', source),
        streamItem: budget(client.maxStreamItemBytes, limits.streamItem, 'maxStreamItemBytes', source),
        streamBuffer: budget(client.maxStreamBufferBytes, limits.streamBuffer, 'maxStreamBufferBytes', source),
        streamItems: budget(client.maxStreamItems, limits.streamItems, 'maxStreamItems', source),
    };
}
async function encodeBody(value: unknown, media: MediaPlan, contentType: string, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits, control: Control): Promise<{ body: string | ArrayBuffer; contentType: string }> {
    const source = media.source.terminal.source;
    const representation = media.representation;
    let bytes: Uint8Array;
    try {
        switch (representation.kind) {
            case 'json': {
                const text = encodeJson(value, descriptor, representation.codec, source, limits.request);
                limitBytes(utf8Length(text), limits.request, descriptor.source, source, 'JSON request');
                return { body: text, contentType };
            }
            case 'text': {
                const wire = representation.codec === null ? value : validatedWire(value, descriptor, representation.codec, limits.request);
                const text = scalarText(wire as Parameters<typeof scalarText>[0], representation.scalar);
                limitBytes(utf8Length(text), limits.request, descriptor.source, source, 'text request');
                return { body: text, contentType };
            }
            case 'binary': {
                if (!isBytes(value)) throw new TypeError('binary body must be in-memory Uint8Array bytes');
                limitBytes(byteLength(value), Math.min(limits.request, representation.bytes.max_bytes), descriptor.source, source, 'binary request');
                bytes = copyBytes(value); break;
            }
            case 'form': bytes = encodeForm(value, representation.form, descriptor, limits); break;
            case 'multipart': {
                const encoded = encodeMultipart(value, representation.multipart, contentType, descriptor, limits);
                return { body: encoded.bytes.buffer as ArrayBuffer, contentType: encoded.contentType };
            }
            case 'stream': bytes = await encodeItems(value, representation.stream, descriptor, limits, control); break;
        }
        limitBytes(bytes.byteLength, limits.request, descriptor.source, source, 'request body');
        return { body: bytes.buffer as ArrayBuffer, contentType };
    } catch (error) { throw requestFailure(error, descriptor.source, source, 'request body'); }
}
function decodeBody(bytes: Uint8Array, media: MediaPlan, contentType: string, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    const source = media.source.terminal.source, representation = media.representation;
    try {
        switch (representation.kind) {
            case 'json': return decodeJson(decodeUtf8(bytes), descriptor, representation.codec, source);
            case 'text': {
                const wire = scalarWire(decodeUtf8(bytes), representation.scalar);
                return representation.codec === null ? wire : decodeJson(stringifyJson(wire), descriptor, representation.codec, source);
            }
            case 'binary': limitBytes(bytes.byteLength, representation.bytes.max_bytes, descriptor.source, source, 'binary response'); return bytes;
            case 'form': return decodeForm(bytes, representation.form, descriptor, limits);
            case 'multipart': return decodeMultipart(bytes, representation.multipart, contentType, descriptor, limits);
            case 'stream': throw new TypeError('streaming representation requires its native item iterator');
        }
    } catch (error) { throw responseFailure(error, descriptor.source, source); }
}

/** Executes one source-planned exchange with pull-driven streaming and no retries. */
export async function executeOperation<I, S>(descriptor: OperationDescriptor<I>, input: I, client: TransportOptions<object> = {}, call: CallOptions = {}): Promise<S> {
    const control = new Control(descriptor.source, call.signal);
    let response: Response | undefined;
    let streamOwnsControl = false;
    try {
        control.check();
        let data: Record<string, unknown>;
        try {
            data = objectData(input);
            if (Object.keys(data).some(key => !descriptor.inputMembers.includes(key))) throw new TypeError('undeclared operation input member');
        } catch (error) { throw new RuntimeError('request-validation', 'operation input must have only declared own data members', descriptor.source, descriptor.source, error); }
        const limits = limitsFor(descriptor, client);
        const captureMaximum = budget(client.maxErrorCaptureBytes, 64 * 1024, 'maxErrorCaptureBytes', descriptor.source);
        const headers = new Headers();
        const query: string[] = [], cookies: string[] = [];
        let path = descriptor.wire.path;
        for (let index = 0; index < descriptor.wire.parameters.length; index++) {
            const parameter = descriptor.wire.parameters[index]!;
            const source = parameter.source.terminal.source;
            const value = data[descriptor.parameterMembers[index]!];
            if (value === undefined) {
                if (parameter.required) throw new RuntimeError('request-validation', `required ${parameter.location} parameter ${parameter.name} is absent`, descriptor.source, source);
                continue;
            }
            const wire = validatedWire(value, descriptor, parameter.codec, limits.request);
            // Preserve the original SDK's optional empty form-array convenience.
            if (!parameter.required && parameter.location === 'query' && parameter.serialization.kind === 'style' && parameter.serialization.style === 'form' && parameter.serialization.shape.kind === 'array' && Array.isArray(wire) && wire.length === 0) continue;
            let text: string;
            try {
                if (parameter.location === 'querystring' && parameter.content_media?.representation.kind === 'form') {
                    text = decodeUtf8(encodeForm(wire, parameter.content_media.representation.form, descriptor, limits, false));
                } else text = serialize(parameter.name, parameter.location, parameter.serialization, wire);
                if (parameter.location === 'querystring') limitBytes(utf8Length(text), limits.request, descriptor.source, source, 'encoded querystring');
            }
            catch (error) { throw new RuntimeError('request-representation', `parameter ${parameter.name} has no unambiguous wire representation`, descriptor.source, source, error); }
            if (parameter.location === 'path') path = path.replaceAll(`{${parameter.name}}`, text);
            else if (parameter.location === 'query' || parameter.location === 'querystring') query.push(text);
            else if (parameter.location === 'cookie') cookies.push(text);
            else setHeader(headers, parameter.name, text, descriptor.source, source);
        }
        const accepts = [...new Set(descriptor.wire.responses.flatMap(response => response.media.map(media => canonicalMedia(media.media_type))))];
        if (accepts.length !== 0) setHeader(headers, 'Accept', accepts.join(', '), descriptor.source, descriptor.source);
        const effectiveServerURL = serverURL(descriptor, client, call);
        let target: string;
        try {
            if (/[{}\\?#]/.test(path)) throw new TypeError('operation route contains an unbound template or unsafe delimiter');
            const base = parseUriReference(effectiveServerURL);
            const wanted = percentEncode(`${base.path.replace(/\/$/, '')}/${path.replace(/^\//, '')}`, 'reserved-expansion');
            target = `${base.scheme}://${base.authority}${wanted}`;
        } catch (error) { throw new RuntimeError('request-representation', 'operation path cannot be represented unchanged', descriptor.source, descriptor.source, error); }
        await credentials(descriptor, client, call, control, headers, query, cookies, effectiveServerURL);
        if (cookies.length !== 0) setHeader(headers, 'Cookie', cookies.join('; '), descriptor.source, descriptor.source);
        if (query.length !== 0) {
            target += `?${query.join('&')}`;
        }
        let body: string | ArrayBuffer | undefined;
        const bodyPlan = descriptor.wire.body;
        if (bodyPlan !== null) {
            let value = data.body;
            if (value === undefined && bodyPlan.required) throw new RuntimeError('request-validation', 'required request body is absent', descriptor.source, bodyPlan.source.terminal.source);
            if (value !== undefined) {
                let contentType: string;
                try {
                    if (descriptor.taggedBody) {
                        const choice = objectData(value);
                        if (!Object.hasOwn(choice, 'data') || typeof choice.contentType !== 'string' || Object.keys(choice).some(key => !['data','contentType','mediaType'].includes(key))) throw new TypeError('body media choice requires contentType and data');
                        contentType = choice.contentType; value = choice.data;
                    } else contentType = canonicalMedia(bodyPlan.media[0]!.media_type);
                    const media = selectMedia(bodyPlan.media, parseMedia(contentType));
                    if (media === undefined) throw new TypeError('request Content-Type does not match any declared representation');
                    if (descriptor.taggedBody) {
                        const choice = objectData(data.body);
                        if (media.media_type.range.kind !== 'concrete' && choice.mediaType === undefined || choice.mediaType !== undefined && choice.mediaType !== canonicalMedia(media.media_type)) {
                            throw new TypeError('wildcard bodies require their matched source mediaType; a broad declaration cannot bypass a more-specific codec');
                        }
                    }
                    const encoded = await encodeBody(value, media, contentType, descriptor, limits, control);
                    body = encoded.body;
                    setHeader(headers, 'Content-Type', encoded.contentType, descriptor.source, bodyPlan.source.terminal.source);
                } catch (error) { throw requestFailure(error, descriptor.source, bodyPlan.source.terminal.source, 'request body media choice'); }
            }
        }
        control.check();
        const transport = client.fetch ?? globalThis.fetch;
        if (typeof transport !== 'function') throw new RuntimeError('transport', 'no Fetch transport is available', descriptor.source, descriptor.source);
        const init: RequestInit = { method: descriptor.wire.method, headers, ...(body === undefined ? {} : { body }), signal: control.signal, redirect: 'error', credentials: 'omit' };
        if (client.fetch === undefined) {
            // Native browsers forbid Cookie and TRACE and may silently strip
            // headers. Detect that before transport instead of claiming success.
            try {
                const request = new Request(target, init);
                if (request.url !== target) throw new TypeError('native Fetch would normalize the exact server, route or query');
                if (request.method !== descriptor.wire.method) throw new TypeError(`native Fetch would normalize method ${descriptor.wire.method} to ${request.method}`);
                for (const [name, value] of headers) if (request.headers.get(name) !== value) throw new TypeError(`native Fetch cannot send declared header ${name}`);
            } catch (error) { throw new RuntimeError('request-representation', 'native Fetch cannot represent this URL, method, body or header; use a capable explicit transport', descriptor.source, descriptor.source, error); }
        }
        try {
            const pending = transport(target, init);
            // A caller transport may ignore AbortSignal and resolve after the
            // public call rejects. Its body must still be cancelled.
            void pending.then(late => { if (control.signal.aborted) cancelBody(late, control.signal.reason); }, () => {});
            response = await control.race(pending);
        } catch (error) {
            if (isSdkError(error)) throw error;
            control.check();
            throw new RuntimeError('transport', 'request transport failed', descriptor.source, descriptor.source, error);
        }
        control.check();
        const selected = selectResponse(descriptor.wire, response.status);
        const source = selected?.source.terminal.source ?? descriptor.source;
        const suppressed = forbiddenBody(descriptor.wire.method, response.status);
        const rawContentType = response.headers.get('content-type');
        let media: MediaPlan | undefined, contentType: string | null = null, mediaError: unknown;
        if (selected !== undefined && !suppressed && selected.media.length !== 0) {
            try {
                if (rawContentType === null) throw new TypeError('declared content requires a real Content-Type');
                const actual = parseMedia(rawContentType);
                media = selectMedia(selected.media, actual);
                if (media === undefined) throw new TypeError('Content-Type does not match the selected status declaration');
                contentType = canonicalMedia(media.media_type, actual);
            } catch (error) { mediaError = error; }
        }
        if (selected === undefined || mediaError !== undefined) {
            const bytes = suppressed ? new Uint8Array() : await new BodyReader(response, control, source, limits.response).all();
            cancelBody(response);
            const raw = capture(bytes, captureMaximum);
            throw Object.assign(new RuntimeError('unexpected-response', `undeclared response ${response.status}${rawContentType === null ? '' : ` ${rawContentType}`}`, descriptor.source, source, mediaError),
                { name: 'UnexpectedResponseError', status: response.status, contentType: rawContentType, headers: response.headers, ...raw });
        }
        let typedHeaders: Record<string, unknown>;
        try { typedHeaders = decodeHeaders(response.headers, selected.headers, descriptor); }
        catch (error) {
            const failure = responseFailure(error, descriptor.source, source);
            throw Object.assign(failure, { ...(failure.kind === 'response-decoding' ? {name:'ResponseDecodingError'} : {}), status: response.status, contentType: rawContentType, headers: response.headers, rawCapture: '', truncated: false });
        }
        const links = Object.create(null) as Record<string, unknown>;
        for (const link of selected.links) Object.defineProperty(links, link.name, { value: freezeMetadata(link), enumerable: true });
        Object.freeze(links);
        let responseData: unknown;
        if (suppressed) { cancelBody(response); responseData = undefined; }
        else if (media?.representation.kind === 'stream') {
            responseData = streamItems(response, media.representation.stream, descriptor,
                { ...limits, response: Math.min(limits.response, selected.max_body_bytes) }, control);
            streamOwnsControl = true;
        } else {
            const bytes = await new BodyReader(response, control, source, Math.min(limits.response, selected.max_body_bytes,
                media?.representation.kind === 'binary' ? media.representation.bytes.max_bytes : limits.response)).all();
            try { responseData = media === undefined ? bytes : decodeBody(bytes, media, rawContentType!, descriptor, limits); }
            catch (error) {
                const failure = responseFailure(error, descriptor.source, source);
                throw Object.assign(failure, { ...(failure.kind === 'response-decoding' ? {name:'ResponseDecodingError'} : {}), status: response.status, contentType: rawContentType, headers: response.headers, ...capture(bytes, captureMaximum) });
            }
        }
        const success = response.status >= 200 && response.status < 300;
        const record = { status: response.status, contentType, mediaType: media === undefined ? null : canonicalMedia(media.media_type), rawContentType, headers: response.headers, typedHeaders, links };
        Object.defineProperty(record, 'data', success ? { value: responseData, enumerable: true } : { get: () => errorSnapshot(responseData), enumerable: true });
        const result = Object.freeze(record) as ApiResponse<unknown, number, string | null, object, object>;
        if (!success) throw new ApiError(descriptor.source, selected.source.use_site.source, result);
        return result as S;
    } catch (error) {
        if (!streamOwnsControl && response !== undefined) cancelBody(response, error);
        throw error;
    } finally { if (!streamOwnsControl) control.dispose(); }
}

/** Snapshot reusable options while keeping explicit credential callbacks callable. */
export function createRuntimeClient<Options extends TransportOptions<object>>(options: Options): Readonly<Options> {
    const given = options.auth === undefined ? undefined : objectData(options.auth);
    const auth = given === undefined ? undefined : Object.fromEntries(Object.entries(given).map(([key, value]) => [key, plainObject(value) ? Object.freeze(objectData(value)) : value]));
    const server = options.server === undefined ? undefined : { ...options.server,
        ...(options.server.variables === undefined ? {} : { variables: Object.freeze({ ...options.server.variables }) }) };
    return Object.freeze({ ...options, ...(auth === undefined ? {} : { auth: Object.freeze(auth) }), ...(server === undefined ? {} : { server: Object.freeze(server) }) });
}
