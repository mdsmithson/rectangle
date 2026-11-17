"""Check port suggestions against Docker allocations and a real socket."""
import json
import os
from pathlib import Path
import shlex
import socket
import subprocess
import tempfile
import unittest

LAUNCHER = Path(__file__).with_name("rectangle-session").absolute()


@unittest.skipUnless(os.name == "posix", "requires the Linux session launcher")
class AgentPortTests(unittest.TestCase):
    def test_next_port_skips_docker_publications_and_live_listener(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        listener = self.enterContext(socket.socket())
        for port in range(4000, 4150):
            try:
                listener.bind(("127.0.0.1", port))
                listener.listen()
                break
            except OSError:
                continue
        else:
            self.skipTest("no suitable local port")
        # Reserve all the lower candidates with Docker, including one dynamic mapping.
        reserved = {f"{num}/tcp": [{"HostPort": str(num)}] for num in range(4000, port)}
        dynamic = {"80/tcp": [{"HostPort": str(port + 1)}]}
        docker = root / "docker"
        docker.write_text("#!/bin/sh\ncase $1 in ps) echo fixture ;; inspect) printf '%s\\n' "
                          + shlex.quote(json.dumps([reserved, dynamic])) + " ;; *) exit 1 ;; esac\n")
        docker.chmod(0o700)
        response = subprocess.run(["sh", str(LAUNCHER)], capture_output=True, text=True, timeout=15,
                                  env=dict(os.environ, HOME=str(root), PATH=f"{root}:{os.environ['PATH']}",
                                           SSH_ORIGINAL_COMMAND="exec ~/.local/bin/rectangle-session agent-next-port"))
        self.assertEqual(response.returncode, 0, response.stderr)
        marker, suggestion = response.stdout.split()
        self.assertEqual(marker, "RECTAGENTPORT1")
        self.assertGreaterEqual(int(suggestion), port + 2)
