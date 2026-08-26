
import { defineRule, r } from "@suspect/rules-sdk";

export default defineRule({
  meta: { id: "operation-summary", description: "ops need summaries" },
  given: r.operation,
  check(op, ctx) {
    if (!op.summary) {
      ctx.report({
        message: `${op.method.toUpperCase()} ${op.path} is missing a summary`,
        at: op,
      });
    }
  },
});
