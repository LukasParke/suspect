import { JsonNumber, stringifyJson, type WireJsonValue } from '../json.js';
import type { ValidationSource } from '../validation.js';
import type { AdditionalParts, FormPlan, HeaderPlan, MultipartPlan, ObjectRules, OperationDescriptor, PartPlan, RuntimeLimits, Serialization, WireShape } from './types.js';
import { RuntimeError, arrayData, byteLength, copyBytes, isBytes, decodeJson, decodeUtf8, encodeJson, encodeUtf8, limitBytes, objectData, requestFailure, responseFailure, sourceKey, validatedWire } from './common.js';
import { canonicalMedia, matchesMedia, parseMedia, parseParameters, splitQuoted } from './media.js';
import { compareKeys, decodeComposite, decodeHeaders, percentDecode, percentEncode, propertyScalar, scalarText, scalarWire, serialize, token, type HeaderReader } from './wire.js';
import { expandStyle, expandsStyle, ownsStyleField, parseStyle } from './multipart-style.js';

const MAX_PARTS = 10_000;
const MAX_PART_HEADER_BYTES = 64 * 1024;
const MAX_PART_HEADERS = 100;

export function wrappedPart(part: PartPlan): boolean {
    return part.headers.length !== 0 || part.content_types.length > 1 || part.content_types.some(media => media.range.kind !== 'concrete');
}
function checkCount(count: number, minimum: number | JsonNumber | undefined, maximum: number | JsonNumber | undefined): void {
    const exact = (value: number | JsonNumber): bigint => {
        if (typeof value === 'number') return BigInt(value);
        const integer = JsonNumber.prototype.toBigInt.call(value, 20);
        if (integer === undefined) throw new TypeError('invalid generated cardinality');
        return integer;
    };
    if (minimum !== undefined && BigInt(count) < exact(minimum) || maximum !== undefined && BigInt(count) > exact(maximum)) throw new TypeError('form/multipart cardinality violates the source schema');
}
function objectRules(data: Record<string, unknown>, rules: ObjectRules, parts: readonly PartPlan[], additional: AdditionalParts): void {
    const names = Object.keys(data);
    checkCount(names.length, rules.min_properties?.value, rules.max_properties?.value);
    for (const required of rules.required) if (!Object.hasOwn(data, required.value)) throw new TypeError(`required field ${required.value} is absent`);
    if (additional.kind === 'forbidden' && names.some(name => !parts.some(part => part.name === name))) throw new TypeError('undeclared form/multipart field is forbidden');
}
function fieldPlan(name: string, parts: readonly PartPlan[], additional: AdditionalParts): PartPlan {
    const declared = parts.find(part => part.name === name);
    if (declared !== undefined) return declared;
    if (additional.kind === 'allowed') return additional.part;
    throw new TypeError('undeclared form/multipart field');
}
interface ValuePart { readonly name: string | undefined; readonly plan: PartPlan; readonly value: unknown; readonly styleText?: string }
function namedValues(value: unknown, rules: ObjectRules, parts: readonly PartPlan[], additional: AdditionalParts, descriptor: OperationDescriptor<unknown>, nativeExtras = true): ValuePart[] {
    const data = { ...objectData(value) };
    const extras = nativeExtras ? descriptor.objectExtras[sourceKey(rules.schema.id)] : undefined;
    if (extras !== undefined && Object.hasOwn(data, extras)) {
        const extraValues = objectData(data[extras]);
        delete data[extras];
        for (const key of Object.keys(extraValues)) {
            if (parts.some(part => part.name === key) || Object.hasOwn(data, key)) throw new TypeError('additionalFields collides with a declared field');
            Object.defineProperty(data, key, { value: extraValues[key], enumerable: true });
        }
    }
    objectRules(data, rules, parts, additional);
    const result: ValuePart[] = [];
    for (const name of Object.keys(data).sort(compareKeys)) {
        const plan = fieldPlan(name, parts, additional);
        if (plan.multiplicity === 'one') result.push({ name, plan, value: data[name] });
        else {
            const raw = data[name];
            if (!Array.isArray(raw) || raw.length === 0) throw new TypeError('repeated fields require a nonempty array; omit an optional empty field');
            checkCount(raw.length, plan.min_items?.value, plan.max_items?.value);
            if (result.length + raw.length > MAX_PARTS) throw new RangeError('multipart/form part count exceeds its finite ceiling');
            for (const item of arrayData(raw)) result.push({ name, plan, value: item });
        }
        if (result.length > MAX_PARTS) throw new RangeError('multipart/form part count exceeds its finite ceiling');
    }
    return result;
}
function valueParts(value: unknown, multipart: MultipartPlan, descriptor: OperationDescriptor<unknown>): ValuePart[] {
    if (multipart.kind === 'named') return namedValues(value, multipart.rules, multipart.parts, multipart.additional, descriptor);
    if (!Array.isArray(value)) throw new TypeError('positional multipart requires an array');
    checkCount(value.length, multipart.min_items?.value, multipart.max_items?.value);
    if (value.length > MAX_PARTS) throw new RangeError('multipart part count exceeds its finite ceiling');
    return arrayData(value).map((item, index) => {
        const plan = multipart.prefix[index] ?? (multipart.items.kind === 'allowed' ? multipart.items.part : undefined);
        if (plan === undefined) throw new TypeError('positional multipart has an undeclared extra item');
        return { name: undefined, plan, value: item };
    });
}
function partFailure(error: unknown, descriptor: OperationDescriptor<unknown>, source: ValidationSource): RuntimeError {
    if (error instanceof RuntimeError) return error;
    if (error instanceof RangeError) return new RuntimeError('resource-limit', error.message, descriptor.source, source, error);
    return new RuntimeError('request-validation', 'form/multipart structure does not satisfy its source rules', descriptor.source, source, error);
}
function partText(plan: PartPlan, name: string, value: unknown, descriptor: OperationDescriptor<unknown>, maximum: number): string {
    const representation = plan.representation;
    if (representation.kind === 'binary') throw new TypeError('binary data is not a text field');
    const wire = validatedWire(value, descriptor, representation.codec, maximum);
    if (representation.kind === 'style') return serialize(name, 'query', representation.serialization, wire);
    const text = representation.kind === 'json' ? stringifyJson(wire) : scalarText(wire, representation.scalar);
    return percentEncode(text, representation.outer_encoding);
}
export function encodeForm(value: unknown, form: FormPlan, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits, nativeExtras = true): Uint8Array {
    try {
        const pieces: string[] = [];
        const occupied = new Set<string>();
        let length = 0;
        for (const field of namedValues(value, form.rules, form.fields, form.additional, descriptor, nativeExtras)) {
            const source = field.plan.source.terminal.source;
            let text: string;
            try {
                const serialized = partText(field.plan, field.name!, field.value, descriptor, limits.part);
                text = field.plan.representation.kind === 'style' ? serialized : `${percentEncode(field.name!, 'form-url-encoded')}=${serialized}`;
            } catch (error) { throw requestFailure(error, descriptor.source, source, 'form field'); }
            limitBytes(encodeUtf8(text, limits.part, descriptor.source, source).byteLength, limits.part, descriptor.source, source, 'form field');
            for (const pair of text.split('&')) {
                const name = percentDecode(pair.split('=', 1)[0]!, true);
                // Repeated values from one declared array are legal; independently
                // named exploded properties cannot overwrite another input field.
                const owner = `${name}\u0000${field.name}`;
                if (Array.from(occupied).some(item => item.startsWith(`${name}\u0000`) && item !== owner)) throw new TypeError('form style expansions target the same wire name');
                occupied.add(owner);
            }
            length += text.length + (pieces.length === 0 ? 0 : 1);
            limitBytes(length, limits.request, descriptor.source, source, 'request body');
            pieces.push(text);
        }
        return encodeUtf8(pieces.join('&'), limits.request, descriptor.source, form.rules.schema.id);
    } catch (error) { throw partFailure(error, descriptor, form.rules.schema.id); }
}

