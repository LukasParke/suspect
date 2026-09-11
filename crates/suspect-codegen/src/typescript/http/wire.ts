import { JsonNumber, parseJson, stringifyJson, type WireJsonValue } from '../json.js';
import { isIntegralJsonNumber } from '../validation.js';
import type { ValidationSource } from '../validation.js';
import { decodeJson, responseFailure, utf8Length } from './common.js';
import type { HeaderPlan, Location, OperationDescriptor, PercentEncoding, Scalar, Serialization, Style, WireShape } from './types.js';

export const token = (text: string): boolean => /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(text);

export function scalarText(value: WireJsonValue, kind?: Scalar): string {
    if (typeof value === 'string' && (kind === undefined || kind === 'string')) return value;
    if (typeof value === 'boolean' && (kind === undefined || kind === 'boolean')) return value ? 'true' : 'false';
    if (JsonNumber.is(value) && (kind === undefined || kind === 'number' || kind === 'integer' && isIntegralJsonNumber(value))) return JsonNumber.prototype.toString.call(value);
    throw new TypeError('value does not have its declared non-null scalar wire shape');
}
export function scalarWire(text: string, kind: Scalar): WireJsonValue {
    if (kind === 'string') return text;
    if (kind === 'boolean') {
        if (text === 'true') return true;
        if (text === 'false') return false;
        throw new TypeError('boolean text must be true or false');
    }
    const value = JsonNumber.parse(text);
    if (kind === 'integer' && !isIntegralJsonNumber(value)) throw new TypeError('integer text must be mathematically integral');
    return value;
}
/** Unicode scalar order matches the protocol reference interpreter, including astral keys. */
export function compareKeys(a: string, b: string): number {
    const left = Array.from(a, char => char.codePointAt(0)!);
    const right = Array.from(b, char => char.codePointAt(0)!);
    for (let i = 0; i < Math.min(left.length, right.length); i++) if (left[i] !== right[i]) return left[i]! - right[i]!;
    return left.length - right.length;
}
function canonicalJson(value: WireJsonValue): string {
    if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
    if (value !== null && typeof value === 'object' && !JsonNumber.is(value)) {
        return `{${Object.keys(value).sort(compareKeys).map(key => `${stringifyJson(key)}:${canonicalJson(value[key]!)}`).join(',')}}`;
    }
    return stringifyJson(value);
}
export function percentEncode(text: string, encoding: PercentEncoding): string {
    utf8Length(text);
    if (encoding === 'none') return text;
    const bytes = new TextEncoder().encode(text);
    let result = '';
    for (let i = 0; i < bytes.length; i++) {
        const char = String.fromCharCode(bytes[i]!);
        if (encoding === 'reserved-expansion' && char === '%' && /^[0-9A-Fa-f]{2}$/.test(String.fromCharCode(bytes[i + 1] ?? 0, bytes[i + 2] ?? 0))) {
            result += `%${String.fromCharCode(bytes[++i]!)}${String.fromCharCode(bytes[++i]!)}`;
        } else if (/^[A-Za-z0-9]$/.test(char) || (encoding === 'form-url-encoded' ? '*-._' : '-._~').includes(char) ||
            encoding === 'reserved-expansion' && ":/?#[]@!$&'()*+,;=".includes(char)) result += char;
        else if (encoding === 'form-url-encoded' && char === ' ') result += '+';
        else result += `%${bytes[i]!.toString(16).toUpperCase().padStart(2, '0')}`;
    }
    return result;
}
export function percentDecode(text: string, form: boolean): string {
    // decodeURIComponent refuses invalid escapes and invalid UTF-8; it never repairs wire bytes.
    return decodeURIComponent(form ? text.replace(/\+/g, ' ') : text);
}
function encodeValue(text: string, encoding: PercentEncoding, location: Location, style?: Style, shape?: WireShape, key = false): string {
    if (encoding === 'none' && /[\u0000-\u0008\u000a-\u001f\u007f-\u009f]/.test(text) || encoding === 'none' && location !== 'header' && text.includes('\t')) throw new TypeError('control characters cannot be injected into a header, cookie or part');
    if (encoding === 'none' && location === 'cookie' && /[\s",;\\\u007f-\uffff]/.test(text)) throw new TypeError('cookie values must be caller-escaped for cookie syntax');
    if (style === 'spaceDelimited' && text.includes(' ') || style === 'pipeDelimited' && text.includes('|') || style === 'deepObject' && /[\[\]]/.test(text)) throw new TypeError('data contains an active percent-encoded style delimiter; an API-defined escape is required');
    const composite = shape !== undefined && shape.kind !== 'scalar';
    if (encoding === 'reserved-expansion') {
        const hazard = location === 'path' ? /[#[\]/?]/ : location === 'query' ? /[#[\]&=+]/ : location === 'cookie' ? /[;,]/ : /$^/;
        const delimiter = style === 'label' ? /[.,]/ : style === 'matrix' ? /[;,]/ : /,/;
        if (hazard.test(text) || composite && delimiter.test(text)) throw new TypeError('allowReserved requires caller-preescaped URI hazards and active data delimiters');
    }
    if (composite && encoding === 'none' && (text.includes(',') || key && text.includes('='))) throw new TypeError('unquoted composite value contains an active delimiter');
    // RFC3986 normally leaves a dot unescaped; it cannot also be label data when
    // it is the active exploded delimiter. Refuse rather than split one item.
    if (composite && style === 'label' && text.includes('.')) throw new TypeError('label composite data contains its active delimiter');
    return percentEncode(text, encoding);
}

export function serialize(name: string, location: Location, serialization: Serialization, value: WireJsonValue): string {
    if (serialization.kind === 'content') {
        const media = serialization.media_type.range;
        const json = media.kind === 'concrete' && (media.type_name === 'application' && media.subtype === 'json' || media.subtype.endsWith('+json'));
        const text = encodeValue(json ? canonicalJson(value) : scalarText(value), serialization.percent_encoding, location);
        return location === 'query' || location === 'cookie' ? `${percentEncode(name, 'uri-component')}=${text}` : text;
    }
    const { style, explode, shape, percent_encoding: encoding } = serialization;
    name = percentEncode(name, location === 'header' || style === 'cookie' ? 'none' : 'uri-component');
    const encode = (text: string, key = false) => encodeValue(text, encoding, location, style, shape, key);
    let scalar: string | undefined;
    let items: string[] = [];
    const properties: [string, string][] = [];
    if (shape.kind === 'scalar') scalar = encode(scalarText(value, shape.scalar));
    else if (shape.kind === 'array') {
        if (!Array.isArray(value) || value.length === 0) throw new TypeError('empty or non-array composite has no parameter expansion');
        items = value.map(item => encode(scalarText(item, shape.items)));
    } else {
        if (value === null || typeof value !== 'object' || Array.isArray(value) || JsonNumber.is(value) || Object.keys(value).length === 0) throw new TypeError('empty or non-object composite has no parameter expansion');
        for (const key of Object.keys(value).sort(compareKeys)) {
            const kind = Object.hasOwn(shape.properties, key) ? shape.properties[key]! : shape.additional.kind === 'typed' ? shape.additional.scalar : undefined;
            if (!Object.hasOwn(shape.properties, key) && shape.additional.kind === 'forbidden') throw new TypeError('undeclared style property');
            properties.push([encode(key, true), encode(scalarText(value[key]!, kind))]);
        }
    }
    const flatten = (delimiter: string) => properties.flat().join(delimiter);
    const pairs = (delimiter: string) => properties.map(([key, item]) => `${key}=${item}`).join(delimiter);
    const named = (key: string, item: string) => item === '' ? key : `${key}=${item}`;
    switch (style) {
        case 'simple': return scalar ?? (shape.kind === 'array' ? items.join(',') : explode ? pairs(',') : flatten(','));
        case 'label': return `.${scalar ?? (shape.kind === 'array' ? items.join(explode ? '.' : ',') : explode ? pairs('.') : flatten(','))}`;
        case 'matrix': return scalar !== undefined ? `;${named(name, scalar)}` : shape.kind === 'array' ?
            explode ? items.map(item => `;${named(name, item)}`).join('') : `;${name}=${items.join(',')}` :
            explode ? properties.map(([key, item]) => `;${named(key, item)}`).join('') : `;${name}=${flatten(',')}`;
        case 'form': case 'cookie': {
            const delimiter = style === 'cookie' ? '; ' : '&';
            return scalar !== undefined ? `${name}=${scalar}` : shape.kind === 'array' ?
                explode ? items.map(item => `${name}=${item}`).join(delimiter) : `${name}=${items.join(',')}` :
                explode ? pairs(delimiter) : `${name}=${flatten(',')}`;
        }
        case 'spaceDelimited': case 'pipeDelimited': {
            const delimiter = style === 'spaceDelimited' ? '%20' : '%7C';
            return `${name}=${shape.kind === 'array' ? items.join(delimiter) : flatten(delimiter)}`;
        }
        case 'deepObject': return properties.map(([key, item]) => `${name}%5B${key}%5D=${item}`).join('&');
    }
}

export function propertyScalar(shape: Extract<WireShape, { kind: 'flat-object' }>, key: string): Scalar {
    if (Object.hasOwn(shape.properties, key)) return shape.properties[key]!;
    if (shape.additional.kind === 'typed') return shape.additional.scalar;
    if (shape.additional.kind === 'forbidden') throw new TypeError('undeclared header/form property');
    return 'string'; // An unconstrained text field stays text; no number/bool sniffing.
}
export function decodeComposite(values: readonly string[], shape: WireShape, explode: boolean): WireJsonValue {
    if (shape.kind === 'scalar') {
        if (values.length !== 1) throw new TypeError('scalar field occurred more than once');
        return scalarWire(values[0]!, shape.scalar);
    }
    if (shape.kind === 'array') return values.map(value => scalarWire(value, shape.items));
    const result: { [key: string]: WireJsonValue } = Object.create(null);
    const properties: [string, string][] = [];
    if (explode) {
        for (const value of values) {
            const equals = value.indexOf('=');
            if (equals < 0) throw new TypeError('exploded object fields require key=value');
            properties.push([value.slice(0, equals), value.slice(equals + 1)]);
        }
    } else {
        if (values.length % 2 !== 0) throw new TypeError('object field needs alternating keys and values');
        for (let i = 0; i < values.length; i += 2) properties.push([values[i]!, values[i + 1]!]);
    }
    for (const [key, value] of properties) {
        if (Object.hasOwn(result, key)) throw new TypeError('duplicate object property in a header/form field');
        Object.defineProperty(result, key, { value: scalarWire(value, propertyScalar(shape, key)), enumerable: true });
    }
    return result;
}
export function headerWire(text: string, plan: HeaderPlan, descriptor: OperationDescriptor<unknown>): WireJsonValue {
    const serialization = plan.serialization;
    if (serialization.kind === 'content') {
        if (plan.codec.input === 'json') return parseJson(text);
        const representation = plan.content_media?.representation;
        if (representation?.kind !== 'text') throw new TypeError('text header is missing its source scalar representation');
        return scalarWire(text, representation.scalar);
    }
    // Simple headers carry literal strings, not URI components or quoted CSV.
    const { shape, explode } = serialization;
    if (shape.kind === 'scalar') return scalarWire(text, shape.scalar);
    return decodeComposite(text.split(','), shape, explode);
}
/** Header access shared by Fetch headers and UTF-8 MIME part headers. */
export interface HeaderReader { get(name: string): string | null; getSetCookie?(): string[] }
export function decodeHeaders(headers: HeaderReader, plans: readonly HeaderPlan[], descriptor: OperationDescriptor<unknown>): Record<string, unknown> {
    const result: Record<string, unknown> = Object.create(null);
    for (const plan of plans) {
        try {
            let text = headers.get(plan.name);
            if (plan.name.toLowerCase() === 'set-cookie') {
                const cookies = headers.getSetCookie?.();
                if (cookies !== undefined) {
                    if (cookies.length > 1) throw new TypeError('the declared scalar Set-Cookie cannot represent multiple fields');
                    text = cookies[0] ?? null;
                }
            }
            if (text === null) {
                if (plan.required) throw new TypeError(`required response header ${plan.name} is absent or unavailable to Fetch`);
                continue;
            }
            const wire = headerWire(text, plan, descriptor);
            // Enforce wire injection/shape guards as well as the source codec.
            serialize(plan.name, 'header', plan.serialization, wire);
            const value = decodeJson(stringifyJson(wire), descriptor, plan.codec, plan.source.terminal.source);
            Object.defineProperty(result, plan.name, { value, enumerable: true });
        } catch (error) { throw responseFailure(error, descriptor.source, plan.source.terminal.source); }
    }
    return Object.freeze(result);
}
