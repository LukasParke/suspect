/**
 * Conformance: TS leaf-function mirrors must agree with the shared fixture
 * corpus, which the Rust side (`suspect-rules::mirrors`) also runs. One
 * file, two runtimes, identical outputs.
 */
import { describe, expect, test } from "bun:test";
import {
  casing,
  defined,
  enumValues,
  isDateTime,
  lengthBetween,
  matches,
  truthy,
} from "../src/functions.ts";

type Case = { fn: string; args: unknown[]; expected: unknown };

const cases: Case[] = await Bun.file(
  new URL("../conformance/cases.json", import.meta.url),
).json();

const fns: Record<string, (...args: unknown[]) => unknown> = {
  casing: (v, style) => casing(v as string, style as never),
  defined,
  truthy,
  lengthBetween: (v, min, max) =>
    lengthBetween(v, min as number, max as number),
  matches: (v, pattern) => matches(v, pattern as string),
  isDateTime,
  enumValues,
};

describe("conformance with suspect-rules::mirrors", () => {
  test(`all ${cases.length} cases agree`, () => {
    for (const c of cases) {
      const fn = fns[c.fn];
      expect(fn).toBeDefined();
      const actual = fn!(...c.args);
      // JSON has no undefined; absent results normalize to null.
      const normalized = actual === undefined ? null : actual;
      expect(normalized).toEqual(c.expected);
    }
  });
});
