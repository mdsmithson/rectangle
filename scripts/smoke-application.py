"""Exercise an Application tab against a real amd64 Docker engine."""
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import tempfile
import time

SERVER = Path(__file__).resolve().parents[1] / "server"
IMAGE = "rectangle-application:smoke"
TOKEN = "a" * 32
REOPENED = "b" * 32
TITLE = "Application".encode().hex()


def docker(*args, timeout=60):
    return subprocess.check_output(("docker", *args), text=True, timeout=timeout)


def handshake(connection):
    def read(length):
        data = bytearray()
        while len(data) < length:
            assert select.select([connection.stdout], [], [], 30)[0], "RFB handshake timed out"
            chunk = os.read(connection.stdout.fileno(), length - len(data))
            assert chunk, "RFB disconnected during handshake"
            data.extend(chunk)
        return data

    assert read(12) == b"RFB 003.008\n"
    connection.stdin.write(b"RFB 003.008\n")
    connection.stdin.flush()
    assert 1 in read(read(1)[0]), "RFB did not offer the local security type"
    connection.stdin.write(b"\x01")
    connection.stdin.flush()
    assert read(4) == b"\x00" * 4, "RFB rejected the security type"
    connection.stdin.write(b"\x01")
    connection.stdin.flush()
    header = read(24)
    assert struct.unpack(">HH", header[:4]) == (1280, 800)
    read(struct.unpack(">I", header[20:])[0])


def main():
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp)
        (home / "images.sh").write_text(
            f"rectangle_application_image={IMAGE}\nrectangle_agent_image={IMAGE}\n"
        )
        (home / "session-seccomp.json").symlink_to(SERVER / "session-seccomp.json")
        env = dict(os.environ, HOME=tmp, RECTANGLE_RELEASE_DIR=tmp)

        def launch(*args, **kwargs):
            return subprocess.Popen(
                ("sh", SERVER / "rectangle-session", *args), env=env, **kwargs
            )

        def request(*args):
            with launch(*args, stdout=subprocess.PIPE) as process:
                output, _ = process.communicate(timeout=60)
                assert process.returncode == 0, args
                return output.decode()

        def check_desktop(token, *options, check):
            container = f"rectangle-tab-{token}"
            with launch("1280x800", token, *options, stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE) as connection:
                try:
                    handshake(connection)
                    deadline = time.monotonic() + 30
                    while True:
                        try:
                            if docker("exec", container, "xdotool", "getdisplaygeometry",
                                      timeout=5).strip() == "1280 800":
                                manager = docker("exec", container, "xprop", "-root",
                                                 "_NET_SUPPORTING_WM_CHECK", timeout=5)
                                if "window id" in manager:
                                    break
                        except subprocess.CalledProcessError:
                            if time.monotonic() >= deadline:
                                raise
                        assert time.monotonic() < deadline, "Desktop did not start"
                        time.sleep(0.5)
                    check(container)
                finally:
                    connection.stdin.close()
                    try:
                        connection.wait(timeout=20)
                    except subprocess.TimeoutExpired:
                        subprocess.run(("docker", "rm", "-f", container), check=False)
                        connection.wait(timeout=20)

        try:
            with socket.socket() as port:
                port.bind(("127.0.0.1", 0))
                assigned = port.getsockname()[1]

            def install(container):
                docker("exec", container, "sh", "-ec",
                       "for tool in python3 node ruby elixir erl cargo rustc cc c++ winetricks; "
                       'do command -v "$tool"; done; pactl info')
                wine_env = ("-e", "WINEDLLOVERRIDES=mscoree,mshtml=")
                try:
                    docker("exec", *wine_env, container, "wineboot", "-u", timeout=180)
                except subprocess.CalledProcessError:
                    # Report the final syscalls if Wine fails before emitting its own diagnostics.
                    trace = subprocess.run(
                        ("docker", "exec", *wine_env, container, "strace", "-f", "wineboot", "-u"),
                        capture_output=True, text=True, timeout=180,
                    )
                    print("\n".join(trace.stderr.splitlines()[-160:]), flush=True)
                    raise
                for cmd in (r"C:\windows\system32\cmd.exe", r"C:\windows\syswow64\cmd.exe"):
                    output = docker("exec", *wine_env, container, "wine", cmd, "/c",
                                    "echo Application", timeout=180)
                    assert "Application" in output
                assert request("save", TOKEN, TOKEN, TITLE) == "OK\n"

            def reopened(container):
                docker("exec", container, "test", "-f", "/agent/home/.wine/system.reg")
                assert f"agent:application:{assigned}" in request("list-v2")

            check_desktop(TOKEN, f"agent:application:{assigned}", check=install)
            check_desktop(REOPENED, TOKEN, check=reopened)
        finally:
            for token in (TOKEN, REOPENED):
                subprocess.run(("docker", "rm", "-f", f"rectangle-tab-{token}"),
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run(("docker", "volume", "rm", f"rectangle-profile-{TOKEN}"),
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()
