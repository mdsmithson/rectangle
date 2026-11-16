"""Exercise the restricted launcher with a fake Docker CLI and real shell/locks."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


LAUNCHER = Path(__file__).resolve().with_name("rectangle-session")
TOKEN = "0123456789abcdef" * 2
PROFILE = "abcdef0123456789" * 2
MIB = 1024 * 1024


@unittest.skipUnless(os.name == "posix" and shutil.which("flock") and shutil.which("timeout"),
                     "requires a Linux shell and launcher tools")
class AgentMemoryTests(unittest.TestCase):
    def setUp(self):
        directory = self.enterContext(tempfile.TemporaryDirectory())
        self.home = Path(directory)
        self.config = self.home / ".config" / "rectangle"
        self.log = self.home / "update"
        mock = self.home / "docker"
        mock.write_text("""#!/bin/sh
case $1 in
    inspect) printf '%s rectangle-tab-%s test-container %s %s %s\n' \
        "$MOCK_ROLE" "$MOCK_TOKEN" "$MOCK_PROFILE" "$MOCK_LIMIT" "$MOCK_SWAP" ;;
    exec) [ "$2 $3 $4" = "test-container cat /sys/fs/cgroup/memory.current" ] || exit 2
        printf '%s\n' "$MOCK_USAGE" ;;
    update) shift; printf '%s\n' "$*" > "$HOME/update" ;;
    *) exit 2 ;;
esac
""")
        mock.chmod(0o700)
        self.env = dict(os.environ, HOME=directory, PATH=f"{directory}:{os.environ['PATH']}",
                        MOCK_TOKEN=TOKEN, MOCK_PROFILE=PROFILE, MOCK_ROLE="agent",
                        MOCK_LIMIT=str(1024 * MIB), MOCK_SWAP=str(1280 * MIB),
                        MOCK_USAGE=str(600 * MIB))

    def request(self, value=""):
        command = f"exec ~/.local/bin/rectangle-session agent-memory {TOKEN}{value}"
        return subprocess.run(["sh", str(LAUNCHER)], text=True, capture_output=True,
                              env=dict(self.env, SSH_ORIGINAL_COMMAND=command), timeout=10)

    def test_live_update_preserves_swap_and_saved_profile_limit(self):
        saved = self.config / "saved"
        saved.mkdir(parents=True)
        record = saved / PROFILE
        record.write_text("4167656e74 agent:demo:8080\n")
        status = self.request()
        self.assertEqual(status.returncode, 0, status.stderr)
        self.assertTrue(status.stdout.startswith(f"RECTAGENTMEM1 {600 * MIB} {1024 * MIB} "))
        self.assertFalse(self.log.exists())
        updated = self.request(" 768")
        self.assertEqual(updated.returncode, 0, updated.stderr)
        self.assertTrue(updated.stdout.startswith(f"RECTAGENTMEM1 {600 * MIB} {768 * MIB} "))
        self.assertEqual(self.log.read_text().strip(),
                         f"--memory {768 * MIB} --memory-swap {1024 * MIB} test-container")
        self.assertEqual((self.config / "agent-memory" / PROFILE).read_text(), "768\n")
        self.assertEqual(record.read_text(), "4167656e74 agent:demo:8080\n")

    def test_rejects_lowering_to_current_usage_and_malformed_requests(self):
        for value in (" 600", " 512", " 0", " 768 --privileged"):
            with self.subTest(value=value):
                self.assertNotEqual(self.request(value).returncode, 0)
        self.assertFalse(self.log.exists())

    def test_rejects_browser_containers(self):
        self.env["MOCK_ROLE"] = "browser"
        self.assertNotEqual(self.request(" 768").returncode, 0)
        self.assertFalse(self.log.exists())
