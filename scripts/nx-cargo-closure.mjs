// The Cargo files one crate's tasks really read, as one digest for an Nx runtime input.
//
//   node scripts/nx-cargo-closure.mjs --crate NAME [--crate NAME]...
//
// Nx's `versionFreeCargo` hashes every manifest and all of Cargo.lock, so a dependency added to
// one suite moved the hash of every Rust task in the workspace, and a pull request's proven
// results missed when another merge landed. A build of `-p NAME` reads the root manifest, and of
// the rest only the packages Cargo.lock reaches from NAME: a lock entry lists its normal, build
// and dev dependencies across every feature and target, so this closure is never narrower than
// what cargo builds. A workspace crate's own version is blanked wherever it is written, as
// scripts/nx-version-free-inputs.mjs blanks it, so a release moves nothing. Anything unreadable
// or unknown hashes as itself and says so on stderr, so a surprise over-runs a task, never skips it.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const MEMBER = /^crates\/[^/]+\/Cargo\.toml$/;

function requested(args) {
  const crates = [];
  for (let at = 0; at < args.length; at += 2) {
    if (args[at] !== "--crate" || !args[at + 1]) return null;
    crates.push(args[at + 1]);
  }
  return crates.length ? crates : null;
}

// The lock is generated, so these line shapes are the whole of its grammar here.
function lockPackages(text) {
  const packages = new Map();
  for (const block of text.split("[[package]]\n").slice(1)) {
    const name = block.match(/^name = "([^"]+)"$/m)?.[1];
    const version = block.match(/^version = "([^"]+)"$/m)?.[1];
    if (!name || !version) throw new Error(`Cargo.lock holds a package with no name or version`);
    const dependencies = [
      ...(block.match(/^dependencies = \[\n([\s\S]*?)^\]$/m)?.[1] ?? "").matchAll(/"([^"]+)"/g),
    ].map((match) => match[1]);
    packages.set(`${name} ${version}`, { name, version, block, dependencies });
  }
  return packages;
}

// A lock dependency is `name` when one version is locked, else `name version` or more.
function resolver(packages) {
  const versions = new Map();
  for (const entry of packages.values())
    versions.set(entry.name, [...(versions.get(entry.name) ?? []), entry]);
  return (dependency) => {
    const [name, version] = dependency.split(" ");
    const found = version ? packages.get(`${name} ${version}`) : versions.get(name)?.[0];
    if (!found) throw new Error(`Cargo.lock names ${dependency}, which it does not lock`);
    return found;
  };
}

function closure(crates, packages, members) {
  const resolve = resolver(packages);
  const pending = crates.map((crate) => {
    if (!members.has(crate)) throw new Error(`${crate} is no workspace crate`);
    return resolve(crate);
  });
  const reached = new Map();
  while (pending.length) {
    const entry = pending.pop();
    if (reached.has(`${entry.name} ${entry.version}`)) continue;
    reached.set(`${entry.name} ${entry.version}`, entry);
    pending.push(...entry.dependencies.map(resolve));
  }
  return [...reached.values()].sort((a, b) =>
    `${a.name} ${a.version}` < `${b.name} ${b.version}` ? -1 : 1,
  );
}

// A manifest with each workspace crate's version blanked: the package's own, the workspace's,
// and any written beside a `path`, which is how a sibling is required.
function versionFree(text) {
  let table = "";
  return text
    .split("\n")
    .map((line) => {
      table = line.match(/^\s*\[+([^\]]+)\]+/)?.[1]?.trim() ?? table;
      if (["package", "workspace.package"].includes(table))
        return line.replace(/^(\s*version\s*=\s*)"[^"]*"/, '$1""');
      return /\bpath\s*=/.test(line)
        ? line.replace(/\bversion\s*=\s*"[^"]*"/, 'version = ""')
        : line;
    })
    .join("\n");
}

function digest(parts) {
  const hash = createHash("sha256");
  for (const [name, body] of parts) hash.update(name).update("\0").update(body).update("\0");
  return hash.digest("hex");
}

const crates = requested(process.argv.slice(2));
if (!crates) {
  console.error("usage: node scripts/nx-cargo-closure.mjs --crate NAME [--crate NAME]...");
  process.exit(2);
}
try {
  const tracked = execFileSync(
    "git",
    ["ls-files", "-z", "--", "Cargo.toml", "Cargo.lock", "crates"],
    {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    },
  ).split("\0");
  const members = new Map();
  for (const path of tracked.filter((file) => MEMBER.test(file))) {
    const name = readFileSync(path, "utf8").match(/^name\s*=\s*"([^"]+)"/m)?.[1];
    if (!name) throw new Error(`${path} names no package`);
    members.set(name, path);
  }
  const packages = lockPackages(readFileSync("Cargo.lock", "utf8"));
  const parts = [["Cargo.toml", versionFree(readFileSync("Cargo.toml", "utf8"))]];
  for (const entry of closure(crates, packages, members)) {
    const path = members.get(entry.name);
    // A workspace crate's lock entry differs from a release to the next by its version alone.
    if (path && !entry.block.includes("source = "))
      parts.push(
        [entry.name, entry.block.replace(/^version = "[^"]*"$/m, 'version = ""')],
        [path, versionFree(readFileSync(path, "utf8"))],
      );
    else parts.push([`${entry.name} ${entry.version}`, entry.block]);
  }
  console.log(digest(parts));
} catch (error) {
  // Every tracked Cargo file, raw: wider than any closure, so nothing reading one goes unhashed.
  console.error(`nx-cargo-closure: ${error.message}; hashing every tracked Cargo file raw`);
  let tracked = [];
  try {
    tracked = execFileSync("git", ["ls-files", "-z"], {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    })
      .split("\0")
      .filter((file) => /(^|\/)Cargo\.(toml|lock)$/.test(file));
  } catch {}
  const unreadable = (file) => {
    try {
      return readFileSync(file);
    } catch (cause) {
      return `unreadable:${cause.code}`;
    }
  };
  console.log(digest(tracked.sort().map((file) => [file, unreadable(file)])));
}
