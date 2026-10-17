import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location('verify_headless', SCRIPTS / 'verify-headless.py')
VERIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFIER)


class HeadlessDependencyTests(unittest.TestCase):
    def setUp(self):
        self.manifest = {'dependencies': {
            alias: {'package': package, 'version': '0.1.7'}
            for alias, package in (
                ('gpui', 'gpui-fast'),
                ('gpui_platform', 'gpui-fast-platform'),
                ('gpui_wgpu', 'gpui-fast-wgpu'),
            )
        }}
        self.lock = {'package': [
            {'name': dependency['package'], 'version': '0.1.8'}
            for dependency in self.manifest['dependencies'].values()
        ]}

    def test_reports_resolved_version_instead_of_manifest_minimum(self):
        self.assertEqual(VERIFIER.resolved_gpui_version(self.manifest, self.lock), '0.1.8')

    def test_rejects_mismatched_backend_versions(self):
        self.lock['package'][0]['version'] = '0.1.7'
        with self.assertRaisesRegex(RuntimeError, 'same version'):
            VERIFIER.resolved_gpui_version(self.manifest, self.lock)

    def test_rejects_missing_backend(self):
        self.lock['package'].pop()
        with self.assertRaisesRegex(RuntimeError, 'Expected one resolved version'):
            VERIFIER.resolved_gpui_version(self.manifest, self.lock)

    def test_rejects_multiple_versions_of_one_backend(self):
        self.lock['package'].append({'name': 'gpui-fast', 'version': '0.1.7'})
        with self.assertRaisesRegex(RuntimeError, 'Expected one resolved version'):
            VERIFIER.resolved_gpui_version(self.manifest, self.lock)
