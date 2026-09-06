"""Refresh declared dependency notices from exact Cargo metadata.
This inventories declarations, not legal approval. Only license/notice texts
are copied; no binaries, font files, or user documents are included.
"""
from pathlib import Path
import argparse
import hashlib
import json


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--metadata', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tree', type=Path, required=True)
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding='utf-8-sig'))
    tree = args.tree.read_text(encoding='utf-8-sig')
    active_versions = {(parts[0], parts[1][1:]) for line in tree.splitlines()
                       if len(parts := line.split()) >= 2 and parts[1].startswith('v')}
    if not active_versions:
        raise RuntimeError('Cargo tree inventory is empty')
    destination = args.output.resolve()
    assets = destination / 'licenses'
    assets.mkdir(parents=True, exist_ok=True)
    inventory_path = destination / 'resolved-licenses.json'
    old = json.loads(inventory_path.read_text(encoding='utf-8-sig')) if inventory_path.exists() else []
    previous = {(p['name'], p['version']): p for p in old}
    inventory, missing = [], []
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    pending = [p['id'] for p in metadata['packages'] if p['name'] == 'echo-desktop']
    if len(pending) != 1:
        raise RuntimeError('Expected exactly one desktop composition root')
    active = set()
    while pending:
        identity = pending.pop()
        if identity in active:
            continue
        active.add(identity)
        pending.extend(d['pkg'] for d in nodes[identity]['deps']
                       if any(k['kind'] != 'dev' for k in d['dep_kinds']))
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['id'] not in active or (package['name'], package['version']) not in active_versions:
            continue
        if package['name'].startswith('echo-'):
            continue
        source = Path(package['manifest_path']).resolve().parent
        candidates = []
        for path in source.iterdir():
            if path.is_file() and path.name.lower().startswith(('license', 'copying', 'notice', 'copyright')):
                candidates.append(path)
            elif path.is_dir() and path.name.lower() in ('license', 'licenses'):
                candidates.extend(p for p in path.rglob('*') if p.is_file() and p.suffix.lower() in ('', '.txt', '.md'))
        declared = package.get('license_file')
        if declared:
            path = (source / declared).resolve()
            if path.is_relative_to(source) and path.is_file():
                candidates.append(path)
        references = set()
        for path in candidates:
            data = path.read_bytes()
            if len(data) > 2 * 1024 * 1024 or b'\x00' in data:
                continue
            digest = hashlib.sha256(data).hexdigest()
            target = assets / (digest + '.txt')
            if target.exists() and target.read_bytes() != data:
                raise RuntimeError('Content-addressed license collision')
            target.write_bytes(data)
            references.add('licenses/' + target.name)
        prior = previous.get((package['name'], package['version']), {})
        for reference in prior.get('license_files', []):
            path = destination / reference
            if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == path.stem:
                references.add(reference)
        if not references:
            missing.append(package['name'] + '@' + package['version'])
        inventory.append(dict(name=package['name'], version=package['version'],
                              license=package.get('license') or prior.get('license', 'UNDECLARED'),
                              source=package.get('repository') or package.get('source') or '',
                              license_files=sorted(references)))
    inventory_path.write_text(json.dumps(inventory, indent=2) + '\n', encoding='utf-8')
    lines = ['Echo - third-party dependency notices', '',
             'Inventory: Cargo metadata for x86_64-pc-windows-msvc with default features.',
             'Includes the default Windows desktop runtime and its build dependencies; excludes dev-only and inactive optional dependencies.',
             'SPDX expressions are upstream declarations, not a legal determination.',
             'The exact included texts are in the content-addressed licenses directory.', '',
             'Slint 1.17.1 offers alternative licensing terms. Distribution must comply',
             'with an applicable upstream license or a separately obtained agreement.', '']
    for package in inventory:
        lines += [package['name'] + ' ' + package['version'],
                  '  Declared: ' + package['license'], '  Source: ' + package['source']]
        lines += ['  Text: ' + name for name in package['license_files']]
        if not package['license_files']:
            lines.append('  No standalone text shipped in the package; see upstream source.')
        lines.append('')
    (destination / 'THIRD-PARTY-NOTICES.txt').write_text('\n'.join(lines), encoding='utf-8')
    print(json.dumps(dict(packages=len(inventory), standalone_text_not_shipped=missing)))


if __name__ == '__main__':
    main()
