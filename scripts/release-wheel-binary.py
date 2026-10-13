"""Use maturin's wheel binary as the single payload of the other release forms."""
import argparse
import os
import sys
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__, epilog='Exit codes: 0 extracted; 1 IO failure; 2 invalid input or wheel contents. Rebuild one maturin wheel for the requested target before retrying.')
parser.add_argument('--wheels', type=Path, required=True)
parser.add_argument('--binary', type=Path, required=True)
args = parser.parse_args()
try:
    wheels = list(args.wheels.glob('*.whl'))
    if len(wheels) != 1:
        parser.error(f'expected one wheel in {args.wheels}, found {len(wheels)}; next: clean that target wheel directory and rebuild it with maturin')
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
