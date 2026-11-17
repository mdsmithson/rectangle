"""Test the share boundary, naming rules, and metadata deletion guard."""
import csv
from contextlib import redirect_stdout
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

if os.name == "posix":
    import smb
    spec = importlib.util.spec_from_file_location("smb_migrate", Path(__file__).with_name("smb-migrate.py"))
    smb_migrate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(smb_migrate)


@unittest.skipUnless(os.name == "posix", "requires the Linux SMB helper")
class SmbTests(unittest.TestCase):
    def test_shared_folder_paths_and_lan_are_constrained(self):
        for path in ("/agent/home", "/agent/home/../etc", "/agent/home/a//b", "/tmp/Music", "/agent/home/a\nb"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                smb.folder(path)
        self.assertEqual(smb.folder("/agent/home/workspace/My, Music"), "workspace/My, Music")
        self.assertEqual(smb.validate_network("192.168.0.2", "192.168.0.0/24")["lan"], "192.168.0.0/24")
        with self.assertRaises(ValueError):
            smb.validate_network("8.8.8.8", "8.8.8.0/24")
        with self.assertRaises(ValueError):
            smb.validate_share("GLOBAL")

    def test_configured_shares_are_read_only_and_have_no_wide_links(self):
        settings = smb.validate_network("192.168.1.2", "192.168.1.0/24")
        text = smb.configuration(settings, [{"name": "Agent-E-Music"}])
        self.assertIn("[Agent-E-Music]\npath = /shares/0\nread only = yes\n", text)
        self.assertIn("follow symlinks = no\nwide links = no", text)
        self.assertIn("hosts allow = 192.168.1.0/255.255.255.0 127.0.0.1", text)
        fields = ["type=volume", 'target=/agent/home/My, "Music"', "readonly"]
        self.assertEqual(next(csv.reader([smb.mount(*fields)])), fields)

    def test_shared_ancestors_and_saved_workspace_cannot_be_deleted(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        profile = "a" * 32
        with patch.object(smb, "ROOT", root):
            smb.store("shares.json", [{"profile": profile, "folder": "/agent/home/workspace/Music", "name": "Music"}])
            with self.assertRaises(ValueError):
                smb.protect(profile)
            with self.assertRaises(ValueError):
                smb.protect(profile, "/agent/home".encode().hex(), "workspace".encode().hex())
            smb.protect(profile, "/agent/home/workspace/Music".encode().hex(), "song.mp3".encode().hex())
            smb.protect("b" * 32)

    def test_host_share_names_are_case_insensitively_unique(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        profile = "a" * 32
        saved = root / "saved"
        saved.mkdir()
        (saved / profile).touch()
        with patch.multiple(smb, ROOT=root, CONFIG=root), patch.object(smb, "inspect_agent", return_value=("Agent", profile)):
            smb.store("settings.json", {"address": "192.168.1.2", "lan": "192.168.1.0/24"})
            smb.store("shares.json", [{"profile": "b" * 32, "folder": "/agent/home/workspace", "name": "Music"}])
            with patch.object(smb, "apply", side_effect=AssertionError("should not publish a duplicate")):
                with self.assertRaisesRegex(ValueError, "already used"):
                    smb.update_listing(profile, "enable", "/agent/home/workspace/Music".encode().hex(), b"mUSIC".hex())

    def test_queue_failure_and_disable_keep_agent_files_available(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        profile = "a" * 32
        volume = "rectangle-smb-media-" + "b" * 32
        path = "/agent/home/workspace/Music"
        (root / "saved").mkdir()
        (root / "saved" / profile).touch()
        (root / "agent-files.py").touch()
        with patch.multiple(smb, ROOT=root, CONFIG=root, ASSETS=root), redirect_stdout(io.StringIO()) as output:
            smb.store("settings.json", {"address": "192.168.1.2", "lan": "192.168.1.0/24"})
            with patch.object(smb, "inspect_agent", return_value=("Agent", profile)), patch.object(smb, "docker"):
                with patch.object(smb, "apply") as apply:
                    smb.update_listing(profile, "enable", path.encode().hex(), b"Music".hex())
                    apply.assert_not_called()
                self.assertIn("Music pending", output.getvalue())
                with patch.object(smb, "migrate", return_value=volume), patch.object(smb, "apply", side_effect=ValueError("port busy")):
                    with self.assertRaisesRegex(ValueError, "port busy"):
                        smb.prepare(profile, "test")
                self.assertFalse(smb.load("shares.json", [])[0]["ready"])
                smb.docker.assert_any_call("volume", "rm", volume, allow_failure=True)
                with patch.object(smb, "migrate", return_value=volume), patch.object(smb, "apply", side_effect=lambda _, __, persist: persist()):
                    smb.prepare(profile, "test")
                    smb.update_listing(profile, "disable", path.encode().hex())
                entry = smb.load("shares.json", [])[0]
                self.assertIsNone(entry["name"])
                self.assertEqual(entry["volume"], volume)
                smb.docker.return_value = profile
                smb.agent_mounts(profile)
                self.assertIn(f"source={volume},target={path},volume-nocopy", output.getvalue())

    def test_copy_keeps_original_and_does_not_follow_links(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        source, target = root / "source", root / "media"
        folder = source / "Music"
        folder.mkdir(parents=True)
        target.mkdir()
        (folder / "A, B.mp3").write_bytes(b"music")
        (source / "private").write_text("private")
        (folder / "link").symlink_to("../private")
        smb_migrate.copy_folder(source, target, "Music")
        self.assertTrue((target / "link").is_symlink())
        self.assertFalse((target / "private").exists())
        self.assertEqual((target / "A, B.mp3").read_bytes(), b"music")
        (target / "A, B.mp3").write_bytes(b"edited")
        self.assertEqual((folder / "A, B.mp3").read_bytes(), b"music")
        (source / "Other").symlink_to("Music")
        with self.assertRaises(OSError):
            smb_migrate.copy_folder(source, target, "Other")
