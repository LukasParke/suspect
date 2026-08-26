/**
 * Leaf rule functions — TS mirrors of the Rust implementations in
 * `suspect-rules::mirrors`. Both sides are gated by the shared conformance
 * suite (`conformance/cases.json`): identical inputs must produce identical
 * outputs or CI fails. Pure functions only — no state, no IO.
 */

export type Casing =
  | "camel"
  | "pascal"
  | "kebab"
  | "snake"
  | "macro"
  | "cobol"
  | "dot";

/** True when `s` is empty or entirely separator characters. */
function words(s: string): string[] {
  return s
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z])([A-Z][a-z])/g, "$1 $2")
    .split(/[^a-zA-Z0-9]+/)
    .filter((w) => w.length > 0);
}

/** `casing(s, "camel")` and friends; mirrors `mirrors::casing`. */
export function casing(s: string, style: Casing): boolean {
  const ws = words(s);
  if (ws.length === 0) return true;
  switch (style) {
    case "camel":
      return (
        /^[a-z][a-zA-Z0-9]*$/.test(s) &&
        ws.join("") === s &&
        ws[0] === ws[0].toLowerCase()
      );
    case "pascal":
      return /^[A-Z][a-zA-Z0-9]*$/.test(s) && ws.join("") === s;
    case "kebab":
      return /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/.test(s) && ws.join("-") === s;
    case "snake":
      return /^[a-z][a-z0-9]*(_[a-z0-9]+)*$/.test(s) && ws.join("_") === s;
    case "macro":
      return /^[A-Z][A-Z0-9]*(_[A-Z0-9]+)*$/.test(s) && ws.join("_") === s;
    case "cobol":
      return /^[A-Z][A-Z0-9]*(-[A-Z0-9]+)*$/.test(s) && ws.join("-") === s;
    case "dot":
      return /^[a-z][a-z0-9]*(\.[a-z0-9]+)*$/.test(s) && ws.join(".") === s;
  }
}

/** `defined(v)`: not undefined and not null. */
export function defined(v: unknown): boolean {
  return v !== undefined && v !== null;
}

/** `truthy(v)`: JS truthiness with explicit falsy list. */
export function truthy(v: unknown): boolean {
  return Boolean(v);
}

/** `lengthBetween(v, min, max)`: string or array length in range. */
export function lengthBetween(
  v: unknown,
  min: number,
  max: number,
): boolean {
  if (typeof v === "string") return v.length >= min && v.length <= max;
  if (Array.isArray(v)) return v.length >= min && v.length <= max;
  return false;
}

/** `matches(v, pattern)`: regex full-source test on strings. */
export function matches(v: unknown, pattern: string): boolean {
  if (typeof v !== "string") return false;
  try {
    return new RegExp(pattern).test(v);
  } catch {
    return false;
  }
}

const ISO_DATE =
  /^(\d{4})-(\d{2})-(\d{2})([Tt](\d{2}):(\d{2}):(\d{2})(\.\d+)?([Zz]|[+-]\d{2}:\d{2})?)?$/;

/** `isDateTime(v)`: ISO 8601 date or date-time (RFC 3339 shape). */
export function isDateTime(v: unknown): boolean {
  if (typeof v !== "string") return false;
  const m = ISO_DATE.exec(v);
  if (!m) return false;
  const month = Number(m[2]);
  const day = Number(m[3]);
  return month >= 1 && month <= 12 && day >= 1 && day <= 31;
}

/** `enumValues(v)`: the enum array of a schema node, or undefined. */
export function enumValues(v: unknown): unknown[] | undefined {
  if (v && typeof v === "object" && !Array.isArray(v)) {
    const e = (v as Record<string, unknown>)["enum"];
    if (Array.isArray(e)) return e;
  }
  return undefined;
}

/** `falsy` list used by `truthy` documentation and conformance cases. */
export const FALSY = [false, 0, "", null, undefined, NaN];
