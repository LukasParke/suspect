/**
 * SDK behavior tests: point rules, selector filtering, walk rules, ctx
 * surface — the same paths the worker exercises in production.
 */
import { describe, expect, test } from "bun:test";
import { defineRule, r } from "../src/sdk.ts";
import { testRule, type CapturedFinding } from "../src/testing.ts";

const petstore = {
  openapi: "3.1.0",
  info: { title: "T", version: "1" },
  paths: {
    "/pets": {
      get: { operationId: "listPets", responses: { "200": { description: "ok" } } },
      post: { responses: {} },
    },
    "/pets/{id}": {
      // Not an operation: path-item-level parameters must be skipped by
      // operation selectors.
      parameters: [{ name: "id", in: "path" }],
      get: { operationId: "showPet", summary: "Info", responses: {} },
      delete: { summary: "Remove", responses: {} },
    },
  },
  components: {
    schemas: {
      Pet: { type: "object" },
    },
  },
};

describe("point rules", () => {
  const rule = defineRule({
    meta: { id: "operation-summary", description: "ops need summary" },
    given: r.operation,
    check(op, ctx) {
      if (!op.summary) {
        ctx.report({
          message: `Operation ${op.method.toUpperCase()} ${op.path} is missing a summary`,
          at: op,
        });
      }
    },
  });

  test("finds operations missing summary, skips non-method path-item keys", async () => {
    const found: CapturedFinding[] = [];
    await testRule(rule, petstore, (f) => found.push(f));
    // Flagged: both /pets operations lack summaries. Not flagged: the
    // path-item-level `parameters` key (not an operation), and the two
    // operations that have summaries.
    expect(found.map((f) => f.pointer).sort()).toEqual([
      "/paths/~1pets/get",
      "/paths/~1pets/post",
    ].sort());
    expect(found[0]?.message).toContain("missing a summary");
  });

  test("field-level `at` targets the sub-pointer", async () => {
    const rule2 = defineRule({
      meta: { id: "summary-length", description: "short summaries" },
      given: r.operation,
      check(op, ctx) {
        if (op.summary && op.summary.length > 3) {
          ctx.report({ message: "too long", at: { node: op, field: "summary" } });
        }
      },
    });
    const found: CapturedFinding[] = [];
    await testRule(rule2, petstore, (f) => found.push(f));
    expect(found.map((f) => f.pointer)).toContain(
      "/paths/~1pets~1{id}/get/summary",
    );
  });

  test("explicit node pointers override selection (custom raw selectors)", async () => {
    const found: CapturedFinding[] = [];
    await testRule(
      rule,
      petstore,
      (f) => found.push(f),
      { nodes: ["/paths/~1pets/get", "/components/schemas/Pet"] },
    );
    // The schema pointer is filtered out (not an operation); the get
    // operation legitimately lacks a summary.
    expect(found.map((f) => f.pointer)).toEqual(["/paths/~1pets/get"]);
  });
});

describe("walk rules", () => {
  test("visitors see typed nodes and shared state", async () => {
    const rule = defineRule({
      meta: { id: "unique-operation-ids", description: "no dup ids" },
      visitors: {
        onDocument(_doc, ctx) {
          ctx.state.ids = new Map<string, string>();
        },
        Operation(op, ctx) {
          const ids = ctx.state.ids as Map<string, string>;
          if (typeof op.operationId === "string") {
            if (ids.has(op.operationId)) {
              ctx.report({
                message: `duplicate operationId ${op.operationId}`,
                at: { pointer: op.pointer },
              });
            }
            ids.set(op.operationId, op.pointer);
          }
        },
      },
    });
    const dup = {
      paths: {
        "/a": { get: { operationId: "same" } },
        "/b": { get: { operationId: "same" } },
      },
    };
    const found: CapturedFinding[] = [];
    await testRule(rule, dup, (f) => found.push(f));
    expect(found).toHaveLength(1);
    expect(found[0]?.pointer).toBe("/paths/~1b/get");
  });
});

describe("leaf functions re-exported from the SDK", () => {
  test("casing is available to rule authors", async () => {
    const rule = defineRule({
      meta: { id: "camel-ids", description: "operationId casing" },
      given: r.operation,
      check(op, ctx) {
        if (op.operationId && !/^[a-z][a-zA-Z0-9]*$/.test(op.operationId)) {
          ctx.report({ message: "operationId must be camelCase", at: op });
        }
      },
    });
    const found: CapturedFinding[] = [];
    await testRule(rule, petstore, (f) => found.push(f));
    expect(found).toHaveLength(0);
  });
});
