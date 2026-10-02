#!/usr/bin/env python3
"""Assemble checked-in license supplements for inclusion alongside Cargo notices."""
import json
import pathlib
import sys
root = pathlib.Path(__file__).resolve().parents[1]
parts = ['Sonora supplemental dependency licenses\n\nSee sources.json for provenance.\n']
for entry in json.loads((root/'vendor/licenses/sources.json').read_text()):
    name = entry['crate']
    parts.extend(['\n' + '='*72 + '\n', name+'\n', entry['source']+'\n\n', (root/'vendor/licenses'/f'{name}.txt').read_text()])
pathlib.Path(sys.argv[1]).write_text(''.join(parts))
