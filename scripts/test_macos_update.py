"""Exercise the real Mac installer with signed fixture executables."""
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'src/update-macos.sh'


@unittest.skipUnless(platform.system() == 'Darwin', 'requires macOS code signing')
class MacUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='Rectangle update spaces ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        source = self.root / 'fixture.c'
        source.write_text('''#include <stdio.h>
int main(int argc, char **argv) {
 if (argc < 2) return 0;
 FILE *f = fopen(argv[1], "w"); if (!f) return 1;
 fprintf(f, "%s\\n", VERSION);
 for (int i=2; i<argc; i++) fprintf(f, "%s\\n", argv[i]);
 return fclose(f);
}
''')
        self.target = self.root / 'installed client'
        self.candidate = self.root / 'new client'
        for path, version in [(self.target, 'old'), (self.candidate, 'new')]:
            subprocess.run(['clang', str(source), '-o', str(path), f'-DVERSION="{version}"'], check=True, capture_output=True)
            subprocess.run(['codesign', '--force', '--sign', '-', str(path)], check=True, capture_output=True)

    def install(self, candidate):
        marker = self.root / 'restarted.txt'
        arg = 'literal spaces $HOME `id` "quotes" 日本語'
        result = subprocess.run(['/bin/sh', str(SCRIPT), str(marker), arg], env={
            **os.environ, 'RECTANGLE_UPDATE_MODE': 'install',
            'RECTANGLE_UPDATE_TARGET': str(self.target),
            'RECTANGLE_UPDATE_CANDIDATE': str(candidate),
            'RECTANGLE_UPDATE_PID': '2147483647',
            'RECTANGLE_UPDATE_CWD': str(self.root),
        }, capture_output=True, timeout=10)
        deadline = time.monotonic() + 5
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue(marker.exists(), result.stderr)
        return result, marker.read_text(), arg

    def test_replaces_signed_client_and_restarts_with_literal_arguments(self):
        result, content, arg = self.install(self.candidate)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(content, f'new\n{arg}\n')
        subprocess.run(['codesign', '--verify', str(self.target)], check=True)
        self.assertFalse(self.candidate.exists())

    def test_invalid_candidate_preserves_and_restarts_previous_client(self):
        self.candidate.write_text('not a signed Mach-O executable')
        result, content, arg = self.install(self.candidate)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(content, f'old\n{arg}\n')
        self.assertIn('previous executable preserved', (self.root / 'install-error.txt').read_text())
        subprocess.run(['codesign', '--verify', str(self.target)], check=True)

    def bundle_fixture(self):
        bundle = self.root / 'Installed Rectangle.app'
        self.target = self.make_bundle(bundle, self.target, 'old')
        payload = self.root / 'Rectangle.app'
        self.make_bundle(payload, self.candidate, 'new')
        return bundle, payload

    def make_bundle(self, bundle, binary, label):
        contents = bundle / 'Contents'
        (contents / 'MacOS').mkdir(parents=True)
        (contents / 'Resources').mkdir()
        executable = contents / 'MacOS/rectangle'
        shutil.copy2(binary, executable)
        (contents / 'Resources/Rectangle.icns').write_bytes(label.encode())
        with (contents / 'Info.plist').open('wb') as out:
            plistlib.dump({'CFBundleIdentifier': 'com.mdsmithson.rectangle.fixture',
                          'CFBundleExecutable': 'rectangle', 'CFBundlePackageType': 'APPL',
                          'CFBundleIconFile': 'Rectangle.icns', 'CFBundleVersion': label}, out)
        subprocess.run(['codesign', '--force', '--sign', '-', str(bundle)], check=True, capture_output=True)
        return executable

    def test_bundle_update_replaces_icon_metadata_and_preserves_signature(self):
        bundle, payload = self.bundle_fixture()
        result, content, arg = self.install(self.candidate)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(content, f'new\n{arg}\n')
        self.assertEqual((bundle / 'Contents/Resources/Rectangle.icns').read_bytes(), b'new')
        with (bundle / 'Contents/Info.plist').open('rb') as source:
            self.assertEqual(plistlib.load(source)['CFBundleVersion'], 'new')
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)

    def test_corrupt_bundle_keeps_previous_application_and_icon(self):
        bundle, payload = self.bundle_fixture()
        (payload / 'Contents/Resources/Rectangle.icns').write_bytes(b'corrupt')
        result, content, arg = self.install(self.candidate)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(content, f'old\n{arg}\n')
        self.assertEqual((bundle / 'Contents/Resources/Rectangle.icns').read_bytes(), b'old')
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)

    def test_missing_bundle_never_falls_back_to_modifying_signed_executable(self):
        bundle, payload = self.bundle_fixture()
        shutil.rmtree(payload)
        result, content, arg = self.install(self.candidate)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(content, f'old\n{arg}\n')
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
