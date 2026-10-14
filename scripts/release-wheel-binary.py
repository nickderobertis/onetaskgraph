"""Use maturin's wheel binary as the single payload of the other release forms."""
import argparse
import os
import re
import sys
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 extracted; 1 IO failure; 2 invalid input or wheel contents. Rebuild one maturin wheel for the requested target before retrying.')
parser.add_argument('--wheels', type=Path, required=True)
parser.add_argument('--binary', type=Path, required=True)
args = parser.parse_args()
# Both callers in release.yml run from the checkout root and name dist/wheels/<target>
# and target/<target>/release/<cli> under it; anything else would read a stray wheel or
# write a binary the archive, carrier and wheel forms never read.
checkout = Path.cwd().resolve()
target = args.wheels.name
if not re.fullmatch(r'[a-z0-9_]+(?:-[a-z0-9_]+)+', target) or args.wheels.resolve() != checkout / 'dist' / 'wheels' / target:
    parser.error(f'--wheels must be dist/wheels/<rust target> under the checkout {checkout}, got {args.wheels}; next: run from the checkout root and pass dist/wheels/<target> as release.yml does')
if args.binary.name not in ('onetaskgraph', 'onetaskgraph.exe') or args.binary.resolve() != checkout / 'target' / target / 'release' / args.binary.name:
    parser.error(f'--binary must be target/{target}/release/onetaskgraph[.exe] under the checkout {checkout}, got {args.binary}; next: name the release binary of the same target as --wheels')
try:
    # llmlint: ignore[async_typed_clients_at_boundaries] One synchronous local extraction step inside an ordered release job; there is no service client and no concurrent work.
    wheels = list(args.wheels.glob('*.whl'))
    if len(wheels) != 1:
        parser.error(f'expected one wheel in {args.wheels}, found {len(wheels)}; next: clean that target wheel directory and rebuild it with maturin')
    # llmlint: ignore[async_typed_clients_at_boundaries] One synchronous local extraction step inside an ordered release job; there is no service client and no concurrent work.
    with zipfile.ZipFile(wheels[0]) as wheel:
        scripts = [p for p in wheel.namelist() if '.data/scripts/' in p and Path(p).name in ('onetaskgraph', 'onetaskgraph.exe')]
        if len(scripts) != 1:
            parser.error('wheel must carry exactly one CLI script binary; next: rebuild the CLI with maturin bin bindings')
        payload = wheel.read(scripts[0])
    args.binary.parent.mkdir(parents=True, exist_ok=True)
    args.binary.write_bytes(payload)
    os.chmod(args.binary, 0o755)
except (OSError, zipfile.BadZipFile) as error:
    print(f'wheel extraction failed: {error}; next: rebuild a readable maturin CLI wheel and retry this command', file=sys.stderr)
    sys.exit(1)
