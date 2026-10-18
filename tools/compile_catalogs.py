"""Prepare MO input without modifying the upstream checkout. Requires Babel (tool only)."""
import argparse
from pathlib import Path
from babel.messages.pofile import read_po
from babel.messages.mofile import write_mo

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True, help='Directory containing upstream gettext PO catalogs')
parser.add_argument('--output', type=Path, default=Path('artifacts/catalogs'))
args = parser.parse_args()
catalogs = sorted(args.source.glob('*.po'))
if not catalogs:
    parser.error(f'No .po catalogs found in {args.source}')
args.output.mkdir(parents=True, exist_ok=True)
for path in catalogs:
    with path.open('rb') as source, (args.output / (path.stem + '.mo')).open('wb') as output:
        write_mo(output, read_po(source, locale=path.stem), use_fuzzy=False)
print(f'Compiled catalogues in {args.output}')
