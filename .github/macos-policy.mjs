import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function classify(event, base, head) {
  // Actions' event payload has no event_name; a PR payload itself identifies the event.
  const pr = event?.pull_request;
  if (!pr)
    return {
      isReleaseOnly: false,
      macosRequired: true,
      reason: "Unknown PR input requires coverage",
    };
  const validRepository = (value) =>
    typeof value === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value);
  if (!validRepository(pr.head?.repo?.full_name) || !validRepository(pr.base?.repo?.full_name)) {
    return {
      isReleaseOnly: false,
      macosRequired: true,
      reason: "Unreadable PR ownership requires coverage",
    };
  }
  if (
    pr.head.repo.full_name !== pr.base.repo.full_name ||
    typeof pr.head.ref !== "string" ||
    !pr.head.ref.startsWith("release-plz-")
  ) {
    return { isReleaseOnly: false, macosRequired: true, reason: "Ordinary pull request" };
  }
  try {
    const isReleaseOnly = JSON.parse(
      execFileSync(
        "python3",
        [fileURLToPath(new URL("./semantic-release.py", import.meta.url)), base, head],
        { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
      ),
    );
    if (typeof isReleaseOnly !== "boolean") throw Error("Invalid semantic classifier result");
    return {
      isReleaseOnly,
      macosRequired: !isReleaseOnly,
      reason: isReleaseOnly
        ? "Same-repository release-plz branch with semantic version/changelog-only diff"
        : "Release branch changes non-version content",
    };
  } catch {
    return {
      isReleaseOnly: false,
      macosRequired: true,
      reason: "Unreadable release diff requires coverage",
    };
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (
      args[0] !== "classify" ||
      args.length !== 7 ||
      args[1] !== "--event" ||
      args[3] !== "--base" ||
      args[5] !== "--head"
    )
      throw Error("usage: classify --event JSON --base REF --head REF");
    let event;
    try {
      event = JSON.parse(readFileSync(args[2], "utf8"));
    } catch {
      event = null;
    }
    // Workflow supplies the authoritative event kind; offline fixtures use the same field.
    const knownEvents = new Set([
      "pull_request",
      "push",
      "release",
      "workflow_run",
      "repository_dispatch",
      "schedule",
      "workflow_dispatch",
      "merge_group",
    ]);
    const suppliedEvent = event?.event_name;
    const eventName =
      process.env.GITHUB_EVENT_NAME ||
      (typeof suppliedEvent === "string" && suppliedEvent ? suppliedEvent : "pull_request");
    const result = !knownEvents.has(eventName)
      ? {
          isReleaseOnly: false,
          macosRequired: true,
          reason: `Unknown event kind ${JSON.stringify(eventName)} requires coverage`,
        }
      : eventName === "pull_request"
        ? classify(event, args[4], args[6])
        : {
            isReleaseOnly: false,
            macosRequired: false,
            reason: "Automatic non-PR events never request macOS",
          };
    console.log(JSON.stringify(result));
    if (process.env.GITHUB_OUTPUT)
      appendFileSync(
        process.env.GITHUB_OUTPUT,
        `macosRequired=${result.macosRequired}\nreason=${result.reason}\n`,
      );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
