import { JsonNumber, type WireJsonValue } from '../json.js';
import type { Serialization, WireShape } from './types.js';
import { compareKeys, decodeComposite, propertyScalar, scalarText, scalarWire } from './wire.js';

/** Physical MIME fields after RFC6570 name/value expansion, without URI encoding. */
export interface StyleField { readonly name: string; readonly text: string }

export function expandsStyle(serialization: Serialization): boolean {
    if (serialization.kind !== 'style') return false;
    return serialization.style === 'deepObject' || serialization.style === 'form' && serialization.explode && serialization.shape.kind !== 'scalar';
}
function assertStyle(serialization: Serialization): asserts serialization is Extract<Serialization, { kind: 'style' }> {
    if (serialization.kind !== 'style' || serialization.percent_encoding !== 'none') throw new TypeError('multipart styles require the unencoded source strategy');
}
function objectValues(value: WireJsonValue, shape: Extract<WireShape, { kind: 'flat-object' }>): [string, string][] {
    if (value === null || typeof value !== 'object' || Array.isArray(value) || JsonNumber.is(value)) throw new TypeError('multipart style requires a flat object');
    const result: [string, string][] = [];
    for (const key of Object.keys(value).sort(compareKeys)) {
        const kind = Object.hasOwn(shape.properties, key) ? shape.properties[key]! : shape.additional.kind === 'typed' ? shape.additional.scalar : undefined;
        if (!Object.hasOwn(shape.properties, key) && shape.additional.kind === 'forbidden') throw new TypeError('undeclared multipart style property');
        result.push([key, scalarText(value[key]!, kind)]);
    }
    return result;
}
function joined(values: readonly string[], delimiter: string): string {
    if (values.some(value => value.includes(delimiter))) throw new TypeError('multipart value contains an active unescaped style delimiter');
    return values.join(delimiter);
}

/** Codec validation precedes this expansion. No payload/name is split as an encoded query string. */
export function expandStyle(name: string, serialization: Serialization, value: WireJsonValue): StyleField[] {
    assertStyle(serialization);
    const { style, shape, explode } = serialization;
    if (shape.kind === 'scalar') {
        if (style !== 'form') throw new TypeError('multipart scalar requires form style');
        return [{ name, text: scalarText(value, shape.scalar) }];
    }
    if (shape.kind === 'array') {
        if (!Array.isArray(value) || value.length === 0) throw new TypeError('empty multipart style arrays have no physical fields');
        const values = value.map(item => scalarText(item, shape.items));
        if (style === 'form') return explode ? values.map(text => ({ name, text })) : [{ name, text: joined(values, ',') }];
        if (style === 'spaceDelimited' || style === 'pipeDelimited') return [{ name, text: joined(values, style === 'spaceDelimited' ? ' ' : '|') }];
        throw new TypeError('unsupported multipart array style');
    }
    const values = objectValues(value, shape);
    if (values.length === 0) throw new TypeError('empty multipart style objects have no physical fields');
    if (style === 'deepObject') {
        if (values.some(([key]) => /[\[\]]/.test(key))) throw new TypeError('deepObject property names contain an active bracket delimiter');
        return values.map(([key, text]) => ({ name: `${name}[${key}]`, text }));
    }
    if (style === 'form') return explode ? values.map(([name, text]) => ({ name, text })) : [{ name, text: joined(values.flat(), ',') }];
    if (style === 'spaceDelimited' || style === 'pipeDelimited') return [{ name, text: joined(values.flat(), style === 'spaceDelimited' ? ' ' : '|') }];
    throw new TypeError('unsupported multipart object style');
}

export function ownsStyleField(logicalName: string, serialization: Serialization, physicalName: string): boolean {
    assertStyle(serialization);
    const { style, shape, explode } = serialization;
    if (style === 'deepObject') {
        if (!physicalName.startsWith(`${logicalName}[`) || !physicalName.endsWith(']')) return false;
        return !/[\[\]]/.test(physicalName.slice(logicalName.length + 1, -1));
    }
    if (style === 'form' && explode && shape.kind === 'flat-object') {
        return Object.hasOwn(shape.properties, physicalName) || shape.additional.kind !== 'forbidden';
    }
    return physicalName === logicalName;
}

/** Inverse physical mapping. Text remains text unless the source provides its scalar type. */
export function parseStyle(name: string, serialization: Serialization, fields: readonly StyleField[]): WireJsonValue {
    assertStyle(serialization);
    if (fields.length === 0) throw new TypeError('a multipart style value has no physical fields');
    const { style, shape, explode } = serialization;
    if (shape.kind === 'flat-object' && (style === 'deepObject' || style === 'form' && explode)) {
        const result: { [key: string]: WireJsonValue } = Object.create(null);
        for (const field of fields) {
            if (!ownsStyleField(name, serialization, field.name)) throw new TypeError('multipart field does not belong to its source object');
            const key = style === 'deepObject' ? field.name.slice(name.length + 1, -1) : field.name;
            if (Object.hasOwn(result, key)) throw new TypeError('duplicate exploded multipart property');
            Object.defineProperty(result, key, { value: scalarWire(field.text, propertyScalar(shape, key)), enumerable: true });
        }
        return result;
    }
    if (fields.some(field => field.name !== name)) throw new TypeError('multipart field name does not match the source value');
    if (shape.kind === 'array' && style === 'form' && explode) return fields.map(field => scalarWire(field.text, shape.items));
    if (fields.length !== 1) throw new TypeError('non-expanded multipart value occurred more than once');
    if (shape.kind === 'scalar') return scalarWire(fields[0]!.text, shape.scalar);
    const delimiter = style === 'spaceDelimited' ? ' ' : style === 'pipeDelimited' ? '|' : ',';
    return decodeComposite(fields[0]!.text.split(delimiter), shape, false);
}