function quoteDisposition(value: string): string {
    if (/[\u0000-\u001f\u007f]/.test(value)) throw new TypeError('multipart disposition cannot contain controls');
    return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}
function disposition(value: string): { kind: string; name?: string; filename?: string } {
    const pieces = splitQuoted(value, ';');
    const kind = pieces.shift()!.trim().toLowerCase();
    if (!token(kind) || /[\u0000-\u001f\u007f]/.test(value)) throw new TypeError('invalid multipart Content-Disposition');
    const parameters = parseParameters(pieces);
    if (Object.hasOwn(parameters, 'filename*')) throw new TypeError('extended filename conventions are not inferred for multipart/form-data');
    return { kind, ...(parameters.name === undefined ? {} : { name: parameters.name }), ...(parameters.filename === undefined ? {} : { filename: parameters.filename }) };
}
function validBoundary(boundary: string): void {
    if (boundary.length < 1 || boundary.length > 70 || boundary.endsWith(' ') || !/^[0-9A-Za-z'()+_,./:=? -]+$/.test(boundary)) throw new TypeError('multipart boundary must be 1–70 RFC 2046 boundary characters');
}
function randomBoundary(): string {
    const bytes = new Uint8Array(24);
    globalThis.crypto.getRandomValues(bytes);
    return `suspect-${Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('')}`;
}
function indexOf(bytes: Uint8Array, pattern: Uint8Array, from: number): number {
    outer: for (let i = from; i <= bytes.length - pattern.length; i++) {
        for (let j = 0; j < pattern.length; j++) if (bytes[i + j] !== pattern[j]) continue outer;
        return i;
    }
    return -1;
}
interface EncodedPart { readonly headers: Uint8Array; readonly body: Uint8Array; readonly source: ValidationSource }
function partInput(field: ValuePart): { readonly value: unknown; readonly metadata: Record<string, unknown> } {
    const wrapped = wrappedPart(field.plan) || field.plan.representation.kind === 'binary' && !isBytes(field.value);
    const metadata = wrapped ? objectData(field.value) : Object.create(null) as Record<string, unknown>;
    if (wrapped && (!Object.hasOwn(metadata, 'data') || Object.keys(metadata).some(key => !['data', 'headers', 'contentType', 'filename'].includes(key)))) throw new TypeError('multipart metadata requires data and only declared metadata members');
    return { value: wrapped ? metadata.data : field.value, metadata };
}
function physicalParts(field: ValuePart, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): ValuePart[] {
    const representation = field.plan.representation;
    if (representation.kind !== 'style') return [field];
    if (field.plan.multiplicity === 'repeated-array-items' && expandsStyle(representation.serialization)) throw new TypeError('repeated composite item expansion has no physical item-group boundary');
    const wire = validatedWire(partInput(field).value, descriptor, representation.codec, limits.request);
    const expanded = expandStyle(field.name ?? '', representation.serialization, wire);
    if (field.name === undefined && expanded.length !== 1) throw new TypeError('positional values must map to one physical part each');
    if (expanded.length > MAX_PARTS) throw new RuntimeError('resource-limit', 'multipart physical part count exceeds its finite ceiling', descriptor.source, field.plan.source.terminal.source);
    return expanded.map(part => ({ ...field, name: field.name === undefined ? undefined : part.name, styleText: part.text }));
}
function encodePart(field: ValuePart, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): EncodedPart {
    const { plan, name } = field;
    const source = plan.source.terminal.source;
    try {
        const { value, metadata } = partInput(field);
        if (metadata.filename !== undefined && typeof metadata.filename !== 'string') throw new TypeError('multipart filename must be a string');
        const typedHeaders = metadata.headers === undefined ? Object.create(null) as Record<string, unknown> : objectData(metadata.headers);
        const headerLines = new Map<string, [string, string]>();
        for (const key of Object.keys(typedHeaders)) {
            const header = plan.headers.find(header => header.name.toLowerCase() === key.toLowerCase());
            if (header === undefined || headerLines.has(key.toLowerCase())) throw new TypeError('undeclared or duplicate multipart header');
            const wire = validatedWire(typedHeaders[key], descriptor, header.codec, limits.part);
            headerLines.set(key.toLowerCase(), [header.name, serialize(header.name, 'header', header.serialization, wire)]);
        }
        for (const header of plan.headers) if (header.required && !headerLines.has(header.name.toLowerCase())) throw new TypeError(`required part header ${header.name} is absent`);
        for (const forbidden of ['content-length', 'content-transfer-encoding', 'transfer-encoding']) if (headerLines.has(forbidden)) throw new TypeError(`unsupported multipart framing header ${forbidden}`);
        if (name !== undefined) {
            const provided = headerLines.get('content-disposition');
            if (provided !== undefined) {
                const parsed = disposition(provided[1]);
                if (parsed.kind !== 'form-data' || parsed.name !== name || metadata.filename !== undefined && parsed.filename !== metadata.filename) throw new TypeError('declared Content-Disposition conflicts with the named part');
            } else {
                const filename = metadata.filename ?? (plan.representation.kind === 'binary' ? 'blob' : undefined);
                headerLines.set('content-disposition', ['Content-Disposition', `form-data; name=${quoteDisposition(name)}${filename === undefined ? '' : `; filename=${quoteDisposition(filename as string)}`}`]);
            }
        } else if (metadata.filename !== undefined) throw new TypeError('positional filenames require an explicit Content-Disposition header');
        let contentType = metadata.contentType;
        if (contentType !== undefined && typeof contentType !== 'string') throw new TypeError('part Content-Type must be a string');
        if (plan.content_types.length !== 0) {
            if (contentType === undefined) {
                if (plan.content_types.length !== 1 || plan.content_types[0]!.range.kind !== 'concrete') throw new TypeError('this part requires an explicit concrete contentType choice');
                contentType = canonicalMedia(plan.content_types[0]!);
            }
            const actual = parseMedia(contentType as string);
            if (!plan.content_types.some(media => matchesMedia(media, actual))) throw new TypeError('part Content-Type does not match its declaration');
            if (plan.representation.kind === 'text' && actual.parameters.charset !== undefined && actual.parameters.charset.toLowerCase() !== 'utf-8') throw new TypeError('text part requires UTF-8');
            headerLines.set('content-type', ['Content-Type', contentType as string]);
        } else if (contentType !== undefined) throw new TypeError('style-encoded part ignores contentType; do not supply a media override');
        let body: Uint8Array;
        if (plan.representation.kind === 'binary') {
            if (!isBytes(value)) throw new TypeError('binary parts require in-memory Uint8Array bytes');
            limitBytes(byteLength(value), Math.min(limits.part, plan.representation.bytes.max_bytes), descriptor.source, source, 'binary part');
            body = copyBytes(value);
        } else body = encodeUtf8(field.styleText ?? partText(plan, name ?? '', value, descriptor, limits.part), limits.part, descriptor.source, source);
        if (headerLines.size > MAX_PART_HEADERS) throw new RangeError('too many part headers');
        const headers = Array.from(headerLines.values()).map(([key, text]) => {
            if (!token(key) || /[\u0000-\u0008\u000a-\u001f\u007f]/.test(text)) throw new TypeError('invalid multipart header framing');
            return `${key}: ${text}\r\n`;
        }).join('') + '\r\n';
        return { body, headers: encodeUtf8(headers, MAX_PART_HEADER_BYTES, descriptor.source, source), source };
    } catch (error) { throw requestFailure(error, descriptor.source, source, 'multipart part'); }
}
export function encodeMultipart(value: unknown, multipart: MultipartPlan, contentType: string, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): { bytes: Uint8Array; contentType: string } {
    const source = multipart.kind === 'named' ? multipart.rules.schema.id : multipart.schema.id;
    let fields: ValuePart[];
    try { fields = valueParts(value, multipart, descriptor); }
    catch (error) { throw partFailure(error, descriptor, source); }
    try {
        const media = parseMedia(contentType);
        const boundary = media.parameters.boundary ?? randomBoundary();
        validBoundary(boundary);
        const needle = encodeUtf8(boundary, 70, descriptor.source, source);
        const delimiter = encodeUtf8(`--${boundary}\r\n`, 74, descriptor.source, source);
        const end = encodeUtf8(`--${boundary}--\r\n`, 76, descriptor.source, source);
        const parts: EncodedPart[] = [];
        const owners = new Map<string, string>();
        let length = end.byteLength;
        limitBytes(length, limits.request, descriptor.source, source, 'multipart framing');
        for (const field of fields) {
            for (const physical of physicalParts(field, descriptor, limits)) {
                if (parts.length >= MAX_PARTS) throw new RuntimeError('resource-limit', 'multipart physical part count exceeds its ceiling', descriptor.source, field.plan.source.terminal.source);
                if (physical.name !== undefined) {
                    const previous = owners.get(physical.name);
                    if (previous !== undefined && previous !== field.name) throw new TypeError('multipart style expansions collide with another logical field');
                    owners.set(physical.name, field.name!);
                }
                const part = encodePart(physical, descriptor, { ...limits, part: Math.min(limits.part, Math.max(0, limits.request - length - delimiter.byteLength - 2)) });
                length += delimiter.byteLength + part.headers.byteLength + part.body.byteLength + 2;
                limitBytes(length, limits.request, descriptor.source, part.source, 'multipart request');
                if (indexOf(part.body, needle, 0) >= 0 || indexOf(part.headers, needle, 0) >= 0) throw new TypeError('multipart boundary collides with part data or headers');
                parts.push(part);
            }
        }
        const bytes = new Uint8Array(length);
        let offset = 0;
        for (const part of parts) {
            for (const chunk of [delimiter, part.headers, part.body]) { bytes.set(chunk, offset); offset += chunk.byteLength; }
            bytes.set([13, 10], offset); offset += 2;
        }
        bytes.set(end, offset);
        return { bytes, contentType: media.parameters.boundary === undefined ? `${contentType};boundary=${boundary}` : contentType };
    } catch (error) { throw requestFailure(error, descriptor.source, source, 'multipart framing'); }
}

interface FormPair { readonly name: string; readonly rawName: string; readonly rawValue: string }
function belongs(pair: FormPair, name: string, serialization?: Serialization): boolean {
    if (serialization?.kind !== 'style') return pair.name === name;
    if (serialization.style === 'deepObject') return pair.name.startsWith(`${name}[`) && pair.name.endsWith(']');
    if (serialization.style === 'form' && serialization.explode && serialization.shape.kind === 'flat-object') {
        return Object.hasOwn(serialization.shape.properties, pair.name) || serialization.shape.additional.kind !== 'forbidden';
    }
    return pair.name === name;
}
function objectWire(pairs: readonly [string, string][], shape: Extract<WireShape, { kind: 'flat-object' }>): WireJsonValue {
    const result: { [key: string]: WireJsonValue } = Object.create(null);
    for (const [key, text] of pairs) {
        if (Object.hasOwn(result, key)) throw new TypeError('duplicate object property in form encoding');
        Object.defineProperty(result, key, { value: scalarWire(text, propertyScalar(shape, key)), enumerable: true });
    }
    return result;
}
function styleWire(name: string, pairs: readonly FormPair[], serialization: Serialization): WireJsonValue {
    if (serialization.kind !== 'style') throw new TypeError('style descriptor required');
    const { shape, style, explode } = serialization;
    const decode = (value: string) => percentDecode(value, serialization.percent_encoding === 'form-url-encoded');
    if (style === 'deepObject' && shape.kind === 'flat-object') return objectWire(pairs.map(pair => [pair.name.slice(name.length + 1, -1), decode(pair.rawValue)]), shape);
    if (style === 'form' && explode && shape.kind === 'flat-object') return objectWire(pairs.map(pair => [pair.name, decode(pair.rawValue)]), shape);
    if (style === 'form' && explode && shape.kind === 'array') return decodeComposite(pairs.map(pair => decode(pair.rawValue)), shape, false);
    if (pairs.length !== 1) throw new TypeError('non-repeated form field occurred more than once');
    if (shape.kind === 'scalar') return scalarWire(decode(pairs[0]!.rawValue), shape.scalar);
    const delimiter = style === 'spaceDelimited' ? /%20/i : style === 'pipeDelimited' ? /%7c/i : /,/;
    return decodeComposite(pairs[0]!.rawValue.split(delimiter).map(decode), shape, explode);
}
function decodeTextField(plan: PartPlan, name: string, pairs: readonly FormPair[], descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    const representation = plan.representation;
    if (representation.kind === 'binary') throw new TypeError('binary form fields are not supported');
    let text: string;
    if (representation.kind === 'style') {
        const wire = styleWire(name, pairs, representation.serialization);
        serialize(name, 'query', representation.serialization, wire);
        text = stringifyJson(wire);
    } else {
        if (pairs.length !== 1) throw new TypeError('scalar form field occurred more than once');
        text = percentDecode(pairs[0]!.rawValue, representation.outer_encoding === 'form-url-encoded');
        limitBytes(encodeUtf8(text, limits.part, descriptor.source, plan.source.terminal.source).byteLength, limits.part, descriptor.source, plan.source.terminal.source, 'form field');
        if (representation.kind === 'text') text = stringifyJson(scalarWire(text, representation.scalar));
    }
    return decodeJson(text, descriptor, representation.codec, plan.source.terminal.source);
}
export function decodeForm(bytes: Uint8Array, form: FormPlan, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    const source = form.rules.schema.id;
    try {
        const text = decodeUtf8(bytes);
        const pairs = text === '' ? [] : text.split('&').map(pair => {
            const equals = pair.indexOf('=');
            const rawName = equals < 0 ? pair : pair.slice(0, equals);
            return { name: percentDecode(rawName, true), rawName, rawValue: equals < 0 ? '' : pair.slice(equals + 1) };
        });
        if (pairs.length > MAX_PARTS) throw new RuntimeError('resource-limit', 'form field count exceeds its ceiling', descriptor.source, source);
        const groups = new Map<string, FormPair[]>();
        for (const pair of pairs) {
            const candidates = form.fields.filter(field => belongs(pair, field.name!, field.representation.kind === 'style' ? field.representation.serialization : undefined));
            if (candidates.length > 1) throw new TypeError('form expansions ambiguously target more than one source field');
            const name = candidates[0]?.name ?? pair.name;
            if (candidates.length === 0 && form.additional.kind === 'forbidden') throw new TypeError('undeclared form field');
            const group = groups.get(name) ?? [];
            group.push(pair); groups.set(name, group);
        }
        const result: Record<string, unknown> = Object.create(null);
        for (const [name, pairs] of groups) {
            const plan = fieldPlan(name, form.fields, form.additional);
            const value = plan.multiplicity === 'one' ? decodeTextField(plan, name, pairs, descriptor, limits) : pairs.map(pair => decodeTextField(plan, name, [pair], descriptor, limits));
            if (plan.multiplicity !== 'one') checkCount(pairs.length, plan.min_items?.value, plan.max_items?.value);
            Object.defineProperty(result, name, { value, enumerable: true });
        }
        objectRules(result, form.rules, form.fields, form.additional);
        return packExtras(result, form.rules, form.fields, descriptor);
    } catch (error) { throw responseFailure(error, descriptor.source, source); }
}

interface Delimiter { readonly start: number; readonly next: number; readonly end: boolean }
function delimiterAt(bytes: Uint8Array, boundary: Uint8Array, from: number): Delimiter | undefined {
    let offset = from;
    for (;;) {
        const start = indexOf(bytes, boundary, offset);
        if (start < 0) return undefined;
        offset = start + 1;
        if (start !== 0 && (bytes[start - 2] !== 13 || bytes[start - 1] !== 10)) continue;
        let next = start + boundary.length;
        const end = bytes[next] === 45 && bytes[next + 1] === 45;
        if (end) next += 2;
        while (bytes[next] === 32 || bytes[next] === 9) next++;
        if (bytes[next] === 13 && bytes[next + 1] === 10) return { start, next: next + 2, end };
        if (end && next === bytes.length) return { start, next, end };
    }
}
function decodePart(plan: PartPlan, headers: HeaderReader, bytes: Uint8Array, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    const source = plan.source.terminal.source;
    try {
        limitBytes(bytes.byteLength, Math.min(limits.part, plan.representation.kind === 'binary' ? plan.representation.bytes.max_bytes : limits.part), descriptor.source, source, 'response part');
        const typedHeaders = decodeHeaders(headers, plan.headers, descriptor);
        let contentType: string | undefined;
        if (plan.content_types.length !== 0) {
            const actual = parseMedia(headers.get('content-type') ?? 'text/plain');
            const declared = plan.content_types.find(media => matchesMedia(media, actual));
            if (declared === undefined) throw new TypeError('part media does not match its source declaration');
            contentType = canonicalMedia(declared, actual);
            if (plan.representation.kind === 'text' && actual.parameters.charset !== undefined && actual.parameters.charset.toLowerCase() !== 'utf-8') throw new TypeError('text part must be UTF-8');
        }
        let data: unknown;
        const representation = plan.representation;
        if (representation.kind === 'binary') data = bytes.slice();
        else {
            let text = decodeUtf8(bytes);
            if (representation.kind === 'text') text = stringifyJson(scalarWire(text, representation.scalar));
            else if (representation.kind === 'style') text = stringifyJson(parseStyle(plan.name ?? '', representation.serialization, [{ name: plan.name ?? '', text }]));
            data = decodeJson(text, descriptor, representation.codec, source);
        }
        if (wrappedPart(plan) || representation.kind === 'binary') {
            const header = headers.get('content-disposition');
            const filename = header === null ? undefined : disposition(header).filename;
            return { data, headers: typedHeaders, ...(contentType === undefined ? {} : { contentType }), ...(filename === undefined ? {} : { filename }) };
        }
        return data;
    } catch (error) { throw responseFailure(error, descriptor.source, source); }
}
interface PhysicalPart { readonly name: string; readonly headers: HeaderReader; readonly bytes: Uint8Array }
function styledValue(name: string, plan: PartPlan, parts: readonly PhysicalPart[], descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    if (plan.representation.kind !== 'style') throw new TypeError('style part descriptor required');
    const source = plan.source.terminal.source;
    try {
        const fields = parts.map(part => {
            limitBytes(part.bytes.byteLength, limits.part, descriptor.source, source, 'response part');
            return { name: part.name, text: decodeUtf8(part.bytes) };
        });
        const wire = parseStyle(name, plan.representation.serialization, fields);
        expandStyle(name, plan.representation.serialization, wire);
        const data = decodeJson(stringifyJson(wire), descriptor, plan.representation.codec, source);
        const metadata = parts.map(part => {
            const headers = decodeHeaders(part.headers, plan.headers, descriptor);
            const declared = part.headers.get('content-disposition');
            const filename = declared === null ? undefined : disposition(declared).filename;
            return { headers, ...(filename === undefined ? {} : { filename }) };
        });
        if (wrappedPart(plan)) {
            const first = metadata[0]!;
            if (metadata.some(value => stringifyJson(value) !== stringifyJson(first))) throw new TypeError('expanded parts have inconsistent metadata for one logical value');
            return { data, ...first };
        }
        return data;
    } catch (error) { throw responseFailure(error, descriptor.source, source); }
}
function namedPart(name: string, multipart: Extract<MultipartPlan, { kind: 'named' }>): { name: string; plan: PartPlan } {
    const candidates = multipart.parts.filter(part => part.representation.kind === 'style' ? ownsStyleField(part.name!, part.representation.serialization, name) : part.name === name);
    if (candidates.length > 1) throw new TypeError('physical multipart field ambiguously targets multiple logical fields');
    const plan = candidates[0];
    if (plan !== undefined) return { name: plan.name!, plan };
    if (multipart.additional.kind === 'allowed') return { name, plan: multipart.additional.part };
    throw new TypeError('undeclared multipart field');
}
export function decodeMultipart(bytes: Uint8Array, multipart: MultipartPlan, contentType: string, descriptor: OperationDescriptor<unknown>, limits: RuntimeLimits): unknown {
    const source = multipart.kind === 'named' ? multipart.rules.schema.id : multipart.schema.id;
    try {
        const boundary = parseMedia(contentType).parameters.boundary;
        if (boundary === undefined) throw new TypeError('multipart response needs a boundary');
        validBoundary(boundary);
        const marker = new TextEncoder().encode(`--${boundary}`);
        const headerEnd = new Uint8Array([13, 10, 13, 10]);
        let current = delimiterAt(bytes, marker, 0);
        if (current === undefined) throw new TypeError('multipart opening boundary is absent');
        const result: Record<string, unknown> = Object.create(null);
        const groups = new Map<string, { readonly plan: PartPlan; readonly parts: PhysicalPart[] }>();
        const positional: unknown[] = [];
        let count = 0;
        while (!current.end) {
            const next = delimiterAt(bytes, marker, current.next);
            if (next === undefined) throw new TypeError('multipart closing boundary is absent');
            if (++count > MAX_PARTS) throw new RuntimeError('resource-limit', 'multipart part count exceeds its ceiling', descriptor.source, source);
            const end = next.start - 2;
            const emptyHeaders = bytes[current.next] === 13 && bytes[current.next + 1] === 10;
            const split = emptyHeaders ? current.next : indexOf(bytes, headerEnd, current.next);
            const bodyStart = split + (emptyHeaders ? 2 : 4);
            if (split < current.next || bodyStart > end || split - current.next > MAX_PART_HEADER_BYTES) throw new TypeError('multipart part header framing is invalid or exceeds its ceiling');
            const headerText = decodeUtf8(bytes.subarray(current.next, split));
            const headerValues = new Map<string, string>();
            const headers: HeaderReader = { get(name) { return headerValues.get(name.toLowerCase()) ?? null; } };
            const lines = headerText === '' ? [] : headerText.split('\r\n');
            if (lines.length > MAX_PART_HEADERS) throw new TypeError('multipart part has too many headers');
            for (const line of lines) {
                const colon = line.indexOf(':');
                const name = line.slice(0, colon);
                if (colon < 1 || !token(name) || headerValues.has(name.toLowerCase()) || /[\u0000-\u0008\u000a-\u001f\u007f]/.test(line)) throw new TypeError('invalid, folded or duplicate multipart header');
                if (['content-length', 'content-transfer-encoding', 'transfer-encoding'].includes(name.toLowerCase())) throw new TypeError('unsupported multipart transfer/framing header');
                headerValues.set(name.toLowerCase(), line.slice(colon + 1).trim());
            }
            const body = bytes.subarray(bodyStart, end);
            if (multipart.kind === 'named') {
                const header = headers.get('content-disposition');
                if (header === null) throw new TypeError('named part requires Content-Disposition');
                const parsed = disposition(header);
                if (parsed.kind !== 'form-data' || parsed.name === undefined) throw new TypeError('named part requires form-data with a name');
                const { name, plan } = namedPart(parsed.name, multipart);
                const group = groups.get(name) ?? { plan, parts: [] };
                group.parts.push({ name: parsed.name, headers, bytes: body });
                groups.set(name, group);
            } else {
                const plan = multipart.prefix[positional.length] ?? (multipart.items.kind === 'allowed' ? multipart.items.part : undefined);
                if (plan === undefined) throw new TypeError('positional multipart contains an undeclared extra item');
                positional.push(decodePart(plan, headers, body, descriptor, limits));
            }
            current = next;
        }
        if (multipart.kind === 'named') {
            for (const [name, {plan, parts}] of groups) {
                let value: unknown;
                if (plan.multiplicity === 'repeated-array-items') {
                    checkCount(parts.length, plan.min_items?.value, plan.max_items?.value);
                    if (plan.representation.kind === 'style' && expandsStyle(plan.representation.serialization)) throw new TypeError('repeated composite expansion has no physical item-group boundary');
                    value = parts.map(part => plan.representation.kind === 'style' ? styledValue(name, plan, [part], descriptor, limits) : decodePart(plan, part.headers, part.bytes, descriptor, limits));
                } else if (plan.representation.kind === 'style') value = styledValue(name, plan, parts, descriptor, limits);
                else {
                    if (parts.length !== 1) throw new TypeError('non-repeated multipart field occurred more than once');
                    value = decodePart(plan, parts[0]!.headers, parts[0]!.bytes, descriptor, limits);
                }
                Object.defineProperty(result, name, { value, enumerable: true });
            }
            objectRules(result, multipart.rules, multipart.parts, multipart.additional);
            return packExtras(result, multipart.rules, multipart.parts, descriptor);
        }
        checkCount(positional.length, multipart.min_items?.value, multipart.max_items?.value);
        return positional;
    } catch (error) { throw responseFailure(error, descriptor.source, source); }
}

function packExtras(data: Record<string, unknown>, rules: ObjectRules, parts: readonly PartPlan[], descriptor: OperationDescriptor<unknown>): Record<string, unknown> {
    const member = descriptor.objectExtras[sourceKey(rules.schema.id)];
    if (member === undefined) return data;
    const result: Record<string, unknown> = Object.create(null), extras: Record<string, unknown> = Object.create(null);
    for (const [key, value] of Object.entries(data)) Object.defineProperty(parts.some(part => part.name === key) ? result : extras, key, { value, enumerable: true });
    if (Object.keys(extras).length !== 0) Object.defineProperty(result, member, { value: extras, enumerable: true });
    return result;
}
