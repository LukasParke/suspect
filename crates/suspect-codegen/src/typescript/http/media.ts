import type { MediaPlan, MediaType, ProtocolOperation, ResponsePlan } from './types.js';
import { token } from './wire.js';

export interface ParsedMedia { readonly type: string; readonly subtype: string; readonly parameters: Readonly<Record<string, string>> }
export function splitQuoted(value: string, delimiter: string): string[] {
    const result: string[] = [];
    let start = 0, quoted = false, escape = false;
    for (let i = 0; i < value.length; i++) {
        const char = value[i];
        if (escape) escape = false;
        else if (quoted && char === '\\') escape = true;
        else if (char === '"') quoted = !quoted;
        else if (!quoted && char === delimiter) { result.push(value.slice(start, i)); start = i + 1; }
    }
    if (quoted || escape) throw new TypeError('unterminated quoted field parameter');
    result.push(value.slice(start));
    return result;
}
export function parseParameters(pieces: readonly string[]): Record<string, string> {
    const parameters: Record<string, string> = Object.create(null);
    for (const piece of pieces) {
        const trimmed = piece.trim();
        const equals = trimmed.indexOf('=');
        const key = trimmed.slice(0, equals).toLowerCase();
        const raw = trimmed.slice(equals + 1);
        if (equals < 1 || !token(key) || Object.hasOwn(parameters, key)) throw new TypeError('invalid or duplicate case-insensitive field parameter');
        let value: string;
        if (raw.startsWith('"')) {
            if (!raw.endsWith('"') || raw.length < 2) throw new TypeError('unterminated quoted field parameter');
            value = '';
            for (let i = 1; i < raw.length - 1; i++) {
                const char = raw[i]!;
                if (char === '"') throw new TypeError('unescaped field-parameter quote');
                if (char === '\\') {
                    if (++i >= raw.length - 1) throw new TypeError('unterminated quoted pair');
                    value += raw[i];
                } else value += char;
            }
        } else {
            if (!token(raw)) throw new TypeError('invalid field parameter value');
            value = raw;
        }
        Object.defineProperty(parameters, key, { value, enumerable: true });
    }
    return parameters;
}
export function parseMedia(value: string): ParsedMedia {
    if (value === '' || /[\u0000-\u0008\u000a-\u001f\u007f]/.test(value)) throw new TypeError('invalid Content-Type controls');
    const pieces = splitQuoted(value, ';');
    const names = pieces.shift()!.trim().split('/');
    const type = names[0]?.toLowerCase(), subtype = names[1]?.toLowerCase();
    if (names.length !== 2 || type === undefined || subtype === undefined || !token(type) || !token(subtype) || type.includes('*') || subtype.includes('*')) throw new TypeError('Content-Type must be concrete type/subtype');
    return { type, subtype, parameters: parseParameters(pieces) };
}
export function matchesMedia(declared: MediaType, actual: ParsedMedia): boolean {
    const range = declared.range;
    if (range.kind !== 'any' && range.type_name !== actual.type || range.kind === 'concrete' && range.subtype !== actual.subtype) return false;
    return Object.entries(declared.parameters).every(([key, value]) => Object.hasOwn(actual.parameters, key) &&
        (key === 'charset' ? actual.parameters[key]!.toLowerCase() === value.toLowerCase() : actual.parameters[key] === value));
}
export function selectMedia(media: readonly MediaPlan[], actual: ParsedMedia): MediaPlan | undefined {
    let selected: MediaPlan | undefined;
    let specificity = -1, parameters = -1;
    for (const candidate of media) {
        if (!matchesMedia(candidate.media_type, actual)) continue;
        const rank = candidate.media_type.range.kind === 'concrete' ? 2 : candidate.media_type.range.kind === 'type' ? 1 : 0;
        const count = Object.keys(candidate.media_type.parameters).length;
        if (rank > specificity || rank === specificity && count > parameters) { selected = candidate; specificity = rank; parameters = count; }
    }
    if (selected !== undefined && ['text', 'form', 'stream'].includes(selected.representation.kind) &&
        actual.parameters.charset !== undefined && actual.parameters.charset.toLowerCase() !== 'utf-8') throw new TypeError('declared text/framing only supports UTF-8');
    return selected;
}
export function canonicalMedia(media: MediaType, actual?: ParsedMedia): string {
    const range = media.range;
    if (range.kind !== 'concrete') {
        if (actual === undefined) return canonical(range.kind === 'any' ? '*' : range.type_name, '*', media.parameters);
        return canonical(actual.type, actual.subtype, actual.parameters);
    }
    return canonical(range.type_name, range.subtype, media.parameters);
}
function canonical(type: string, subtype: string, parameters: Readonly<Record<string, string>>): string {
    return `${type}/${subtype}${Object.keys(parameters).sort().map(key => {
        const value = key === 'charset' ? parameters[key]!.toLowerCase() : parameters[key]!;
        return `;${key}=${token(value) ? value : `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`}`;
    }).join('')}`;
}
export function selectResponse(operation: ProtocolOperation, status: number): ResponsePlan | undefined {
    if (!Number.isInteger(status) || status < 100 || status >= 600) return undefined;
    return operation.responses.find(response => response.status.kind === 'exact' && response.status.value === status) ??
        operation.responses.find(response => response.status.kind === 'range' && response.status.value === Math.floor(status / 100)) ??
        operation.responses.find(response => response.status.kind === 'default');
}
export const forbiddenBody = (method: string, status: number): boolean => method === 'HEAD' || status < 200 || status === 204 || status === 205 || status === 304;
