import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "node:test";
import { workflows } from "../workflow-graph.mjs";
import { history } from "./policy-fixture.mjs";

test("actual carrier run bodies preserve success, failure, cancellation, missing and authorized omission", () => {
  const ci = workflows()["ci.yml"];
  for (const id of ["macos-check", "macos-install"]) {
    const job = ci.jobs[id];
    assert.equal(job.if, `\${{ always() }}`);
    const run = job.steps.find((s) => s.run)?.run;
    assert.match(run, /macos-result\.mjs/);
    for (const diagnostic of [null, false, "invalid", {}]) {
      const needs = {
        "macos-lanes": { result: "success", outputs: { macosRequired: "false", diagnostic } },
        macos: { result: "skipped" },
        "test-macos": { result: "skipped" },
      };
      assert.notEqual(
        spawnSync("bash", ["-c", run], {
          cwd: history,
          encoding: "utf8",
          env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
        }).status,
        0,
      );
    }
    for (const diagnostic of ["true", "false"])
      for (const result of ["success", "skipped", "cancelled", "failure"]) {
        const needs = {
          "macos-lanes": {
            result: "success",
            outputs: { macosRequired: "false", diagnostic, reason: "manual" },
          },
          // The diagnostic reads `macos` alone, a pull request `test-macos` alone.
          macos: { result: diagnostic === "true" ? result : "failure" },
          "test-macos": { result: diagnostic === "true" ? "failure" : result },
        };
        const outcome = spawnSync("bash", ["-c", run], {
          cwd: history,
          encoding: "utf8",
          env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
        });
        assert.equal(
          outcome.status === 0,
          diagnostic === "true" ? result === "success" : result === "skipped",
        );
      }
    for (const classification of ["success", "failure", "cancelled", "skipped", undefined])
      for (const required of ["true", "false", undefined])
        for (const result of ["success", "failure", "cancelled", "skipped", undefined]) {
          const needs = {
            "macos-lanes": {
              result: classification,
              outputs: { macosRequired: required, reason: "semantic release omission" },
            },
            macos: { result: "failure" },
            "test-macos": { result },
          };
          const outcome = spawnSync("bash", ["-c", run], {
            cwd: history,
            encoding: "utf8",
            env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
          });
          const allowed =
            classification === "success" &&
            ((required === "true" && result === "success") ||
              (required === "false" && result === "skipped"));
          assert.equal(outcome.status === 0, allowed, JSON.stringify(needs));
          if (allowed && required === "false") {
            assert.match(outcome.stdout, /omitted/);
            assert.doesNotMatch(outcome.stdout, /proofs passed/);
          }
        }
  }
  rmSync(resolve(history, "needs.json"), { force: true });
});
