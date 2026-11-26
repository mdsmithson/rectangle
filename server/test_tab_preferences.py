import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

if os.name == "posix":
    import tab_preferences

AGENT = "a" * 32
BROWSER = "b" * 32
TOKEN = "c" * 32
SERVER = Path(__file__).resolve().parent


@unittest.skipUnless(os.name == "posix", "requires Linux SSH launcher and file locks")
class TabPreferencesTests(unittest.TestCase):
    def setUp(self):
        self.home = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.root = self.home / ".config/rectangle"
        saved = self.root / "saved"
        saved.mkdir(parents=True)
        (saved / AGENT).write_text("4167656e74 agent:demo:4007\n")
        (saved / BROWSER).write_text("42726f77736572\n")
        shutil.copyfile(SERVER / "tab_preferences.py", self.root / "tab_preferences.py")
        docker = self.home / "docker"
        docker.write_text(f'#!/bin/sh\ncase "$*" in *rectangle-tab-{TOKEN}) echo "agent {AGENT}" ;; *) exit 1 ;; esac\n')
        docker.chmod(0o700)
        self.env = dict(os.environ, HOME=str(self.home), PATH=f"{self.home}:{os.environ['PATH']}")
        self.env.pop("RECTANGLE_RELEASE_DIR", None)
        self.enterContext(patch.object(tab_preferences, "ROOT", self.root))

    def request(self, identity, field="get", value=""):
        command = f"exec ~/.local/bin/rectangle-session tab-preferences {identity} {field}" + (f" {value}" if value else "")
        return subprocess.run(["sh", str(SERVER / "rectangle-session")], env=dict(self.env, SSH_ORIGINAL_COMMAND=command), capture_output=True, text=True, timeout=15)

    def test_live_tab_and_saved_profile_share_preferences_across_clients(self):
        self.assertEqual(self.request(AGENT).stdout, "RECTPREF1\n-\noff\n-\n")
        self.assertEqual(self.request(TOKEN, "resolution", "qhd").stdout, "OK\n")
        self.assertIn("\nqhd\n", self.request(AGENT).stdout)
        self.assertEqual(self.request(BROWSER, "smooth", "on").stdout, "OK\n")
        self.assertIn("\non\n", self.request(BROWSER).stdout)
        for profile in (BROWSER, AGENT):
            for resolution in ("half-tile", "third-tile", "tile"):
                self.assertEqual(self.request(profile, "resolution", resolution).stdout, "OK\n")
                self.assertIn(f"\n{resolution}\n", self.request(profile).stdout)
        self.assertIn("\non\n", self.request(BROWSER).stdout)
        record = self.root / "tab-preferences" / (AGENT + ".json")
        self.assertEqual(record.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.root / "saved" / AGENT).read_text(), "4167656e74 agent:demo:4007\n")

    def test_concurrent_fields_do_not_overwrite_each_other(self):
        folder = "/agent/home/workspace/日本語 files"
        with concurrent.futures.ThreadPoolExecutor() as pool:
            futures = [pool.submit(self.request, AGENT, "folder", folder.encode().hex()),
                       pool.submit(self.request, AGENT, "resolution", "uhd")]
            for future in futures:
                result = future.result()
                self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(tab_preferences.load(AGENT), {"folder": folder, "resolution": "uhd"})
        self.assertIn(folder.encode().hex(), self.request(AGENT).stdout)

    def test_rejects_wrong_tab_types_invalid_paths_and_unknown_profiles(self):
        for identity, field, value in [(BROWSER, "resolution", "qhd"), (AGENT, "smooth", "on"),
                                       (AGENT, "folder", "2f2e2e"), (AGENT, "folder", "72656c6174697665"),
                                       (AGENT, "folder", "2f00"), (AGENT, "folder", "ff"),
                                       (AGENT, "resolution", "9999x9999"), ("d" * 32, "resolution", "qhd"),
                                       ("../bad", "resolution", "qhd")]:
            with self.subTest(identity=identity, field=field, value=value):
                self.assertNotEqual(self.request(identity, field, value).returncode, 0)
        self.assertFalse((self.root / "tab-preferences" / (AGENT + ".json")).exists())

    def test_cloned_profiles_copy_values_and_remain_independent(self):
        self.assertEqual(self.request(AGENT, "resolution", "host").returncode, 0)
        clone = "e" * 32
        tab_preferences.copy_profile(AGENT, clone)
        self.assertEqual(tab_preferences.load(clone), {"resolution": "host"})
        self.request(AGENT, "resolution", "full-hd")
        self.assertEqual(tab_preferences.load(clone), {"resolution": "host"})
