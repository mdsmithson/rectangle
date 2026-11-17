"""Test the file protocol against real Linux filesystem operations."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name("agent-files.py").absolute()


@unittest.skipUnless(sys.platform.startswith("linux"), "requires Linux dirfd semantics")
class AgentFilesTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.home = self.root / "home"
        self.home.mkdir()

    def call(self, action, path, *args, content=b""):
        result = subprocess.run(
            [sys.executable, "-I", str(HELPER), action, str(path).encode().hex(), *args],
            env=dict(os.environ, HOME=str(self.home)), capture_output=True,
            input=content, timeout=10,
        )
        return result

    def test_upload_and_download_preserve_binary_data(self):
        filename = "nested % 日本語.bin"
        encoded = filename.encode().hex()
        data = b"\x00\xff\r\n" * 100_000
        upload = self.call("upload", self.home, encoded, str(len(data)), content=data)
        self.assertEqual(upload.stdout, b"OK\n", upload.stderr)
        download = self.call("download", self.home, encoded)
        self.assertEqual(download.stdout, f"RECTDOWNLOAD1 {len(data)}\n".encode() + data)
        overlong = self.call("upload", self.home, "6f74686572", "1", content=b"too long")
        self.assertNotEqual(overlong.returncode, 0)
        self.assertEqual(sorted(p.name for p in self.home.iterdir()), [filename])

    def test_rename_does_not_overwrite_and_delete_does_not_follow_links(self):
        (self.home / "source").write_bytes(b"source")
        (self.home / "target").write_bytes(b"target")
        rename = self.call("rename", self.home, b"source".hex(), str(self.home).encode().hex(), b"target".hex())
        self.assertNotEqual(rename.returncode, 0)
        self.assertEqual((self.home / "target").read_bytes(), b"target")
        folder = self.home / "folder"
        folder.mkdir()
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "keep").write_text("keep")
        (folder / "link").symlink_to(outside, target_is_directory=True)
        moved = self.call("rename", self.home, b"source".hex(), str(folder).encode().hex(), b"source".hex())
        self.assertEqual(moved.stdout, b"OK\n", moved.stderr)
        deleted = self.call("delete", self.home, b"folder".hex())
        self.assertEqual(deleted.stdout, b"RECTDELETE1 3\nOK\n", deleted.stderr)
        self.assertFalse(folder.exists())
        self.assertTrue((outside / "keep").exists())

    def test_listing_has_dates_and_sorts_before_pagination(self):
        for entry in ("alpha", "Zulu", "beta"):
            (self.home / entry).touch()
        result = self.call("list", self.home, "1", "desc")
        self.assertEqual(result.returncode, 0, result.stderr)
        marker, first, second = result.stdout.decode().splitlines()
        self.assertEqual(marker, "RECTFILES2 3")
        self.assertEqual([bytes.fromhex(row.split()[2]).decode() for row in (first, second)], ["beta", "alpha"])
        self.assertEqual(len(first.split()), 5)

    def test_editor_view_shell_syntax(self):
        subprocess.run(["sh", "-n", str(HELPER.with_name("agent-view"))], check=True, timeout=10)
