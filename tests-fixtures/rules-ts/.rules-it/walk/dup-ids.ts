
import { defineRule } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "unique-operation-ids", description: "no duplicate ids" },
  visitors: {
    onDocument(_doc, ctx) {
      ctx.state.ids = new Map();
    },
    Operation(op, ctx) {
      const ids = ctx.state.ids;
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
