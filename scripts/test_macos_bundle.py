"""Check the packaged icon and signature, independently of the running app."""
import os
from pathlib import Path
import platform
import plistlib
import subprocess
import tempfile
import unittest


@unittest.skipUnless(platform.system() == 'Darwin', 'requires Apple bundle tools')
class MacBundleTests(unittest.TestCase):
    def test_finder_icon_is_packaged_and_has_retina_representations(self):
        app = Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / 'release/Rectangle.app'
        if not app.exists():
            self.skipTest('run scripts/package-macos.sh first')
        with (app / 'Contents/Info.plist').open('rb') as source:
            plist = plistlib.load(source)
        icon = app / 'Contents/Resources' / plist['CFBundleIconFile']
        self.assertTrue(icon.is_file())
        self.assertEqual(icon.read_bytes()[:4], b'icns')
        with tempfile.TemporaryDirectory() as directory:
            iconset = Path(directory) / 'Rectangle.iconset'
            subprocess.run(['iconutil', '-c', 'iconset', str(icon), '-o', str(iconset)], check=True)
            self.assertTrue((iconset / 'icon_16x16.png').is_file())
            retina = iconset / 'icon_512x512@2x.png'
            self.assertTrue(retina.is_file())
            dimensions = subprocess.check_output(['sips', '-g', 'pixelWidth', '-g', 'pixelHeight', str(retina)], text=True)
            self.assertIn('pixelWidth: 1024', dimensions)
            self.assertIn('pixelHeight: 1024', dimensions)
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
