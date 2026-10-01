#!/usr/bin/env python3
"""Refresh, verify and package the Linux release's retained provenance records.

Refresh is an explicit maintainer operation. Validation never invents or downloads
license grants. Missing upstream notices must be supplied from pinned sources.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib

sys.dont_write_bytecode = True

SUPPORTED_TARGETS = ('aarch64-unknown-linux-gnu', 'x86_64-unknown-linux-gnu')
def toolchain():
    text = subprocess.run(['rustc', '-Vv'], check=True, capture_output=True, text=True).stdout
    fields = dict(line.split(': ', 1) for line in text.splitlines() if ': ' in line)
    if fields['host'] not in SUPPORTED_TARGETS:
        raise ManifestError('license packaging currently supports native Linux aarch64/x86_64 only')
    return fields
LICENSE_ROOT = '.local/share/omatainer/licenses'
RECORD_FILES = ('manifest.json', 'notices.json')
INSTALLED = {f'{LICENSE_ROOT}/{name}': f'licenses/{name}' for name in RECORD_FILES}
RECEIPT = f'{LICENSE_ROOT}/release.json'

class ManifestError(ValueError):
    pass

def sha(data):
    return hashlib.sha256(data).hexdigest()

def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + '\n').encode()

def load(path):
    return json.loads(path.read_text())

def relative(name):
    path = Path(name)
    if path.is_absolute() or not path.parts or any(part in ('.', '..') for part in path.parts):
        raise ManifestError(f'unsafe release path: {name}')
    return path

def regular(path):
    if path.is_symlink() or not path.is_file():
        raise ManifestError(f'expected regular release file: {path}')
    return path.read_bytes()

def metadata(root):
    result = subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
                             '--filter-platform', toolchain()['host'], '--manifest-path', str(root/'Cargo.toml')],
                            check=True, capture_output=True)
    return json.loads(result.stdout)

def graph(meta, root):
    lock = tomllib.loads((root/'Cargo.lock').read_text())
    checksums = {(p['name'], p['version']): p.get('checksum') for p in lock['package']}
    nodes = {node['id']: node for node in meta['resolve']['nodes']}
    return sorted([{'name': p['name'], 'version': p['version'], 'license': p['license'],
                    'checksum': checksums[(p['name'], p['version'])],
                    'features': sorted(nodes[p['id']]['features'])}
                   for p in meta['packages'] if p['name'] != 'omatainer' and p['id'] in nodes],
                  key=lambda p: (p['name'], p['version']))

def rights(expression):
    if expression == 'MPL-2.0':
        return ('Commercial use is permitted by MPL-2.0.',
                'Keep notices and MPL-2.0 terms; make the covered source, including modifications, available to recipients. The exact unmodified source archive is linked below.')
    if expression == 'CC0-1.0':
        return ('CC0 dedicates copyright and related rights to the public domain to the extent possible.',
                'See the retained CC0 waiver and fallback license; it does not grant trademark or patent rights.')
    if expression == 'Unicode-3.0':
        return ('Use, including commercial use, is permitted under Unicode-3.0.',
                'Retain the Unicode copyright and permission notice; see the exact terms below.')
    return ('The recorded open-source license permits commercial use subject to its terms.',
            'Retain the supplied copyright, license and required notices. OR denotes a choice of terms; AND requires all applicable terms. See the exact notices, including any third-party portions, below.')

def update(root, meta, supplements=None):
    policy = load(root/'licenses/assets.json')
    font_package = next(p for p in meta['packages'] if p['name']==policy['font_component']['name'])
    if font_package['version'] != policy['font_component']['version']:
        raise ManifestError('embedded font dependency changed; review fonts and license mapping in assets.json')
    fonts = Path(font_package['manifest_path']).parent
    font_archive = f"https://static.crates.io/crates/{font_package['name']}/{font_package['name']}-{font_package['version']}.crate"
    old = load(root/'licenses/manifest.json') if (root/'licenses/manifest.json').exists() else {'entries': []}
    old_notes = load(root/'licenses/notices.json') if (root/'licenses/notices.json').exists() else {}
    old_entries = {entry['id']: entry for entry in old['entries']}
    notes = {}
    def notice(data, location):
        text = data.decode('utf-8') if isinstance(data, bytes) else data
        digest = sha(text.encode())
        notes[digest] = text
        return {'location': location, 'sha256': digest}
    project_notice = notice(regular(root/'LICENSE'), 'LICENSE')
    entries = []
    for asset in policy['assets']:
        entry = dict(asset)
        entry['notices'] = [project_notice] if entry.pop('project_license', False) else []
        entries.append(entry)
    cargos = graph(meta, root)
    by_name = {(p['name'],p['version']): p for p in meta['packages']}
    for row in cargos:
        p = by_name[(row['name'],row['version'])]
        path = Path(p['manifest_path']).parent
        archive = f"https://static.crates.io/crates/{p['name']}/{p['name']}-{p['version']}.crate"
        component_id = f"crate:{p['name']}@{p['version']}"
        refs = []
        for file in sorted(path.rglob('*')):
            if file.is_file() and (any(word in file.name.upper() for word in ('LICENSE','LICENCE','COPYING','NOTICE','COPYRIGHT','UNLICENSE'))
                                   or (p['name']=='epaint_default_fonts' and file.suffix=='.txt')):
                if file.stat().st_size > 1_000_000:
                    raise ManifestError(f'notice is unexpectedly large: {file}')
                refs.append(notice(file.read_bytes(), f"{archive}#{file.relative_to(path).as_posix()}"))
        if not refs:
            supplied = supplements.get(p['name'], []) if supplements else []
            if supplied:
                refs.extend(notice(record['text'], record['url']) for record in supplied)
            elif component_id in old_entries:
                refs.extend(notice(old_notes[r['sha256']], r['location']) for r in old_entries[component_id]['notices'])
            else:
                raise ManifestError(f'missing pinned upstream license record: {component_id}')
        commercial, redistribution = rights(p['license'])
        entries.append({'id': component_id, 'name': f"{p['name']} {p['version']}", 'category': 'rust-component',
                        'delivery': 'resolved Linux build/runtime dependency; not a claim that every module is linked',
                        'license': p['license'], 'commercial_use': commercial, 'redistribution': redistribution,
                        'sources': [{'location': archive, 'sha256': row['checksum']}], 'notices': refs,
                        'members': []})
    for font in policy['fonts']:
        refs = [notice((fonts/name).read_bytes(), font_archive+'#'+name) for name in font['notice_files']]
        # Font name-table copyright records are part of the exact bundled file,
        # including notices omitted from the standalone license template.
        import struct
        data = (fonts/font['file']).read_bytes()
        count = struct.unpack_from('>H', data, 4)[0]
        names = []
        for i in range(count):
            tag, _, offset, _ = struct.unpack_from('>4sIII', data, 12+i*16)
            if tag != b'name': continue
            _, n, base = struct.unpack_from('>HHH', data, offset)
            for j in range(n):
                platform, _, _, kind, length, start = struct.unpack_from('>HHHHHH', data, offset+6+j*12)
                if kind not in (0,7,8,9,13,14): continue
                raw = data[offset+base+start:offset+base+start+length]
                text = raw.decode('utf-16-be' if platform in (0,3) else 'mac_roman').strip()
                if text and text not in names:names.append(text)
        if names:
            refs.append(notice('\n\n'.join(names)+'\n', font_archive+'#'+font['file']+' (embedded name-table notices)'))
        entry = {k:v for k,v in font.items() if k not in ('file','notice_files')}
        entry['delivery'] += ' '+font_package['version']
        entry['sources'] = [{'location': font_archive+'#'+font['file'], 'sha256': sha(data)}]
        entry['notices'] = refs
        entries.append(entry)
    rust = toolchain()
    sysroot = Path(subprocess.run(['rustc', '--print', 'sysroot'], check=True, capture_output=True, text=True).stdout.strip())
    copyright = sysroot/'share/doc/rust/COPYRIGHT-library.html'
    from html.parser import HTMLParser
    class Text(HTMLParser):
        def __init__(self):super().__init__();self.parts=[]
        def handle_data(self,data):self.parts.append(data)
    parser = Text(); parser.feed(regular(copyright).decode())
    source = f"https://github.com/rust-lang/rust/tree/{rust['commit-hash']}/library"
    refs = [notice(''.join(parser.parts), source+' (toolchain COPYRIGHT-library.html text)')]
    entries.append({'id':'toolchain:rust-library@'+rust['release'], 'name':'Rust standard library '+rust['release'],
        'category':'toolchain-runtime', 'delivery':'standard library/runtime portions linked by rustc; compiler and documentation are not shipped',
        'license':'MIT OR Apache-2.0; constituent licenses are listed in the retained copyright record',
        'commercial_use':'The Rust library is open-source software; commercial use is subject to the supplied library and constituent terms.',
        'redistribution':'Retain the library copyright and constituent notices. The exact upstream revision and complete toolchain-supplied library copyright record are retained.',
        'sources':[{'location':source, 'sha256':sha(regular(copyright))}], 'notices':refs, 'members':[]})
    tracked = list(policy['package'].keys()) + ['LICENSE','Cargo.toml','Cargo.lock','licenses/assets.json',
        'scripts/license-manifest.py','scripts/install-transaction.py'] + [p.relative_to(root).as_posix() for p in sorted((root/'src').rglob('*.rs'))]
    document = {'schema': 1, 'target': rust['host'], 'application': tomllib.loads((root/'Cargo.toml').read_text())['package']['version'],
                'scope': policy['scope'], 'entries': entries, 'toolchain': rust, 'cargo': cargos, 'package': policy['package'],
                'external': policy['external'], 'absent': policy['absent'],
                'source_files': {name:sha(regular(root/name)) for name in sorted(set(tracked))}}
    (root/'licenses/manifest.json').write_bytes(encoded(document))
    (root/'licenses/notices.json').write_bytes(encoded(notes))
    return document

def validate(root, meta=None):
    document = load(root/'licenses/manifest.json'); notes = load(root/'licenses/notices.json')
    if document['schema'] != 1 or document['target'] not in SUPPORTED_TARGETS:
        raise ManifestError('unsupported license manifest schema/target')
    if len(set(document['package'].values())) != len(document['package']):raise ManifestError('duplicate package destination')
    for name in [*document['source_files'], *document['package'], *document['package'].values()]:relative(name)
    if set(document['package'].values()) & ({'.local/bin/omatainer', RECEIPT}|set(INSTALLED)):raise ManifestError('package destination collides with release records')
    ids=set(); used=set()
    for entry in document['entries']:
        if entry['id'] in ids or not entry['notices'] or not all(entry.get(k) for k in ('id','name','category','license','commercial_use','redistribution','sources')):
            raise ManifestError('invalid or duplicate licensed entry')
        ids.add(entry['id'])
        for record in entry['notices']:
            key=record['sha256']; used.add(key)
            if key not in notes or sha(notes[key].encode()) != key:
                raise ManifestError(f'missing or altered notice for {entry["id"]}')
    if used != set(notes):raise ManifestError('unindexed notice records')
    if document['toolchain'] != toolchain():raise ManifestError('Rust toolchain changed; refresh the retained runtime notices')
    expected_src = {p.relative_to(root).as_posix() for p in (root/'src').rglob('*.rs')}
    if expected_src != {name for name in document['source_files'] if name.startswith('src/')}:raise ManifestError('unmanifested or missing Rust source')
    for name,digest in document['source_files'].items():
        if sha(regular(root/name)) != digest:raise ManifestError(f'source changed; review and refresh license manifest: {name}')
    found={p.relative_to(root).as_posix() for directory in ('contrib','plugin') for p in (root/directory).rglob('*') if p.is_file() or p.is_symlink()}
    if found != set(document['package']):raise ManifestError('unmanifested or missing integration asset')
    if graph(meta if meta is not None else metadata(root), root) != document['cargo']:
        raise ManifestError('resolved Cargo components/features changed; refresh reviewed license records')
    return document

def verify_binary(root, binary):
    verify_embedded(binary, {name:regular(root/'licenses'/name) for name in RECORD_FILES})

def verify_embedded(binary, records):
    target=json.loads(records['manifest.json'])['target']
    if target not in SUPPORTED_TARGETS:raise ManifestError('unsupported embedded-record target')
    data = regular(binary)
    if data[:4] != b'\x7fELF' or data[4:6] != b'\x02\x01' or data[18:20] != ({'x86_64-unknown-linux-gnu': b'\x3e\x00', 'aarch64-unknown-linux-gnu': b'\xb7\x00'}[target]):
        raise ManifestError('release binary must be native Linux ELF matching the manifest target')
    if b'omatainer:offline-license-records:v1' not in data:
        raise ManifestError('binary predates the offline license-record protocol; rebuild before packaging')
    for name in RECORD_FILES:
        try:
            result = subprocess.run([str(binary.resolve()), 'licenses', '--'+name.removesuffix('.json')],
                                    capture_output=True, timeout=5, check=True)
        except subprocess.SubprocessError as error:
            raise ManifestError(f'cannot verify embedded license records: {error}') from error
        if result.stdout != records[name]:
            raise ManifestError(f'executable embeds different license records: {name}')

def release_record(root, binary, document):
    try:
        revision=subprocess.run(['git','-C',str(root),'rev-parse','HEAD'],check=True,capture_output=True,text=True).stdout.strip()
        dirty=bool(subprocess.run(['git','-C',str(root),'status','--porcelain','--',*document['source_files']],check=True,capture_output=True,text=True).stdout)
    except subprocess.SubprocessError:
        revision=None;dirty=None
    return {'schema':1, 'target':document['target'], 'application':document['application'],
            'files': {'.local/bin/omatainer':sha(regular(binary)),
                      **{dest:sha(regular(root/src)) for src,dest in document['package'].items()},
                      **{dest:sha(regular(root/src)) for dest,src in INSTALLED.items()}},
            'source_files':document['source_files'],
            'project_source': {'repository':'https://github.com/michaelmonetized/omatainer', 'revision':revision, 'source_tree_modified':dirty, 'identity':'source_files SHA-256 values identify the exact local sources, including reviewed local modifications'},
            'external_configuration': 'Existing user desktop configuration is merged by the installer; it is not re-licensed. The transaction journal retains the exact before/after bytes.',
            'source_notice': 'Unmodified MPL component sources are available at the exact archive URLs in manifest.json under MPL-2.0. Archive SHA-256s identify the source releases; retained notices contain their full terms.'}

def verify_package(directory):
    receipt=load(directory/RECEIPT)
    if receipt['schema'] != 1 or receipt['target'] not in SUPPORTED_TARGETS:raise ManifestError('unsupported release receipt')
    expected=set(receipt['files'])|{RECEIPT}
    found={p.relative_to(directory).as_posix() for p in directory.rglob('*') if p.is_file() or p.is_symlink()}
    if found != expected:raise ManifestError('unmanifested or missing package file')
    for name,digest in receipt['files'].items():
        if sha(regular(directory/relative(name))) != digest:raise ManifestError(f'package hash mismatch: {name}')
    manifest=load(directory/f'{LICENSE_ROOT}/manifest.json')
    if {**{'.local/bin/omatainer':None}, **{dest:None for dest in manifest['package'].values()}, **{dest:None for dest in INSTALLED}}.keys() != receipt['files'].keys():
        raise ManifestError('receipt content differs from licensed package inventory')
    notes=load(directory/f'{LICENSE_ROOT}/notices.json')
    for source,dest in manifest['package'].items():
        if receipt['files'][dest] != manifest['source_files'][source]:raise ManifestError('packaged asset differs from reviewed source')
    for entry in manifest['entries']:
        for ref in entry['notices']:
            if sha(notes[ref['sha256']].encode()) != ref['sha256']:raise ManifestError('altered package notice')
    return receipt

def publish_directory(source, destination):
    # Linux renameat2 gives atomic no-replace publication, including a destination
    # created by another process after the preflight check.
    libc=ctypes.CDLL(None, use_errno=True)
    result=libc.renameat2(-100, os.fsencode(source), -100, os.fsencode(destination), 1)
    if result != 0:
        error=ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))

def package(root,binary,destination,meta=None):
    document=validate(root,meta)
    verify_binary(root,binary)
    if destination.exists():raise ManifestError('package destination already exists')
    destination.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.omatainer-package-',dir=destination.parent) as temporary:
        stage=Path(temporary)/'payload';stage.mkdir()
        files={'.local/bin/omatainer':binary, **{dest:root/src for src,dest in document['package'].items()}, **{dest:root/src for dest,src in INSTALLED.items()}}
        for name,source in files.items():
            out=stage/name;out.parent.mkdir(parents=True,exist_ok=True);out.write_bytes(regular(source));out.chmod(0o755 if name=='.local/bin/omatainer' else 0o644)
        (stage/RECEIPT).write_bytes(encoded(release_record(root,binary,document)))
        verify_package(stage)
        verify_embedded(stage/'.local/bin/omatainer', {name:regular(stage/LICENSE_ROOT/name) for name in RECORD_FILES})
        # A package is an immutable release directory, published only after all
        # bytes validate. An existing destination is never overwritten.
        publish_directory(stage,destination)
    return destination

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['update','check','package','verify-package'])
    parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--supplements',type=Path,help='explicit pinned upstream {crate:[{url,text}]} records for missing crate notices')
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--destination',type=Path)
    args=parser.parse_args();root=args.root.resolve()
    if args.action in ('package','verify-package') and args.destination is None:parser.error('--destination is required')
    try:
        if args.action=='verify-package':verify_package(args.destination)
        else:
            meta=metadata(root)
            if args.action=='update':update(root,meta,load(args.supplements) if args.supplements else None);validate(root,meta)
            elif args.action=='check':validate(root,meta)
            else:package(root,args.binary or root/'target/release/omatainer',args.destination,meta)
        print('License records verified.')
    except (OSError,ValueError,KeyError,TypeError,subprocess.SubprocessError) as error:
        parser.exit(1,f'License records: {error}\n')
if __name__=='__main__':main()
