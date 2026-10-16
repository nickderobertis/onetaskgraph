import { registerHooks } from "node:module";

// The published evaluator emits extensionless ESM imports. Resolve those solely inside
// that package so the required Node entrypoint can use its unmodified published code.
registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (error) {
      if (
        !["ERR_MODULE_NOT_FOUND", "ERR_UNSUPPORTED_DIR_IMPORT"].includes(error.code) ||
        !context.parentURL?.includes("/@actions/expressions/") ||
        !specifier.startsWith(".")
      )
        throw error;
      try {
        return nextResolve(`${specifier}.js`, context);
      } catch {
        return nextResolve(`${specifier}/index.js`, context);
      }
    }
  },
});
export const { Lexer, Parser, Evaluator, data } = await import("@actions/expressions");
