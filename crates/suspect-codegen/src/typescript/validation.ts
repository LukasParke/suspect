import { JsonNumber, type WireJsonValue } from './json.js';
import { checkCompiledPattern, matchesCompiledPattern, type CompiledPattern } from './pattern.js';
import { checkResourceContext, ResourceScope, type ValidationResourceContext } from './validation-resources.js';
export type { ValidationResource, ValidationResourceContext } from './validation-resources.js';

/** Original document identity and escaped RFC 6901 schema pointer. */
export interface ValidationSource { readonly document: string; readonly pointer: string }
/** One mismatch or one reason evaluation could not be completed. */
export interface ValidationFinding { readonly source: ValidationSource; readonly instancePath: string; readonly message: string }
/** Incomplete evaluation is never ordinary invalidity or successful validation. */
export type ValidationOutcome =
    | { readonly kind: 'valid' }
    | { readonly kind: 'invalid'; readonly findings: readonly ValidationFinding[] }
    | { readonly kind: 'evaluationFailure'; readonly finding: ValidationFinding };
/** A completed schema application; trial results do not establish root validity.
 * V3 also supplies an exact program-local ordered resource-context signature.
 * V1/V2 observers retain the original three-argument call.
 */
export type ValidationTrace = (source: ValidationSource, instancePath: string, valid: boolean, resourceContext?: string) => void;
type JsonType = 'null' | 'boolean' | 'integer' | 'number' | 'string' | 'array' | 'object';
type Instruction =
    | { readonly op: 'always'; readonly value: boolean }
    | { readonly op: 'type'; readonly types: readonly JsonType[] }
    | { readonly op: 'ref' | 'not'; readonly target: number }
    | { readonly op: 'dynamicRef'; readonly target: number; readonly initialResource: number; readonly anchor: string | null }
    | { readonly op: 'properties'; readonly properties: readonly { readonly name: string; readonly target: number }[] }
    | { readonly op: 'additionalProperties'; readonly declared: readonly string[]; readonly target: number }
    | { readonly op: 'required'; readonly names: readonly string[] }
    | { readonly op: 'items'; readonly target: number; readonly start: number }
    | { readonly op: 'prefixItems' | 'allOf' | 'anyOf' | 'oneOf'; readonly targets: readonly number[] }
    | { readonly op: 'bound'; readonly value: string; readonly maximum: boolean; readonly exclusive: boolean }
    | { readonly op: 'multipleOf'; readonly value: string }
    | { readonly op: 'count'; readonly value: string; readonly maximum: boolean; readonly target: 'string' | 'array' | 'object' }
    | { readonly op: 'pattern'; readonly program: CompiledPattern }
    | { readonly op: 'enum'; readonly values: readonly WireJsonValue[] }
    | { readonly op: 'const'; readonly value: WireJsonValue }
    | { readonly op: 'uniqueItems' }
    | { readonly op: 'if'; readonly condition: number; readonly thenTarget: number | null; readonly elseTarget: number | null }
    | { readonly op: 'dependentRequired'; readonly dependencies: readonly (readonly [string, readonly string[]])[] }
    | { readonly op: 'dependentSchemas'; readonly dependencies: readonly { readonly name: string; readonly target: number }[] }
    | { readonly op: 'contains'; readonly target: number; readonly minimum: string | null; readonly maximum: string | null }
    | { readonly op: 'patternProperties'; readonly patterns: readonly (readonly [string, CompiledPattern, number])[] }
    | { readonly op: 'additionalPropertiesWithPatterns'; readonly declared: readonly string[]; readonly target: number }
    | { readonly op: 'propertyNames' | 'unevaluatedProperties' | 'unevaluatedItems'; readonly target: number };
type Check = Instruction & { readonly source: ValidationSource };
type Node = { readonly source: ValidationSource; readonly checks: readonly Check[] };
/** Experimental compiled instruction graph; this is not raw OpenAPI/JSON Schema. */
export interface ValidationProgram {
    readonly version: 'suspect.validation.experimental.v1' | 'suspect.validation.experimental.v2' | 'suspect.validation.experimental.v3';
    readonly profile: 'oas31-jsonschema202012-static-subset' | 'oas31-jsonschema202012-static-applicators' | 'oas31-jsonschema202012-resources-dynamic';
    readonly roots: readonly { readonly source: ValidationSource; readonly target: number }[];
    /** Compiler-indexed node table. Generated root slices may contain deliberate holes for unreachable nodes. */
    readonly nodes: readonly (Node | undefined)[];
    readonly limits: {
        readonly maxDepth: number; readonly maxErrors: number; readonly maxNumberBytes: number;
        readonly maxEqualitySteps: number; readonly maxEvaluationSteps: number;
    };
    /** Present only in the explicitly checked resource/dynamic v3 profile. */
    readonly resourceContext?: ValidationResourceContext;
}

// Generated modules and builtins are trusted. Capture the base methods so a
// branded JsonNumber subclass cannot override token extraction or dispatch.
const isJsonNumber = JsonNumber.is;
const numberToken = JsonNumber.prototype.toString;
/** @internal Exact representation predicate for admitted typed codec plans. */
export function isIntegralJsonNumber(value: JsonNumber): boolean {
    const token = numberToken.call(value);
    return isIntegral(exact(token, token.length));
}
/** @internal Compares a wire number with an admitted numeric literal token. */
export function equalJsonNumberToken(value: JsonNumber, token: string): boolean {
    const actual = numberToken.call(value);
    return compare(exact(actual, actual.length), exact(token, token.length)) === 0;
}
const own = Object.prototype.hasOwnProperty;
const prototype = Object.prototype;
const rootKey = (source: ValidationSource): string => JSON.stringify([source.document, source.pointer]);
const childPath = (path: string, key: string): string => `${path}/${key.replace(/~/g, '~0').replace(/\//g, '~1')}`;
// Rust's retained JSON maps compare Unicode scalar values (UTF-8 lexical
// order). JavaScript's default UTF-16 sort reverses some BMP/astral pairs.
function compareKeys(left: string, right: string): number {
    let a = 0, b = 0;
    while (a < left.length && b < right.length) {
        const ac = left.codePointAt(a)!, bc = right.codePointAt(b)!;
        if (ac !== bc) return ac < bc ? -1 : 1;
        a += ac > 0xffff ? 2 : 1;
        b += bc > 0xffff ? 2 : 1;
    }
    return a < left.length ? 1 : b < right.length ? -1 : 0;
}

class EvaluationFailure extends Error {
    constructor(readonly finding: ValidationFinding) { super(finding.message); }
}
function fail(source: ValidationSource, instancePath: string, message: string): never {
    throw new EvaluationFailure({ source, instancePath, message });
}

