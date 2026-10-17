import { readFileSync } from "node:fs";

// Job outcomes are the GitHub boundary. Missing or cancelled proofs never become green.
try {
  if (process.argv.slice(2).length !== 1)
    throw Error(
      "usage: macos-result.mjs NEEDS_JSON; next: pass one readable job-outcome JSON file",
    );
  const needs = JSON.parse(readFileSync(process.argv[2], "utf8"));
  if (needs["macos-lanes"]?.result !== "success")
    throw Error("macOS classification failed or is missing");
  const diagnostic = needs["macos-lanes"].outputs?.diagnostic;
  if (diagnostic !== undefined && !["true", "false"].includes(diagnostic))
    throw Error("Malformed macOS diagnostic output");
  const required =
    needs["macos-lanes"].outputs?.diagnostic === "true"
      ? "true"
      : needs["macos-lanes"].outputs?.macosRequired;
  // The proof is the pull request's `test-macos`, or the manual diagnostic's `macos`.
  const result = (diagnostic === "true" ? needs.macos : needs["test-macos"])?.result;
  if (required === "true" && result === "success")
    console.log("The combined macOS check and install proofs passed.");
  else if (required === "false" && result === "skipped") {
    const reason = needs["macos-lanes"].outputs.reason;
    if (typeof reason !== "string" || !reason.trim() || /[\r\n]/.test(reason))
      throw Error("Missing or malformed authorized omission reason");
    console.log(`macOS omitted: ${reason}`);
  } else throw Error(`Required macOS work did not succeed: required=${required}, result=${result}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
