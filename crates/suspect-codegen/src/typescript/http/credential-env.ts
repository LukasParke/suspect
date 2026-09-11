import { objectData } from './common.js';

/** Generation supplies variable names only; this module is emitted only for an opted-in policy. */
export interface CredentialEnvBinding { readonly name: string; readonly variable: string }

/** Snapshot environment defaults once, before the existing client snapshot is created. */
export function withCredentialEnv<T extends object>(options: T, bindings: readonly CredentialEnvBinding[]): T {
    // objectData intentionally omits undefined values. Decide credential
    // precedence from the original argument so auth:undefined is still explicit.
    const explicit = Object.prototype.hasOwnProperty.call(options, 'auth');
    const supplied = objectData(options) as T;
    if (explicit) return options;

    let environment: Record<string, unknown> | undefined;
    try {
        const host = (globalThis as { process?: { env?: unknown } }).process;
        const value = host?.env;
        if (value !== null && typeof value === 'object') environment = value as Record<string, unknown>;
    } catch { /* Unavailable host environment means no defaults. */ }

    const values = new Map<string, string | undefined>();
    const auth: Record<string, string> = Object.create(null);
    for (const binding of bindings) {
        if (!values.has(binding.variable)) {
            let value: unknown;
            try { value = environment?.[binding.variable]; }
            catch { /* One unavailable variable must not disable other alternatives. */ }
            values.set(binding.variable, typeof value === 'string' && value !== '' ? value : undefined);
        }
        const value = values.get(binding.variable);
        if (value !== undefined) auth[binding.name] = value;
    }
    return { ...supplied, auth };
}
