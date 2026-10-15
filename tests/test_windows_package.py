import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/package-windows.py'
SPEC = importlib.util.spec_from_file_location('package_windows', SCRIPT)
PACKAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGER)


def executable():
    """Small PE fixture with an import and the required resource directory types."""
    data = bytearray(0xa00)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    struct.pack_into('<HH', data, 0x84, 0x8664, 1)
    struct.pack_into('<H', data, 0x94, 240)
    optional = 0x98
    struct.pack_into('<H', data, optional, 0x20b)
    struct.pack_into('<H', data, optional + 68, 2)
    struct.pack_into('<II', data, optional + 120, 0x1000, 40)
    struct.pack_into('<II', data, optional + 128, 0x1100, 40)
    struct.pack_into('<IIII', data, optional + 240 + 8, 0x800, 0x1000, 0x800, 0x200)
    struct.pack_into('<I', data, 0x200 + 12, 0x1050)
    data[0x250:0x25d] = b'KERNEL32.dll\0'
    struct.pack_into('<HH', data, 0x300 + 12, 0, 3)
    for index, resource_type in enumerate([3, 14, 24]):
        struct.pack_into('<I', data, 0x310 + index * 8, resource_type)
    return data


class WindowsPackageTests(unittest.TestCase):
    def test_requires_x64_gui_and_embedded_resources(self):
        self.assertEqual(PACKAGER.inspect_executable(executable()), ['kernel32.dll'])
        for offset, value in [(0x84, 0x14c), (0x98 + 68, 3), (0x320, 99)]:
            data = executable()
            struct.pack_into('<H', data, offset, value)
            with self.assertRaises(ValueError):
                PACKAGER.inspect_executable(data)

    def test_rejects_runner_only_runtime_and_missing_dlls(self):
        with tempfile.TemporaryDirectory() as directory:
            system = Path(directory)
            for name in ['vcruntime140.dll', 'msvcp140.dll', 'concrt140.dll', 'thirdparty.dll']:
                (system / name).touch()
            for name in ['vcruntime140.dll', 'msvcp140.dll', 'concrt140.dll', 'missing.dll']:
                with self.assertRaises(ValueError):
                    PACKAGER.check_dependencies([name], system)
            PACKAGER.check_dependencies(['thirdparty.dll', 'api-ms-win-core-file-l1-1-0.dll'], system)

    def test_zip_and_checksum_cover_the_inspected_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            system = root / 'System32'
            system.mkdir()
            (system / 'kernel32.dll').touch()
            binary = root / 'ferrydock.exe'
            binary.write_bytes(executable())
            output = root / 'packages'
            with patch.dict(os.environ, {'WINDIR': str(root), 'GITHUB_OUTPUT': ''}), patch('builtins.print'):
                PACKAGER.package(binary, output, '1.2.3')
            archive = output / 'ferrydock-1.2.3-windows-x86_64.zip'
            with zipfile.ZipFile(archive) as bundle:
                self.assertEqual(bundle.read('ferrydock-1.2.3-windows-x86_64/ferrydock.exe'), binary.read_bytes())
                self.assertIn(b'FerryDock 1.2.3', bundle.read('ferrydock-1.2.3-windows-x86_64/INSTALL.txt'))
            self.assertEqual(
                (output / 'SHA256SUMS').read_bytes(),
                f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n'.encode('ascii'),
            )
            self.assertFalse(json.loads((output / 'windows-package-check.json').read_text())['native_desktop_verified'])


if __name__ == '__main__':
    unittest.main()
