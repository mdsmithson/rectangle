import os
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import proton


@unittest.skipUnless(os.name == "posix", "requires Linux TUN devices and shell helpers")
class ProtonRecoveryTest(unittest.TestCase):
    def test_creates_and_opens_container_tun(self):
        with patch.object(Path, "mkdir") as mkdir, \
                patch.object(os, "mknod") as mknod, \
                patch.object(os, "open", return_value=23) as open_device, \
                patch.object(os, "close") as close:
            proton.prepare_tun()
        mkdir.assert_called_once_with(exist_ok=True)
        mknod.assert_called_once_with(Path("/dev/net/tun"), stat.S_IFCHR | 0o600,
                                     os.makedev(10, 200))
        open_device.assert_called_once_with(Path("/dev/net/tun"), os.O_RDWR | os.O_NOFOLLOW)
        close.assert_called_once_with(23)

    def test_existing_device_must_be_tun(self):
        for mode, device, valid in ((stat.S_IFCHR, os.makedev(10, 200), True),
                                    (stat.S_IFREG, 0, False),
                                    (stat.S_IFLNK, 0, False),
                                    (stat.S_IFCHR, os.makedev(1, 3), False)):
            with self.subTest(mode=mode, device=device), \
                    patch.object(Path, "mkdir"), \
                    patch.object(os, "mknod", side_effect=FileExistsError), \
                    patch.object(Path, "lstat", return_value=SimpleNamespace(st_mode=mode, st_rdev=device)), \
                    patch.object(os, "open", return_value=23) as open_device, \
                    patch.object(os, "close"):
                if valid:
                    proton.prepare_tun()
                    open_device.assert_called_once()
                else:
                    with self.assertRaisesRegex(SystemExit, "unexpected TUN"):
                        proton.prepare_tun()
                    open_device.assert_not_called()

    def test_unavailable_driver_reports_failure(self):
        with patch.object(Path, "mkdir"), patch.object(os, "mknod"), \
                patch.object(os, "open", side_effect=OSError):
            with self.assertRaisesRegex(SystemExit, "host TUN driver"):
                proton.prepare_tun()

    def test_restart_respects_offline_and_container_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            config = home / ".config/rectangle/proton"
            config.mkdir(parents=True)
            (config / "client.conf").write_text("mock profile")
            (config / "client.conf").chmod(0o600)
            (config / "resolv.conf").write_text("nameserver 1.1.1.1\n")
            (config / "resolv.conf").chmod(0o644)
            bin_dir = home / "bin"
            bin_dir.mkdir()
            log = home / "docker.log"
            docker = bin_dir / "docker"
            docker.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$DOCKER_LOG"\n'
                              'case "$*" in\n*"inspect -f"*) echo unrelated ;;\nesac\n')
            docker.chmod(0o755)
            env = dict(os.environ, HOME=str(home), PATH=f"{bin_dir}:{os.environ['PATH']}",
                       DOCKER_LOG=str(log))
            env.pop("RECTANGLE_RELEASE_DIR", None)
            helper = Path(__file__).with_name("rectangle-proton")
            offline = config.parent / "offline"
            offline.touch()
            result = subprocess.run(["sh", str(helper), "restart"], env=env,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("offline mode", result.stderr)
            self.assertFalse(log.exists())
            offline.unlink()
            result = subprocess.run(["sh", str(helper), "restart"], env=env,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("another service", result.stderr)
            self.assertNotIn("rm -f", log.read_text())