const sourceChild = (source: ValidationSource, key: string): ValidationSource => ({ document: source.document, pointer: childPath(source.pointer, key) });
function instructionKeyword(check: Check): string | undefined {
    switch (check.op) {
        case 'always': return undefined;
        case 'ref': return '$ref';
        case 'dynamicRef': return '$dynamicRef';
        case 'additionalPropertiesWithPatterns': return 'additionalProperties';
        case 'bound': return check.maximum ? check.exclusive ? 'exclusiveMaximum' : 'maximum' : check.exclusive ? 'exclusiveMinimum' : 'minimum';
        case 'count': return `${check.maximum ? 'max' : 'min'}${check.target === 'string' ? 'Length' : check.target === 'array' ? 'Items' : 'Properties'}`;
        default: return check.op;
    }
}
function checkProgramData(program: ValidationProgram): void {
    const pending: { value: unknown; leave?: object }[] = [{ value: program }];
    const active = new Set<object>(), complete = new Set<object>();
    while (pending.length !== 0) {
        const entry = pending.pop()!;
        if (entry.leave !== undefined) { active.delete(entry.leave); complete.add(entry.leave); continue; }
        const value = entry.value;
        if (value === null || value === undefined || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean' || isJsonNumber(value)) continue;
        if (typeof value !== 'object' || (Array.isArray(value) ? Object.getPrototypeOf(value) !== Array.prototype : Object.getPrototypeOf(value) !== null && Object.getPrototypeOf(value) !== prototype)) throw new TypeError('compiled metadata must contain only data properties');
        if (active.has(value)) throw new TypeError('compiled metadata must be acyclic');
        if (complete.has(value)) continue;
        active.add(value); pending.push({ value: null, leave: value });
        for (const key of Reflect.ownKeys(value)) {
            const descriptor = Object.getOwnPropertyDescriptor(value, key)!;
            if (typeof key !== 'string' || !('value' in descriptor) || !descriptor.enumerable && !(Array.isArray(value) && key === 'length')) throw new TypeError('compiled metadata cannot use accessors or hidden properties');
            pending.push({ value: descriptor.value });
        }
    }
}
function checkProgramStructure(program: ValidationProgram, scoped: boolean): void {
    const unicode = (value: string): boolean => {
        for (const character of value) {
            const point = character.codePointAt(0)!;
            if (point >= 0xd800 && point <= 0xdfff) return false;
        }
        return true;
    };
    const literal = (value: WireJsonValue): void => {
        const pending: { value: unknown; leave?: object }[] = [{ value }];
        const active = new Set<object>();
        while (pending.length !== 0) {
            const entry = pending.pop()!;
            if (entry.leave !== undefined) { active.delete(entry.leave); continue; }
            const item = entry.value;
            if (item === null || typeof item === 'boolean' || isJsonNumber(item)) continue;
            if (typeof item === 'string') {
                if (!unicode(item)) throw new Error('invalid Unicode literal');
                continue;
            }
            if (typeof item !== 'object' || (!Array.isArray(item) && Object.getPrototypeOf(item) !== null && Object.getPrototypeOf(item) !== prototype) || active.has(item)) throw new Error('literal operand must be an acyclic exact JSON value');
            active.add(item);
            pending.push({ value: null, leave: item });
            if (Array.isArray(item)) {
                for (let index = 0; index < item.length; index++) {
                    const property = Object.getOwnPropertyDescriptor(item, String(index));
                    if (property === undefined || !property.enumerable || !('value' in property)) throw new Error('invalid literal array member');
                    pending.push({ value: property.value });
                }
                for (const key of Reflect.ownKeys(item)) {
                    if (key !== 'length' && (typeof key !== 'string' || !/^(0|[1-9][0-9]*)$/.test(key) || Number(key) >= item.length)) throw new Error('extra literal array member');
                }
            } else for (const key of Reflect.ownKeys(item)) {
                const property = Object.getOwnPropertyDescriptor(item, key)!;
                if (typeof key !== 'string' || !unicode(key) || !property.enumerable || !('value' in property)) throw new Error('invalid literal object member');
                pending.push({ value: property.value });
            }
        }
    };
    const names = (values: readonly string[], label: string): Set<string> => {
        if (!Array.isArray(values) || [...values].some(value => typeof value !== 'string' || !unicode(value)) || new Set(values).size !== values.length) throw new Error(`invalid or duplicate ${label}`);
        return new Set(values);
    };
    const source = (value: ValidationSource) => {
        if (value === null || typeof value !== 'object' || typeof value.document !== 'string' || !/^[A-Za-z][A-Za-z0-9+.-]*:/.test(value.document) || value.document.includes('#') || /[\u0000-\u0020\u007f]/.test(value.document) || typeof value.pointer !== 'string' || value.pointer !== '' && !value.pointer.startsWith('/') || /~(?![01])/.test(value.pointer)) throw new Error('invalid validation source identity');
    };
    for (const key of ['maxDepth', 'maxErrors', 'maxNumberBytes', 'maxEqualitySteps', 'maxEvaluationSteps'] as const) {
        if (!Number.isSafeInteger(program.limits[key]) || program.limits[key] < 0) throw new Error('invalid or missing validation limit');
    }
    const located = (target: number, expected: ValidationSource) => {
        if (!Number.isSafeInteger(target) || target < 0 || program.nodes[target] === undefined || rootKey(program.nodes[target]!.source) !== rootKey(expected)) throw new Error('applicator target has a different source identity');
    };
    const identities = new Set<string>();
    for (const node of program.nodes) {
        if (node === undefined) continue;
        source(node.source);
        if (identities.has(rootKey(node.source))) throw new Error('duplicate schema source identity');
        identities.add(rootKey(node.source));
        const seen = new Set<string>();
        let declared: Set<string> | undefined, properties = new Set<string>(), patterns = 0, additionalPatterns = false, prefix = 0, start: number | undefined, tail = false;
        for (const check of node.checks) {
            source(check.source);
            if (check.op === 'dynamicRef' && program.version !== 'suspect.validation.experimental.v3') throw new Error('v3 instruction in a v1/v2 program');
            if (['if','dependentRequired','dependentSchemas','contains','patternProperties','additionalPropertiesWithPatterns','propertyNames','unevaluatedProperties','unevaluatedItems'].includes(check.op) && !scoped) throw new Error('v2 instruction in a v1 program');
            const unevaluated = check.op === 'unevaluatedProperties' || check.op === 'unevaluatedItems';
            if (scoped && tail && !unevaluated) throw new Error('unevaluated checks must follow the other node checks');
            tail ||= unevaluated;
            const keyword = instructionKeyword(check), expected = keyword === undefined ? node.source : sourceChild(node.source, keyword);
            const normalized = check.op === 'bound' && check.exclusive && rootKey(check.source) === rootKey(sourceChild(node.source, check.maximum ? 'maximum' : 'minimum'));
            if (rootKey(check.source) !== rootKey(expected) && !normalized || seen.has(rootKey(check.source))) throw new Error('invalid or duplicate instruction source identity');
            seen.add(rootKey(check.source));
            switch (check.op) {
                case 'always': if (typeof check.value !== 'boolean' || node.checks.length !== 1) throw new Error('invalid boolean schema'); break;
                case 'type': if (names(check.types,'type').size === 0 || check.types.some(kind => !['null','boolean','integer','number','string','array','object'].includes(kind))) throw new Error('invalid type instruction'); break;
                case 'properties': properties = names(check.properties.map(property => property.name),'property'); for (const property of check.properties) located(property.target,sourceChild(check.source,property.name)); break;
                case 'additionalProperties': case 'additionalPropertiesWithPatterns': declared = names(check.declared,'declared property'); additionalPatterns = check.op === 'additionalPropertiesWithPatterns'; located(check.target,check.source); break;
                case 'required': names(check.names,'required property'); break;
                case 'items': if (!Number.isSafeInteger(check.start) || check.start < 0) throw new Error('invalid items start'); start = check.start; located(check.target,check.source); break;
                case 'prefixItems': case 'allOf': case 'anyOf': case 'oneOf':
                    if (!Array.isArray(check.targets) || check.targets.length === 0) throw new Error('empty applicator target list');
                    if (check.op === 'prefixItems') prefix = check.targets.length;
                    check.targets.forEach((target,index) => located(target,sourceChild(check.source,String(index)))); break;
                case 'not': case 'propertyNames': case 'unevaluatedProperties': case 'unevaluatedItems': located(check.target,check.source); break;
                case 'if': located(check.condition,check.source); if (check.thenTarget !== null) located(check.thenTarget,sourceChild(node.source,'then')); if (check.elseTarget !== null) located(check.elseTarget,sourceChild(node.source,'else')); break;
                case 'dependentRequired': names(check.dependencies.map(entry => entry[0]),'dependency trigger'); for (const entry of check.dependencies) { if (entry.length !== 2) throw new Error('invalid dependency tuple'); names(entry[1],'dependent required property'); } break;
                case 'dependentSchemas': names(check.dependencies.map(entry => entry.name),'dependency trigger'); for (const entry of check.dependencies) located(entry.target,sourceChild(check.source,entry.name)); break;
                case 'contains': located(check.target,check.source); if (check.minimum !== null && typeof check.minimum !== 'string' || check.maximum !== null && typeof check.maximum !== 'string') throw new Error('invalid contains count'); break;
                case 'patternProperties': names(check.patterns.map(entry => entry[0]),'property pattern'); patterns = check.patterns.length; for (const entry of check.patterns) { if (entry.length !== 3) throw new Error('invalid pattern tuple'); checkCompiledPattern(entry[1]); located(entry[2],sourceChild(check.source,entry[0])); } break;
                case 'bound': if (typeof check.maximum !== 'boolean' || typeof check.exclusive !== 'boolean') throw new Error('invalid bound flags'); break;
                case 'multipleOf': if (compare(exact(check.value,program.limits.maxNumberBytes),exact('0',1)) <= 0) throw new Error('invalid divisor'); break;
                case 'count': { const value = exact(check.value,program.limits.maxNumberBytes); if (!isIntegral(value) || exactState(value).negative || !['string','array','object'].includes(check.target) || typeof check.maximum !== 'boolean') throw new Error('invalid count'); break; }
                case 'enum': if (!Array.isArray(check.values)) throw new Error('invalid enum operands'); for (const value of check.values) literal(value); break;
                case 'const': literal(check.value); break;
                case 'ref': case 'dynamicRef': case 'pattern': case 'uniqueItems': break;
                default: throw new Error('unknown validation instruction');
            }
        }
        if (declared !== undefined && (declared.size !== properties.size || [...declared].some(name => !properties.has(name)) || additionalPatterns !== (patterns > 0))) throw new Error('additional-properties exclusions disagree with adjacent declarations');
        if (start !== undefined && start !== prefix) throw new Error('items start disagrees with prefixItems');
    }
}

/**
 * Builds a validator for an admitted compiled program. Each call owns its
 * evaluation/equality budgets. Pass parseJson values or valid WireJsonValue
 * constructions; this does not coerce JavaScript numbers into exact numbers.
 * Program initialization is outside per-call visit accounting.
 */
export function createValidator(program: ValidationProgram): (root: ValidationSource, value: WireJsonValue, trace?: ValidationTrace) => ValidationOutcome {
    checkProgramData(program);
    const resources = program.version === 'suspect.validation.experimental.v3' && program.profile === 'oas31-jsonschema202012-resources-dynamic';
    const scoped = resources || program.version === 'suspect.validation.experimental.v2' && program.profile === 'oas31-jsonschema202012-static-applicators';
    if (!scoped && !(program.version === 'suspect.validation.experimental.v1' && program.profile === 'oas31-jsonschema202012-static-subset')) throw new Error('unsupported validation program');
    if (!resources && Object.hasOwn(program, 'resourceContext')) throw new Error('resource metadata requires the v3 profile');
    for (const cap of Object.values(program.limits)) {
        if (!Number.isSafeInteger(cap) || cap < 0) throw new Error('invalid validation program limit');
    }
    const target = (index: number): void => {
        if (!Number.isSafeInteger(index) || index < 0 || index >= program.nodes.length || program.nodes[index] === undefined) throw new Error('invalid validation target');
    };
    const roots = new Map<string, number>();
    for (const root of program.roots) {
        target(root.target);
        if (roots.has(rootKey(root.source)) || rootKey(program.nodes[root.target]!.source) !== rootKey(root.source)) throw new Error('inconsistent selected root');
        roots.set(rootKey(root.source), root.target);
    }
    const bounds = new Map<Check, Exact>();
    const declared = new Map<Check, ReadonlySet<string>>();
    const containsBounds = new Map<Check, { readonly minimum: Exact | undefined; readonly maximum: Exact | undefined }>();
    for (const node of program.nodes) {
        if (node === undefined) continue;
        for (const check of node.checks) {
            switch (check.op) {
                case 'ref': case 'not': case 'dynamicRef': target(check.target); break;
                case 'properties': for (const property of check.properties) target(property.target); break;
                case 'additionalProperties': case 'additionalPropertiesWithPatterns': target(check.target); declared.set(check, new Set(check.declared)); break;
                case 'items':
                    target(check.target);
                    if (!Number.isSafeInteger(check.start) || check.start < 0) throw new Error('invalid items start');
                    break;
                case 'prefixItems': case 'allOf': case 'anyOf': case 'oneOf':
                    for (const index of check.targets) target(index);
                    break;
                case 'bound': case 'multipleOf': case 'count': bounds.set(check, exact(check.value, program.limits.maxNumberBytes)); break;
                case 'pattern': checkCompiledPattern(check.program); break;
                case 'if': target(check.condition); if (check.thenTarget !== null) target(check.thenTarget); if (check.elseTarget !== null) target(check.elseTarget); break;
                case 'dependentRequired': break;
                case 'dependentSchemas': for (const dependency of check.dependencies) target(dependency.target); break;
                case 'contains': {
                    target(check.target);
                    const count = (token: string | null) => {
                        if (token === null) return undefined;
                        const value = exact(token, program.limits.maxNumberBytes);
                        if (!isIntegral(value) || exactState(value).negative) throw new Error('contains bounds require nonnegative integral tokens');
                        return value;
                    };
                    containsBounds.set(check, { minimum: count(check.minimum), maximum: count(check.maximum) });
                    break;
                }
                case 'patternProperties': for (const [, pattern, index] of check.patterns) { checkCompiledPattern(pattern); target(index); } break;
                case 'propertyNames': case 'unevaluatedProperties': case 'unevaluatedItems': target(check.target); break;
                case 'always': case 'type': case 'required': case 'enum': case 'const': case 'uniqueItems': break;
                default: throw new Error('unknown compiled validation instruction');
            }
        }
    }
    checkProgramStructure(program, scoped);
    const resourceContext = resources ? checkResourceContext(program) : undefined;
    // The generated module keeps the program private. Freeze it defensively
    // for direct typed callers; exact numeric private brands remain intact.
    const pending: object[] = [program];
    const frozen = new Set<object>();
    while (pending.length !== 0) {
        const value = pending.pop()!;
        if (frozen.has(value)) continue;
        frozen.add(value);
        if (!isJsonNumber(value)) {
            for (const child of Object.values(value)) if (child !== null && typeof child === 'object') pending.push(child);
        }
        Object.freeze(value);
    }

    return (root, value, trace) => {
        const selected = roots.get(rootKey(root));
        if (selected === undefined) return { kind: 'evaluationFailure', finding: { source: root, instancePath: '', message: 'schema root was not selected during compilation' } };
        const state = scoped ? new ScopedState(program, bounds, declared, trace, containsBounds, resourceContext) : new State(program, bounds, declared, trace);
        try {
            return state.eval(selected, value, '') ? { kind: 'valid' } : { kind: 'invalid', findings: state.findings };
        } catch (error) {
            if (error instanceof EvaluationFailure) return { kind: 'evaluationFailure', finding: error.finding };
            // Native representation/arithmetic/stack failures are incomplete
            // evaluation too; they cannot flow through logical inversion.
            return { kind: 'evaluationFailure', finding: { source: state.source, instancePath: state.path, message: error instanceof Error ? error.message : 'unexpected evaluation failure' } };
        }
    };
}

/** @internal Defers trusted generated program initialization until its selected root is first used. */
export function createLazyValidator(factory: () => ReturnType<typeof createValidator>): ReturnType<typeof createValidator> {
    let validator: ReturnType<typeof createValidator> | undefined;
    return (root, value, trace) => (validator ??= factory())(root, value, trace);
}

class State {
    findings: ValidationFinding[] = [];
    source: ValidationSource = { document: '', pointer: '' };
    path = '';
    private steps: number;
    private equalitySteps: number;
    private depth = 0;
    private readonly active = new Set<string>();
    private readonly numbers = new WeakMap<JsonNumber, Exact>();
    constructor(protected readonly program: ValidationProgram, protected readonly bounds: ReadonlyMap<Check, Exact>, protected readonly declared: ReadonlyMap<Check, ReadonlySet<string>>, protected readonly trace: ValidationTrace | undefined) {
        this.steps = program.limits.maxEvaluationSteps;
        this.equalitySteps = program.limits.maxEqualitySteps;
    }
    protected step(source: ValidationSource, path: string): void {
        this.source = source; this.path = path;
        if (this.steps === 0) fail(source, path, `schema evaluation exceeds ${this.program.limits.maxEvaluationSteps} evaluation steps`);
        this.steps--;
    }
    protected emit(source: ValidationSource, instancePath: string, message: string): void {
        if (this.program.limits.maxErrors === 0 || this.findings.length < this.program.limits.maxErrors) this.findings.push({ source, instancePath, message });
    }
    protected kind(value: WireJsonValue, source: ValidationSource, path: string): Exclude<JsonType, 'integer'> {
        if (value === null) return 'null';
        if (isJsonNumber(value)) return 'number';
        if (Array.isArray(value)) return 'array';
        if (typeof value === 'boolean') return 'boolean';
        if (typeof value === 'string') return 'string';
        if (typeof value === 'object' && (Object.getPrototypeOf(value) === null || Object.getPrototypeOf(value) === prototype)) return 'object';
        return fail(source, path, 'instance is not an exact WireJsonValue');
    }
    protected number(value: JsonNumber, source: ValidationSource, path: string): Exact {
        let number = this.numbers.get(value);
        if (number === undefined) {
            try { number = exact(numberToken.call(value), this.program.limits.maxNumberBytes); }
            catch (error) { return fail(source, path, error instanceof Error ? error.message : 'exact numeric evaluation failed'); }
            this.numbers.set(value, number);
        }
        return number;
    }
    protected member(object: object, name: string, source: ValidationSource, path: string): WireJsonValue {
        const descriptor = Object.getOwnPropertyDescriptor(object, name);
        if (descriptor === undefined || !('value' in descriptor) || !descriptor.enumerable) return fail(source, path, 'instance member is not an enumerable JSON data property');
        return descriptor.value as WireJsonValue;
    }
    eval(index: number, value: WireJsonValue, path: string): boolean {
        const node = this.program.nodes[index]!;
        this.step(node.source, path);
        if (this.depth >= this.program.limits.maxDepth) fail(node.source, path, `schema evaluation depth exceeds ${this.program.limits.maxDepth}`);
        const key = `${index}\u0000${path}`;
        if (this.active.has(key)) fail(node.source, path, 'recursive schema evaluation revisited the same schema and instance without progress');
        this.active.add(key); this.depth++;
        try {
            const kind = this.kind(value, node.source, path);
            let valid = true;
            for (const check of node.checks) {
                this.step(check.source, path);
                const accepted = this.check(check, value, kind, path);
                valid = accepted && valid;
            }
            if (this.trace !== undefined) {
                try { this.trace(node.source, path, valid); }
                catch (error) { fail(node.source, path, `validation trace failed: ${error instanceof Error ? error.message : 'observer threw'}`); }
            }
            return valid;
        } finally { this.depth--; this.active.delete(key); }
    }
    private trial(index: number, value: WireJsonValue, path: string): boolean {
        const findings = this.findings;
        this.findings = [];
        try { return this.eval(index, value, path); }
        finally { this.findings = findings; }
    }
    protected check(check: Check, value: WireJsonValue, kind: Exclude<JsonType, 'integer'>, path: string): boolean {
        const source = check.source;
        let valid = true;
        let message = '';
        switch (check.op) {
            case 'always': valid = check.value; message = 'value is rejected by a false schema'; break;
            case 'type':
                valid = check.types.includes(kind) || (kind === 'number' && check.types.includes('integer') && isIntegral(this.number(value as JsonNumber, source, path)));
                message = 'instance does not match the declared type'; break;
            case 'ref': return this.eval(check.target, value, path);
            case 'properties':
                if (kind === 'object') for (const property of check.properties) {
                    this.step(source, path);
                    if (own.call(value, property.name)) {
                        const accepted = this.eval(property.target, this.member(value as object, property.name, source, path), childPath(path, property.name));
                        valid = accepted && valid;
                    }
                }
                return valid;
            case 'additionalProperties':
                if (kind === 'object') for (const name of Object.keys(value as object).sort(compareKeys)) {
                    this.step(source, path);
                    if (!this.declared.get(check)!.has(name)) {
                        const accepted = this.eval(check.target, this.member(value as object, name, source, path), childPath(path, name));
                        valid = accepted && valid;
                    }
                }
                return valid;
            case 'required':
                if (kind === 'object') for (const name of check.names) {
                    this.step(source, path);
                    if (!own.call(value, name)) { this.emit(source, path, `required property ${JSON.stringify(name)} is absent`); valid = false; }
                }
                return valid;
            case 'items':
                if (kind === 'array') for (let index = check.start; index < (value as WireJsonValue[]).length; index++) {
                    this.step(source, path);
                    const accepted = this.eval(check.target, this.member(value as object, String(index), source, path), childPath(path, String(index)));
                    valid = accepted && valid;
                }
                return valid;
            case 'prefixItems':
                if (kind === 'array') for (let index = 0; index < Math.min(check.targets.length, (value as WireJsonValue[]).length); index++) {
                    this.step(source, path);
                    const accepted = this.eval(check.targets[index]!, this.member(value as object, String(index), source, path), childPath(path, String(index)));
                    valid = accepted && valid;
                }
                return valid;
            case 'allOf':
                for (const index of check.targets) { this.step(source, path); const accepted = this.eval(index, value, path); valid = accepted && valid; }
                return valid;
            case 'anyOf': case 'oneOf': {
                let accepted = 0;
                for (const index of check.targets) { this.step(source, path); if (this.trial(index, value, path)) accepted++; }
                valid = check.op === 'anyOf' ? accepted !== 0 : accepted === 1;
                message = `composition matched ${accepted} alternatives`; break;
            }
            case 'not': valid = !this.trial(check.target, value, path); message = 'instance matches the negated schema'; break;
            case 'bound':
                if (kind === 'number') {
                    const order = compare(this.number(value as JsonNumber, source, path), this.bounds.get(check)!);
                    valid = (check.maximum ? order < 0 : order > 0) || (order === 0 && !check.exclusive);
                }
                message = `numeric value violates bound ${check.value}`; break;
            case 'multipleOf':
                if (kind === 'number') valid = multipleOf(this.number(value as JsonNumber, source, path), this.bounds.get(check)!);
                message = `numeric value is not a multiple of ${check.value}`; break;
            case 'count':
                if (kind === check.target) {
                    let count = 0;
                    if (kind === 'string') { for (const _ of value as string) count++; }
                    else count = kind === 'array' ? (value as WireJsonValue[]).length : Object.keys(value as object).length;
                    // Resident lengths are safe integers. Their representation is
                    // internal metadata, not an instance numeric operand cap.
                    const order = compare(exact(String(count), 32), this.bounds.get(check)!);
                    valid = check.maximum ? order <= 0 : order >= 0;
                    message = `size ${count} violates cardinality ${check.value}`;
                }
                break;
            case 'enum':
                valid = false;
                for (const operand of check.values) { this.step(source, path); if (this.equal(value, operand, source, path)) { valid = true; break; } }
                message = 'instance does not equal any enum value'; break;
            case 'pattern':
                if (kind === 'string') valid = matchesCompiledPattern(check.program, value as string, () => this.step(source, path));
                message = 'string does not match the compiled pattern'; break;
            case 'const': valid = this.equal(value, check.value, source, path); message = 'instance does not equal the const value'; break;
            case 'uniqueItems':
                if (kind === 'array') {
                    const array = value as WireJsonValue[];
                    outer: for (let index = 0; index < array.length; index++) {
                        this.step(source, path);
                        for (let previous = 0; previous < index; previous++) {
                            this.step(source, path);
                            if (this.equal(this.member(array, String(index), source, path), this.member(array, String(previous), source, path), source, path)) { valid = false; break outer; }
                        }
                    }
                }
                message = 'array contains equal items'; break;
            default: return fail(source, path, 'unknown compiled validation instruction');
        }
        if (!valid) this.emit(source, path, message);
        return valid;
    }
    private equal(left: WireJsonValue, right: WireJsonValue, source: ValidationSource, path: string): boolean {
        const pending: [WireJsonValue, WireJsonValue, number][] = [[left, right, 0]];
        while (pending.length !== 0) {
            const [a, b, depth] = pending.pop()!;
            if (this.equalitySteps === 0) fail(source, path, `equality evaluation exceeds ${this.program.limits.maxEqualitySteps} node comparisons`);
            this.equalitySteps--;
            if (depth > this.program.limits.maxDepth) fail(source, path, `equality evaluation depth exceeds ${this.program.limits.maxDepth}`);
            const kind = this.kind(a, source, path);
            if (kind !== this.kind(b, source, path)) return false;
            switch (kind) {
                case 'null': break;
                case 'boolean': case 'string': if (a !== b) return false; break;
                case 'number': if (compare(this.number(a as JsonNumber, source, path), this.number(b as JsonNumber, source, path)) !== 0) return false; break;
                case 'array': {
                    const aa = a as WireJsonValue[], bb = b as WireJsonValue[];
                    if (aa.length !== bb.length) return false;
                    if (aa.length + pending.length > this.equalitySteps) fail(source, path, `equality evaluation exceeds ${this.program.limits.maxEqualitySteps} node comparisons`);
                    for (let index = aa.length - 1; index >= 0; index--) pending.push([this.member(aa, String(index), source, path), this.member(bb, String(index), source, path), depth + 1]);
                    break;
                }
                case 'object': {
                    const keys = Object.keys(a as object).sort(compareKeys);
                    if (keys.length !== Object.keys(b as object).length) return false;
                    if (keys.length + pending.length > this.equalitySteps) fail(source, path, `equality evaluation exceeds ${this.program.limits.maxEqualitySteps} node comparisons`);
                    for (let index = keys.length - 1; index >= 0; index--) {
                        const key = keys[index]!;
                        if (!own.call(b, key)) return false;
                        pending.push([this.member(a as object, key, source, path), this.member(b as object, key, source, path), depth + 1]);
                    }
                    break;
                }
            }
        }
        return true;
    }
}

interface Evaluated {
    readonly valid: boolean;
    readonly properties: Set<string>;
    readonly items: Set<number>;
}
const evaluation = (valid = true): Evaluated => ({ valid, properties: new Set<string>(), items: new Set<number>() });

/** V2 scopes are separate from v1 so frozen v1 visit counts stay unchanged. */
class ScopedState extends State {
    private scopedDepth = 0;
    private readonly scopedActive = new Set<string>();
    private readonly identities = new WeakMap<object, number>();
    private readonly children = new WeakMap<object, Map<string, object>>();
    private nextIdentity = 0;
    private readonly resources: ResourceScope | undefined;
    constructor(program: ValidationProgram, bounds: ReadonlyMap<Check, Exact>, declared: ReadonlyMap<Check, ReadonlySet<string>>, trace: ValidationTrace | undefined,
        private readonly containsBounds: ReadonlyMap<Check, { readonly minimum: Exact | undefined; readonly maximum: Exact | undefined }>, resourceContext: ValidationResourceContext | undefined) {
        super(program, bounds, declared, trace);
        this.resources = resourceContext === undefined ? undefined : new ResourceScope(resourceContext, (source, path) => this.step(source, path));
    }
    override eval(index: number, value: WireJsonValue, path: string): boolean {
        return this.scoped(index, value, path, typeof value === 'object' && value !== null ? value : {}).valid;
    }
    private childIdentity(parent: object, name: string, value: WireJsonValue): object {
        if (typeof value === 'object' && value !== null) return value;
        let children = this.children.get(parent);
        if (children === undefined) { children = new Map<string, object>(); this.children.set(parent, children); }
        let identity = children.get(name);
        if (identity === undefined) { identity = {}; children.set(name, identity); }
        return identity;
    }
    private merge(target: Evaluated, produced: Evaluated, source: ValidationSource, path: string): void {
        for (const name of [...produced.properties].sort(compareKeys)) { this.step(source, path); target.properties.add(name); }
        for (const index of [...produced.items].sort((a, b) => a - b)) { this.step(source, path); target.items.add(index); }
    }
    private scoped(index: number, value: WireJsonValue, path: string, identity: object): Evaluated {
        const node = this.program.nodes[index]!;
        this.step(node.source, path);
        if (this.scopedDepth >= this.program.limits.maxDepth) fail(node.source, path, `schema evaluation depth exceeds ${this.program.limits.maxDepth}`);
        let identityIndex = this.identities.get(identity);
        if (identityIndex === undefined) { identityIndex = this.nextIdentity++; this.identities.set(identity, identityIndex); }
        const entered = this.resources?.enter(index, node.source, path);
        const active = `${index}:${identityIndex}:${this.resources?.identity ?? 0}`;
        if (this.scopedActive.has(active)) {
            this.resources?.leave(entered);
            fail(node.source, path, 'recursive schema evaluation revisited the same schema and instance without progress');
        }
        this.scopedActive.add(active); this.scopedDepth++;
        try {
            const kind = this.kind(value, node.source, path), local = evaluation();
            let valid = true;
            for (const check of node.checks) {
                this.step(check.source, path);
                const result = this.apply(node, check, value, kind, path, identity, local);
                valid = result.valid && valid;
                if (result.valid) this.merge(local, result, check.source, path);
            }
            if (this.trace !== undefined) {
                try {
                    if (this.resources === undefined) this.trace(node.source, path, valid);
                    else this.trace(node.source, path, valid, this.resources.signature);
                }
                catch (error) { fail(node.source, path, `validation trace failed: ${error instanceof Error ? error.message : 'observer threw'}`); }
            }
            return valid ? local : evaluation(false);
        } finally { this.scopedDepth--; this.scopedActive.delete(active); this.resources?.leave(entered); }
    }
    private scopedTrial(index: number, value: WireJsonValue, path: string, identity: object): Evaluated {
        const findings = this.findings; this.findings = [];
        try { return this.scoped(index, value, path, identity); }
        finally { this.findings = findings; }
    }
    private apply(node: Node, check: Check, value: WireJsonValue, kind: Exclude<JsonType, 'integer'>, path: string, identity: object, local: Evaluated): Evaluated {
        const source = check.source, produced = evaluation();
        let valid = true;
        const child = (target: number, name: string) => {
            const member = this.member(value as object, name, source, path);
            return this.scoped(target, member, childPath(path, name), this.childIdentity(identity, name, member));
        };
        switch (check.op) {
            case 'ref': return this.scoped(check.target, value, path, identity);
            case 'dynamicRef':
                return this.scoped(this.resources!.target(check.target, check.anchor, source, path), value, path, identity);
            case 'properties':
                if (kind === 'object') for (const property of check.properties) {
                    this.step(source, path);
                    if (own.call(value, property.name)) {
                        valid = child(property.target, property.name).valid && valid;
                        produced.properties.add(property.name);
                    }
                }
                break;
            case 'additionalProperties': case 'additionalPropertiesWithPatterns': {
                const patterns = node.checks.find((check): check is Extract<Check, { op: 'patternProperties' }> => check.op === 'patternProperties')?.patterns ?? [];
                if (kind === 'object') for (const name of Object.keys(value as object).sort(compareKeys)) {
                    this.step(source, path);
                    if (this.declared.get(check)!.has(name)) continue;
                    let excluded = false;
                    if (check.op === 'additionalPropertiesWithPatterns') for (const [, pattern] of patterns) {
                        this.step(source, path);
                        if (matchesCompiledPattern(pattern, name, () => this.step(source, path))) { excluded = true; break; }
                    }
                    if (!excluded) { valid = child(check.target, name).valid && valid; produced.properties.add(name); }
                }
                break;
            }
            case 'items': case 'prefixItems':
                if (kind === 'array') {
                    const begin = check.op === 'items' ? check.start : 0;
                    const end = check.op === 'items' ? (value as WireJsonValue[]).length : Math.min(check.targets.length, (value as WireJsonValue[]).length);
                    for (let index = begin; index < end; index++) {
                        this.step(source, path);
                        valid = child(check.op === 'items' ? check.target : check.targets[index]!, String(index)).valid && valid;
                        produced.items.add(index);
                    }
                }
                break;
            case 'allOf': case 'anyOf': case 'oneOf': {
                const passing: Evaluated[] = [];
                for (const index of check.targets) {
                    this.step(source, path);
                    const result = check.op === 'allOf' ? this.scoped(index, value, path, identity) : this.scopedTrial(index, value, path, identity);
                    valid = result.valid && valid;
                    if (result.valid) passing.push(result);
                }
                if (check.op !== 'allOf') {
                    valid = check.op === 'anyOf' ? passing.length > 0 : passing.length === 1;
                    if (!valid) this.emit(source, path, `composition matched ${passing.length} alternatives`);
                }
                if (valid) for (const result of passing) this.merge(produced, result, source, path);
                break;
            }
            case 'not':
                valid = !this.scopedTrial(check.target, value, path, identity).valid;
                if (!valid) this.emit(source, path, 'instance matches the negated schema');
                break;
            case 'if': {
                const condition = this.scopedTrial(check.condition, value, path, identity);
                if (condition.valid) this.merge(local, condition, source, path);
                const selected = condition.valid ? check.thenTarget : check.elseTarget;
                return selected === null ? produced : this.scoped(selected, value, path, identity);
            }
            case 'dependentRequired':
                if (kind === 'object') for (const [trigger, names] of check.dependencies) {
                    this.step(source, path);
                    if (own.call(value, trigger)) for (const name of names) {
                        this.step(source, path);
                        if (!own.call(value, name)) {
                            this.emit(sourceChild(source, trigger), path, `required property ${JSON.stringify(name)} is absent while ${JSON.stringify(trigger)} is present`);
                            valid = false;
                        }
                    }
                }
                break;
            case 'dependentSchemas': {
                const passing: Evaluated[] = [];
                if (kind === 'object') for (const dependency of check.dependencies) {
                    this.step(source, path);
                    if (own.call(value, dependency.name)) {
                        const result = this.scoped(dependency.target, value, path, identity);
                        valid = result.valid && valid;
                        if (result.valid) passing.push(result);
                    }
                }
                if (valid) for (const result of passing) this.merge(produced, result, source, path);
                break;
            }
            case 'contains':
                if (kind === 'array') {
                    const matched = evaluation();
                    for (let index = 0; index < (value as WireJsonValue[]).length; index++) {
                        this.step(source, path);
                        const name = String(index), member = this.member(value as object, name, source, path);
                        if (this.scopedTrial(check.target, member, childPath(path, name), this.childIdentity(identity, name, member)).valid) matched.items.add(index);
                    }
                    const bounds = this.containsBounds.get(check)!, count = exact(String(matched.items.size), 32);
                    const zeroMinimum = bounds.minimum !== undefined && compare(bounds.minimum, exact('0', 1)) === 0;
                    // Count assertions can fail after contains has contributed its
                    // local set. A failing enclosing scope still exports no sets.
                    if (matched.items.size !== 0 || zeroMinimum) this.merge(local, matched, source, path);
                    const lower = bounds.minimum === undefined ? matched.items.size >= 1 : compare(count, bounds.minimum) >= 0;
                    const upper = bounds.maximum === undefined || compare(count, bounds.maximum) <= 0;
                    if (!lower) this.emit(check.minimum === null ? source : sourceChild(node.source, 'minContains'), path, `array has ${matched.items.size} contains matches, fewer than required`);
                    if (!upper) this.emit(sourceChild(node.source, 'maxContains'), path, `array has ${matched.items.size} contains matches, more than allowed`);
                    valid = lower && upper;
                }
                break;
            case 'patternProperties':
                if (kind === 'object') for (const name of Object.keys(value as object).sort(compareKeys)) {
                    this.step(source, path);
                    for (const [, pattern, target] of check.patterns) {
                        this.step(source, path);
                        if (matchesCompiledPattern(pattern, name, () => this.step(source, path))) {
                            valid = child(target, name).valid && valid;
                            produced.properties.add(name);
                        }
                    }
                }
                break;
            case 'propertyNames':
                if (kind === 'object') for (const name of Object.keys(value as object).sort(compareKeys)) {
                    this.step(source, path);
                    valid = this.scoped(check.target, name, childPath(path, name), {}).valid && valid;
                }
                break;
            case 'unevaluatedProperties':
                if (kind === 'object') for (const name of Object.keys(value as object).sort(compareKeys)) {
                    this.step(source, path);
                    if (!local.properties.has(name)) { valid = child(check.target, name).valid && valid; produced.properties.add(name); }
                }
                break;
            case 'unevaluatedItems':
                if (kind === 'array') for (let index = 0; index < (value as WireJsonValue[]).length; index++) {
                    this.step(source, path);
                    if (!local.items.has(index)) { valid = child(check.target, String(index)).valid && valid; produced.items.add(index); }
                }
                break;
            default: valid = super.check(check, value, kind, path);
        }
        return valid ? produced : evaluation(false);
    }
}

// Exact decimal arithmetic over private token strings.
/** Private exact decimal arithmetic for schema evaluation, not SDK value types. */
declare const exactBrand: unique symbol;
interface Exact { readonly [exactBrand]: true; }

/** Numeric work failures must remain evaluation failures, never invalid results. */
class ExactNumberError extends Error {
    override readonly name = 'ExactNumberError';
    constructor(readonly kind: 'syntax' | 'limit' | 'operand', message: string) {
        super(message);
    }
}

type ExactState = {
    readonly negative: boolean;
    /** Empty for zero; otherwise no leading or trailing decimal zeroes. */
    readonly digits: string;
    readonly exponent: bigint;
    readonly order: bigint;
    factors?: ExactFactors;
};
type ExactFactors = { readonly coprime: bigint; readonly twos: bigint; readonly fives: bigint };
const exactStates = new WeakMap<Exact, ExactState>();

function exactState(value: Exact): ExactState {
    const state = exactStates.get(value);
    if (state === undefined) {
        throw new ExactNumberError('operand', 'expected a numeric value created by exact()');
    }
    return state;
}
function exactWork<T>(operation: () => T): T {
    try { return operation(); }
    catch (error) {
        if (error instanceof RangeError) {
            throw new ExactNumberError('limit', 'exact numeric work exceeds the native BigInt resource limit');
        }
        throw error;
    }
}
function exactDigit(code: number): boolean { return code >= 48 && code <= 57; }
function exactValue(negative: boolean, digits: string, exponent: bigint): Exact {
    const value = Object.freeze({}) as Exact;
    exactStates.set(value, { negative, digits, exponent, order: exponent + BigInt(digits.length) });
    return value;
}

/** Checks JSON numeric grammar and byte admission before exact normalization. */
function exact(token: string, maxNumberBytes: number): Exact {
    if (typeof token !== 'string') {
        throw new ExactNumberError('syntax', 'numeric token must be a JSON number string');
    }
    if (!Number.isSafeInteger(maxNumberBytes) || maxNumberBytes < 0) {
        throw new ExactNumberError('limit', 'maxNumberBytes must be a nonnegative safe integer');
    }
    // Every accepted token is ASCII, so its code-unit count is its byte count.
    if (token.length > maxNumberBytes) {
        throw new ExactNumberError('limit', `exact numeric operand exceeds ${maxNumberBytes} source bytes`);
    }
    const fail = (): never => { throw new ExactNumberError('syntax', 'expected a JSON numeric token'); };
    let at = 0;
    const negative = token[at] === '-';
    if (negative) at++;
    const integerStart = at;
    if (token[at] === '0') at++;
    else {
        const first = token.charCodeAt(at);
        if (!(first >= 49 && first <= 57)) fail();
        do { at++; } while (exactDigit(token.charCodeAt(at)));
    }
    const integerEnd = at;
    let fractionStart = at;
    let fractionEnd = at;
    if (token[at] === '.') {
        fractionStart = ++at;
        if (!exactDigit(token.charCodeAt(at))) fail();
        do { at++; } while (exactDigit(token.charCodeAt(at)));
        fractionEnd = at;
    }
    let exponentStart = -1;
    if (token[at] === 'e' || token[at] === 'E') {
        exponentStart = ++at;
        if (token[at] === '+' || token[at] === '-') at++;
        if (!exactDigit(token.charCodeAt(at))) fail();
        do { at++; } while (exactDigit(token.charCodeAt(at)));
    }
    if (at !== token.length) fail();

    return exactWork(() => {
        const coefficient = token.slice(integerStart, integerEnd) + token.slice(fractionStart, fractionEnd);
        let first = 0;
        while (first < coefficient.length && coefficient.charCodeAt(first) === 48) first++;
        if (first === coefficient.length) return exactValue(false, '', 0n);
        let end = coefficient.length;
        while (coefficient.charCodeAt(end - 1) === 48) end--;
        const writtenExponent = exponentStart < 0 ? 0n : BigInt(token.slice(exponentStart));
        const exponent = writtenExponent - BigInt(fractionEnd - fractionStart) + BigInt(coefficient.length - end);
        return exactValue(negative, coefficient.slice(first, end), exponent);
    });
}

/** Normalization leaves a negative exponent exactly when a value is fractional. */
function isIntegral(value: Exact): boolean { return exactState(value).exponent >= 0n; }

/** Compares written coefficients with implicit zero padding; never expands 10^e. */
function compare(left: Exact, right: Exact): -1 | 0 | 1 {
    const a = exactState(left);
    const b = exactState(right);
    if (a.negative !== b.negative) return a.negative ? -1 : 1;
    let result: -1 | 0 | 1 = 0;
    if (a.digits === '') result = b.digits === '' ? 0 : -1;
    else if (b.digits === '') result = 1;
    else if (a.order !== b.order) result = a.order < b.order ? -1 : 1;
    else {
        const length = Math.max(a.digits.length, b.digits.length);
        for (let i = 0; i < length; i++) {
            const ad = i < a.digits.length ? a.digits.charCodeAt(i) : 48;
            const bd = i < b.digits.length ? b.digits.charCodeAt(i) : 48;
            if (ad !== bd) { result = ad < bd ? -1 : 1; break; }
        }
    }
    return a.negative && result !== 0 ? result === 1 ? -1 : 1 : result;
}

function exactFactors(state: ExactState): ExactFactors {
    if (state.factors !== undefined) return state.factors;
    let coprime = BigInt(state.digits);
    let twos = 0n;
    let fives = 0n;
    const last = state.digits.charCodeAt(state.digits.length - 1);
    // A normalized coefficient cannot contain both a factor of two and five.
    if (last % 2 === 0) {
        const binary = coprime.toString(2);
        let end = binary.length;
        while (binary.charCodeAt(end - 1) === 48) end--;
        twos = BigInt(binary.length - end);
        coprime >>= twos;
    } else if (last === 53) {
        // Remove successively doubled powers, then finish with descending
        // powers. At most O(log written digits) BigInt divisions are needed.
        // Every recorded power is bounded by the written coefficient itself.
        const powers: { power: bigint; count: bigint }[] = [];
        let power = 5n;
        let count = 1n;
        while (coprime % power === 0n) {
            coprime /= power;
            fives += count;
            powers.push({ power, count });
            if (coprime < power) break;
            power *= power;
            count *= 2n;
        }
        for (let i = powers.length - 1; i >= 0; i--) {
            const candidate = powers[i]!;
            if (coprime % candidate.power === 0n) {
                coprime /= candidate.power;
                fives += candidate.count;
            }
        }
    }
    const result = Object.freeze({ coprime, twos, fives });
    state.factors = result;
    return result;
}

/** Exact divisibility by a positive divisor, including symbolic huge exponents. */
function multipleOf(value: Exact, divisor: Exact): boolean {
    const a = exactState(value);
    const b = exactState(divisor);
    if (b.negative || b.digits === '') {
        throw new ExactNumberError('operand', 'multipleOf divisor must be positive');
    }
    if (a.digits === '') return true;
    return exactWork(() => {
        const delta = a.exponent - b.exponent;
        // Both normalized coefficients lack decimal trailing zeroes. A
        // negative delta would require a power of ten to divide a.digits.
        if (delta < 0n) return false;
        const factors = exactFactors(b);
        const missingTwos = factors.twos > delta ? factors.twos - delta : 0n;
        const missingFives = factors.fives > delta ? factors.fives - delta : 0n;
        if (factors.coprime === 1n && missingTwos === 0n && missingFives === 0n) return true;
        // Powers here are bounded by the divisor's written coefficient, never
        // by its decimal exponent or by the exponent difference.
        const remaining = (factors.coprime << missingTwos) * 5n ** missingFives;
        return BigInt(a.digits) % remaining === 0n;
    });
}
