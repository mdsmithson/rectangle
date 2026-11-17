"""Build a complete Rectangle server release before switching the active release."""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path.home() / ".config/rectangle"
CURRENT = ROOT / "current-release"
BIN = Path.home() / ".local/bin"
# Names, build recipes, and all local sources read by each Dockerfile.
IMAGES = {
    "browser": ("Dockerfile", "firefox-policies.json", "rectangle-autoconfig.js",
                "rectangle.cfg", "navigation-stream.py", "entrypoint", "audio-stream",
                "clipboard-stream.py", "memory-stream.py", "demo.html"),
    "agent": ("Dockerfile.agent", "agent-entrypoint", "clipboard-stream.py", "memory-stream.py"),
    "egress": ("Dockerfile.egress", "egress.py"),
    "vpn": ("Dockerfile.vpn", "egress.py", "vpn.py"),
    "proton": ("Dockerfile.proton", "egress.py", "proton.py"),
    "smb": ("Dockerfile.smb",),
}
VERSION = re.compile(r"(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})")
SHA = re.compile(r"[a-f0-9]{40}")


def version(value):
    if not VERSION.fullmatch(value):
        raise ValueError("Invalid Rectangle version.")
    return tuple(map(int, value.split(".")))


def source_version(source):
    with (source / "Cargo.toml").open(encoding="utf-8") as stream:
        for line in stream:
            if match := re.fullmatch(r'version\s*=\s*"([^"]+)"\s*', line):
                value = match[1]
                version(value)
                return value
    raise ValueError("Rectangle release has no version.")


def installed():
    try:
        data = json.loads((CURRENT / "release.json").read_text())
        version(data["version"])
        if not SHA.fullmatch(data["commit"]):
            raise ValueError("Invalid server release record.")
        return data
    except FileNotFoundError:
        return None


@contextlib.contextmanager
def lock():
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (ROOT / "server-update.lock").open("a") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        yield stream.fileno()


def command(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def image_tag(server, name, inputs):
    digest = hashlib.sha256()
    for entry in inputs:
        digest.update(entry.encode() + b"\0")
        digest.update(hashlib.sha256((server / entry).read_bytes()).digest())
    return f"rectangle-{name}:build-{digest.hexdigest()}"


def stage_images(server):
    result = {}
    for name, inputs in IMAGES.items():
        if name == "smb" and not (ROOT / "smb/settings.json").is_file():
            continue
        tag = image_tag(server, name, inputs)
        exists = subprocess.run(("docker", "image", "inspect", tag), stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, timeout=15).returncode == 0
        if not exists:
            print(f"Building Rectangle {name} image.", file=sys.stderr, flush=True)
            command("docker", "build", "--pull", "-f", inputs[0], "-t", tag, ".",
                    cwd=server, stdout=sys.stderr, timeout=1800)
        result[name] = tag
    return result


def copy(source, destination, mode):
    shutil.copyfile(source, destination)
    destination.chmod(mode)


def activate(source, commit, expected):
    if not SHA.fullmatch(commit) or source_version(source) != expected:
        raise ValueError("The checked-out server source does not match the requested release.")
    server = source / "server"
    for filename in ("rectangle-session", "rectangle-proton", "agent-view", "release-launcher"):
        command("sh", "-n", server / filename, timeout=10)
    images = stage_images(server)
    releases = ROOT / "releases"
    releases.mkdir(exist_ok=True, mode=0o700)
    release = Path(tempfile.mkdtemp(prefix=f"{expected}-{commit[:12]}-", dir=releases))
    try:
        for filename in ("rectangle-session", "rectangle-proton", "release-launcher"):
            copy(server / filename, release / filename, 0o700)
        for filename in ("release.py", "server-update.py", "agent-view", "agent-files.py", "session-seccomp.json", "smb.py", "smb-migrate.py", "Dockerfile.smb"):
            copy(server / filename, release / filename, 0o600)
        context = release / "agent-context"
        context.mkdir(mode=0o700)
        for filename in ("agent-entrypoint", "clipboard-stream.py", "memory-stream.py"):
            copy(server / filename, context / filename, 0o600)
        (release / "images.sh").write_text("".join(f"rectangle_{name}_image={tag}\n" for name, tag in images.items()))
        (release / "release.json").write_text(json.dumps({"version": expected, "commit": commit}))
        next_link = release / ".activate"
        next_link.symlink_to(release, target_is_directory=True)
        os.replace(next_link, CURRENT)
    except BaseException:
        if CURRENT.resolve() != release.resolve():
            shutil.rmtree(release)
        raise
    return release


def bootstrap(source, mode, enable):
    proposed = source_version(source)
    # Source archives work for manual installs; the version still prevents downgrades.
    if (source / ".git").exists():
        commit = command("git", "-C", source, "rev-parse", "HEAD", stdout=subprocess.PIPE,
                         text=True, timeout=10).stdout.strip()
    else:
        commit = "0" * 40
    with lock():
        active = installed()
        if active and version(active["version"]) > version(proposed):
            raise ValueError(f"Server is already on newer Rectangle {active['version']}.")
        release = activate(source, commit, proposed)
        BIN.mkdir(parents=True, exist_ok=True, mode=0o700)
        for program in ("rectangle-proton", "rectangle-session"):
            fd, stage = tempfile.mkstemp(prefix=f".{program}-", dir=BIN)
            os.close(fd)
            try:
                copy(release / "release-launcher", stage_path := Path(stage), 0o700)
                os.replace(stage_path, BIN / program)
            finally:
                Path(stage).unlink(missing_ok=True)
        if mode == "--direct-https":
            (ROOT / "cloud-egress").touch()
        elif mode == "--vpn-only":
            (ROOT / "cloud-egress").unlink(missing_ok=True)
        if enable:
            (ROOT / "allow-server-updates").touch(mode=0o600)
    print(f"Rectangle host is ready ({proposed}).")


if __name__ == "__main__":
    try:
        bootstrap(Path(__file__).resolve().parents[1], sys.argv[1], "--enable-server-updates" in sys.argv[2:])
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Rectangle server install failed: {error}")
