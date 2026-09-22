// A compiled Unicode-scalar automaton. Source regex syntax is interpreted once
// by the Rust compiler; generated runtimes execute these bounded instructions.
type PatternState =
    | { readonly op: 'match' }
    | { readonly op: 'char'; readonly ranges: readonly (readonly [number, number])[]; readonly target: number }
    | { readonly op: 'split'; readonly first: number; readonly second: number }
    | { readonly op: 'jump' | 'start' | 'end'; readonly target: number };

/** @internal Portable finite automaton for the admitted ECMA Unicode subset. */
export interface CompiledPattern {
    readonly version: 'suspect.pattern.experimental.v1';
    readonly start: number;
    readonly states: readonly PatternState[];
}

/** @internal Admission precedes allocation and execution of a compiled graph. */
export function checkCompiledPattern(program: CompiledPattern): void {
    if (program.version !== 'suspect.pattern.experimental.v1'
        || program.states.length === 0 || program.states.length > 8192) throw new Error('unsupported compiled pattern version or state count');
    const target = (index: number): void => {
        if (!Number.isSafeInteger(index) || index < 0 || index >= program.states.length) throw new Error('invalid compiled pattern target');
    };
    target(program.start);
    let rangeCount = 0;
    for (const state of program.states) {
        if (state.op === 'char') {
            rangeCount += state.ranges.length;
            if (rangeCount > 65536) throw new Error('compiled pattern exceeds aggregate range limit');
        }
    }
    for (const state of program.states) {
        switch (state.op) {
            case 'match': break;
            case 'jump': case 'start': case 'end': target(state.target); break;
            case 'split': target(state.first); target(state.second); break;
            case 'char': {
                target(state.target);
                if (state.ranges.length > 8192) throw new Error('compiled pattern exceeds range limit');
                let previous = -2;
                for (const [start, end] of state.ranges) {
                    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end)
                        || start < 0 || end > 0x10ffff || start > end
                        || start <= previous + 1 || start <= 0xdfff && end >= 0xd800) throw new Error('invalid compiled Unicode scalar range');
                    previous = end;
                }
                break;
            }
            default: throw new Error('unknown compiled pattern instruction');
        }
    }
}

/**
 * @internal Search without backtracking. Each input boundary, attempted queue
 * insertion, processed state and range comparison spends the caller's shared
 * evaluation allowance. Epsilon cycles never revisit a state at one boundary.
 */
export function matchesCompiledPattern(program: CompiledPattern, text: string, spend: () => void): boolean {
    const seen = new Uint32Array(program.states.length);
    const pending: number[] = [];
    const active: number[] = [];
    let seeds: number[] = [];
    let next: number[] = [];
    let epoch = 0;
    let offset = 0;
    const enqueue = (target: number): void => {
        spend();
        if (seen[target] !== epoch) {
            seen[target] = epoch;
            pending.push(target);
        }
    };
    while (true) {
        spend();
        epoch++;
        if (epoch > 0xffffffff) {
            // Preserve deduplication even on engines permitting enormous strings.
            for (let index = 0; index < seen.length; index++) { spend(); seen[index] = 0; }
            epoch = 1;
        }
        const scalar = offset < text.length ? text.codePointAt(offset)! : undefined;
        if (scalar !== undefined && scalar >= 0xd800 && scalar <= 0xdfff) throw new Error('pattern input contains an unpaired UTF-16 surrogate');
        pending.length = 0;
        active.length = 0;
        enqueue(program.start);
        for (const target of seeds) enqueue(target);
        while (pending.length !== 0) {
            const index = pending.pop()!;
            spend();
            const state = program.states[index]!;
            switch (state.op) {
                case 'match': return true;
                case 'char': active.push(index); break;
                case 'jump': enqueue(state.target); break;
                case 'split': enqueue(state.second); enqueue(state.first); break;
                case 'start': if (offset === 0) enqueue(state.target); break;
                case 'end': if (scalar === undefined) enqueue(state.target); break;
                default: throw new Error('unknown compiled pattern instruction');
            }
        }
        if (scalar === undefined) return false;
        next.length = 0;
        for (const index of active) {
            const state = program.states[index]!;
            if (state.op !== 'char') throw new Error('invalid active compiled pattern instruction');
            for (const [start, end] of state.ranges) {
                spend();
                if (scalar < start) break;
                if (scalar <= end) { next.push(state.target); break; }
            }
        }
        [seeds, next] = [next, seeds];
        offset += scalar > 0xffff ? 2 : 1;
    }
}
