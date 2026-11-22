"""Exercise the real Mac installer with signed fixture executables."""
import os
from pathlib import Path
import platform
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
