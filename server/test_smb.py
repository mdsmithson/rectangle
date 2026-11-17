"""Test the share boundary, naming rules, and metadata deletion guard."""
import csv
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

if os.name == "posix":
    import smb


@unittest.skipUnless(os.name == "posix", "requires the Linux SMB helper")
class SmbTests(unittest.TestCase):
    def test_setup_requires_docker_subpath_support(self):
        for version in ("25.0.5 1.44", "26.0.1 1.44", "25.0.5 1.45"):
            with patch.object(smb, "docker", return_value=version), self.assertRaisesRegex(ValueError, "26 or newer"):
                smb.require_subpath_support()
        with patch.object(smb, "docker", return_value="28.0.0 1.48"):
            smb.require_subpath_support()

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
        fields = ["type=volume", 'volume-subpath=My, "Music"', "readonly"]
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
