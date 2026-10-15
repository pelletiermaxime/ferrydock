#!/usr/bin/env python3
"""Package and inspect the Windows x64 executable using only the standard library."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]


def inspect_executable(data):
    """Read PE32+ headers, imports and resource types before shipping a ZIP."""
    if data[:2] != b'MZ':
        raise ValueError('Not a Windows executable')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Invalid PE signature')
    machine, sections = struct.unpack_from('<HH', data, pe + 4)
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    if machine != 0x8664 or struct.unpack_from('<H', data, optional)[0] != 0x20b:
        raise ValueError('Windows preview requires a PE32+ x64 executable')
    if struct.unpack_from('<H', data, optional + 68)[0] != 2:
        raise ValueError('Executable must use the Windows GUI subsystem')
    section_table = optional + optional_size

    def offset(rva):
        for index in range(sections):
            entry = section_table + index * 40
            _, address, size, raw = struct.unpack_from('<IIII', data, entry + 8)
            if address <= rva < address + size:
                return raw + rva - address
        raise ValueError(f'PE address outside file sections: {rva:#x}')

    def directory(index):
        return struct.unpack_from('<II', data, optional + 112 + index * 8)

    def imports(index, descriptor_size, name_field, relative=True):
        rva, size = directory(index)
        if not rva:
            return []
        start = offset(rva)
        names = []
        for entry in range(start, start + size, descriptor_size):
            if not any(data[entry:entry + descriptor_size]):
                return names
            name_rva = struct.unpack_from('<I', data, entry + name_field)[0]
            if not relative and not struct.unpack_from('<I', data, entry)[0] & 1:
                raise ValueError('Unsupported absolute delay import')
            name = offset(name_rva)
            names.append(data[name:data.index(b'\0', name)].decode('ascii').lower())
        raise ValueError('Unterminated PE import directory')

    dependencies = sorted(set(imports(1, 20, 12) + imports(13, 32, 4, relative=False)))
    resource_rva, _ = directory(2)
    resource = offset(resource_rva)
    named, numbered = struct.unpack_from('<HH', data, resource + 12)
    types = {
        struct.unpack_from('<I', data, resource + 16 + index * 8)[0]
        for index in range(named + numbered)
    }
    if not {3, 14, 24}.issubset(types):
        raise ValueError('Executable needs an icon, icon group and application manifest')
    return dependencies


def check_dependencies(dependencies, system_directory):
    for name in dependencies:
        # A runner may have the VC redistributable installed; that does not make
        # those DLLs part of a clean Windows installation. Build with static CRT.
        if name.startswith(('vcruntime', 'msvcp', 'concrt')):
            raise ValueError(f'Portable package requires an unbundled runtime: {name}')
        if name.startswith(('api-ms-win-', 'ext-ms-win-')):
            continue
        if not (system_directory / name).is_file():
            raise ValueError(f'Package has an unbundled non-system dependency: {name}')


def package(binary, output, version):
    data = binary.read_bytes()
    dependencies = inspect_executable(data)
    check_dependencies(dependencies, Path(os.environ['WINDIR']) / 'System32')
    output.mkdir(parents=True, exist_ok=True)
    name = f'ferrydock-{version}-windows-x86_64'
    files = {
        f'{name}/ferrydock.exe': data,
        f'{name}/LICENSE': (ROOT / 'LICENSE').read_bytes(),
        f'{name}/INSTALL.txt': (ROOT / 'packaging/windows/INSTALL.txt').read_text()
            .replace('@VERSION@', version).encode('utf-8'),
    }
    archive = output / f'{name}.zip'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as bundle:
        for filename, contents in files.items():
            bundle.writestr(filename, contents)
    # Inspect the distributed archive, including executable identity and CRCs.
    with zipfile.ZipFile(archive) as bundle:
        if bundle.testzip() is not None or set(bundle.namelist()) != set(files):
            raise ValueError('Archive verification failed')
        for filename, contents in files.items():
            if bundle.read(filename) != contents:
                raise ValueError(f'Archive contents changed: {filename}')
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    # Linux combines platform manifests during publication; keep LF filenames.
    (output / 'SHA256SUMS').write_text(
        f'{checksum}  {archive.name}\n', encoding='ascii', newline='\n',
    )
    report = {
        'status': 'passed', 'archive': archive.name,
        'sha256': checksum, 'imports': dependencies,
        'checks': ['PE32+ x64', 'GUI subsystem', 'icon and manifest resources',
                   'system DLLs only', 'ZIP contents and CRCs'],
        'native_desktop_verified': False,
    }
    (output / 'windows-package-check.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'Windows preview written to {archive}')
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as stream:
            stream.write(f'zip={archive.resolve()}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', help='Optional release tag, which must match Cargo.toml')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/x86_64-pc-windows-msvc/release/ferrydock.exe')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/packages')
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        parser.error('Windows package requires a numeric Cargo version')
    if args.tag and args.tag != f'v{version}':
        parser.error(f'Tag {args.tag} does not match Cargo version {version}')
    if os.name != 'nt':
        parser.error('Package on Windows to verify system DLL dependencies')
    package(args.binary, args.output, version)


if __name__ == '__main__':
    main()
