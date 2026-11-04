import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// Read the pinned production script's literal selector declaration. Unknown syntax fails
// closed instead of letting a second inventory certify files the runtime no longer reads.
export function canonicalInputNames(workspace = fileURLToPath(new URL("../../", import.meta.url))) {
  const source = readFileSync(join(workspace, "scripts/nx-version-free-inputs.mjs"), "utf8");
  const declarations = [
    ...source.matchAll(/const selectedNames = \{([\s\S]*?)\n\}\[ecosystem\];/g),
  ];
  assert.equal(declarations.length, 1, "one canonicalizer selector declaration");
  const body = declarations[0][1];
  const entries = [...body.matchAll(/\b([a-z]+):\s*\[([^\]]*)\],?/g)];
  assert.equal(
    body.replace(/\b([a-z]+):\s*\[([^\]]*)\],?/g, "").trim(),
    "",
    "literal ecosystem selectors",
  );
  assert.ok(entries.length > 0);
  return Object.fromEntries(
    entries.map(([, ecosystem, values]) => {
      const names = [...values.matchAll(/'([A-Za-z0-9._-]+)'/g)].map((match) => match[1]);
      assert.ok(
        names.length > 0 && values.replace(/'[A-Za-z0-9._-]+'/g, "").replace(/[\s,]/g, "") === "",
        `${ecosystem}: literal manifest basenames`,
      );
      return [ecosystem, names];
    }),
  );
}
