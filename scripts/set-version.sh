#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
binary_manifest=crates/onetaskgraph/Cargo.toml
current=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$binary_manifest" | head -n1)

if [[ ${1:-} == --check ]]; then
  expected=$current
else
  expected=${1:?usage: scripts/set-version.sh VERSION | --check}
  [[ $expected =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || {
    echo "invalid semantic version: $expected" >&2; exit 2;
  }
fi

fail=0
check_value() { [[ $1 == "$expected" ]] || { echo "$2 has $1; expected $expected" >&2; fail=1; }; }

if [[ ${1:-} == --check ]]; then
  check_value "$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n1)" Cargo.toml
  for manifest in crates/*/Cargo.toml; do
    value=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest" | head -n1)
    check_value "$value" "$manifest"
  done
  while IFS= read -r value; do check_value "$value" 'Cargo.toml path dependency'; done < <(sed -n 's/.*path = "crates\/[^" ]*", version = "\([^"]*\)".*/\1/p' Cargo.toml)
  check_value "$(sed -n 's/^version = "\([^"]*\)"/\1/p' pyproject.toml | head -n1)" pyproject.toml
  check_value "$(sed -n 's/^version = "\([^"]*\)"/\1/p' sdks/python/pyproject.toml | head -n1)" sdks/python/pyproject.toml
  check_value "$(sed -n 's/.*onetaskgraph-cli==\([^" ]*\).*/\1/p' sdks/python/pyproject.toml)" 'Python SDK CLI pin'
  node -e 'const fs=require("fs"); const v=process.argv[1]; for (const f of ["npm/cli/package.json",...fs.readdirSync("npm/platforms").map(x=>`npm/platforms/${x}/package.json`),"sdks/typescript/package.json"]) { const p=JSON.parse(fs.readFileSync(f)); if(p.version!==v) throw Error(`${f} has ${p.version}; expected ${v}`); for(const [n,x] of Object.entries(p.optionalDependencies||{})) if(x!==v) throw Error(`${f} ${n} pin has ${x}; expected ${v}`) }' "$expected" || fail=1
  exit "$fail"
fi

perl -pi -e 's/^version = "[^"]+"/version = "'$expected'"/' Cargo.toml crates/*/Cargo.toml pyproject.toml sdks/python/pyproject.toml
perl -pi -e 's/(path = "crates\/[^" ]+", version = ")[^"]+/${1}'$expected'/g' Cargo.toml
perl -pi -e 's/(onetaskgraph-cli==)[^" ]+/${1}'$expected'/' sdks/python/pyproject.toml
for manifest in npm/cli/package.json npm/platforms/*/package.json sdks/typescript/package.json; do
  node -e 'const fs=require("fs"),f=process.argv[1],v=process.argv[2],p=JSON.parse(fs.readFileSync(f)); p.version=v; for(const n of Object.keys(p.optionalDependencies||{}))p.optionalDependencies[n]=v; fs.writeFileSync(f,JSON.stringify(p,null,2)+"\n")' "$manifest" "$expected"
done
cargo generate-lockfile --quiet
uv lock --project . --quiet
uv lock --project sdks/python --quiet
