"""Snapshot a live tab's persistent files into an independent saved profile."""
from contextlib import suppress
import csv
import fcntl
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import signal
import socket
import subprocess
import sys
import tempfile
import uuid

ROOT = Path.home() / ".config/rectangle"
ID = re.compile(r"[0-9a-f]{32}")


def docker(*args, timeout=90):
    p = subprocess.run(["docker", *args], capture_output=True, text=True, timeout=timeout)
    if p.returncode:
        raise ValueError(p.stderr.strip() or "Docker could not copy this tab.")
    return p.stdout.strip()


def interrupted(_signal, _frame):
    raise InterruptedError("The clone was interrupted.")


def mount(*fields):
    output = io.StringIO()
    csv.writer(output, lineterminator="").writerow(fields)
    return output.getvalue()


def write(path, value):
    fd, temporary = tempfile.mkstemp(prefix=".clone-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def free_port(saved):
    used = set()
    for entry in saved.iterdir():
        if ID.fullmatch(entry.name) and entry.is_file():
            fields = entry.read_text().split()
            if len(fields) > 1 and fields[1].startswith("agent:"):
                used.add(int(fields[1].split(":")[2]))
    ids = docker("ps", "-q").splitlines()
    for offset in range(0, len(ids), 100):
        records = docker("inspect", "-f", "{{json .HostConfig.PortBindings}}", *ids[offset:offset + 100])
        for record in records.splitlines():
            for protocol, bindings in (json.loads(record) or {}).items():
                if protocol.endswith("/tcp"):
                    used.update(int(item["HostPort"]) for item in (bindings or [])
                                if item.get("HostPort", "").isdigit())
    ipv6 = False
    try:
        with socket.socket(socket.AF_INET6) as probe:
            probe.bind(("::", 0))
            ipv6 = True
    except OSError:
        pass
    for port in range(4000, 65536):
        if port in used:
            continue
        try:
            with socket.socket() as probe:
                probe.bind(("0.0.0.0", port))
            if ipv6:
                with socket.socket(socket.AF_INET6) as probe:
                    probe.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
                    probe.bind(("::", port))
            return port
        except OSError:
            continue
    raise ValueError("No unused Agent port is available.")


COPY = """
import hashlib, os, shutil, stat, sys
from pathlib import Path
def sockets(directory, names):
    return [name for name in names if stat.S_ISSOCK(os.lstat(os.path.join(directory, name)).st_mode)]
def checked(source, target):
    mode = os.lstat(source).st_mode
    if stat.S_ISFIFO(mode):
        os.mkfifo(target, stat.S_IMODE(mode))
        return target
    if not stat.S_ISREG(mode):
        raise ValueError('The workspace contains an unsupported special file.')
    shutil.copy2(source, target, follow_symlinks=False)
    with open(source, 'rb') as a, open(target, 'rb') as b:
        if hashlib.file_digest(a, 'sha256').digest() != hashlib.file_digest(b, 'sha256').digest():
            raise ValueError('The file copy did not verify.')
    return target
target = Path('/agent/home')
# Docker initializes an empty volume from the helper image's home skeleton.
shutil.rmtree(target / '.local', ignore_errors=True)
shutil.copytree('/source', target, symlinks=True, dirs_exist_ok=True,
                copy_function=checked, ignore=sockets)
if sys.argv[1] == 'browser':
    for name in ('lock', '.parentlock', 'parent.lock'):
        (target / name).unlink(missing_ok=True)
"""


def clone(token, encoded, helper_image, startup=""):
    if not ID.fullmatch(token):
        raise ValueError("Invalid tab.")
    title = bytes.fromhex(encoded).decode("utf-8").strip()
    if not title or len(title.encode()) > 80 or any(ord(c) < 32 or ord(c) == 127 for c in title):
        raise ValueError("Give the clone a new name, up to 80 bytes.")
    saved = ROOT / "saved"
    saved.mkdir(parents=True, exist_ok=True)
    with (ROOT / "saved.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if sum(bool(ID.fullmatch(p.name)) for p in saved.iterdir()) >= 24:
            raise ValueError("Saved sessions are full. Delete one before cloning.")
        for entry in saved.iterdir():
            if ID.fullmatch(entry.name) and entry.is_file():
                if bytes.fromhex(entry.read_text().split()[0]).decode().casefold() == title.casefold():
                    raise ValueError("Choose a name that is not already saved on this host.")
        container = "rectangle-tab-" + token
        info = json.loads(docker("inspect", container))[0]
        labels = info["Config"]["Labels"] or {}
        role = labels.get("rectangle.role")
        if role not in ("agent", "browser") or not info["State"]["Running"] or info["State"]["Paused"]:
            raise ValueError("The source tab must be running and available.")
        root = "/agent/home" if role == "agent" else "/profile/data"
        mounts = [m for m in info["Mounts"] if m["Destination"] == root or
                  PurePosixPath(m["Destination"]).is_relative_to(PurePosixPath(root))]
        if not any(m["Destination"] == root and m["Type"] == "volume" for m in mounts):
            raise ValueError("This older tab has no persistent profile. Save and reopen it before cloning.")
        if any(m["Type"] != "volume" for m in mounts):
            raise ValueError("This tab has an unsupported workspace mount.")
        identity = uuid.uuid4().hex
        volume = "rectangle-profile-" + identity
        page = ""
        if role == "agent":
            port = free_port(saved)
            prefix = "rectangle-agent:"
            image = info["Config"]["Image"].removeprefix(prefix)
            flavor = "codex" if info["Config"]["Image"].startswith("rectangle-codex:") else "demo"
            page = f"agent:{image if ID.fullmatch(image) else flavor}:{port}"
            if labels.get("rectangle.agent-view") == "files":
                page += ":files"
        else:
            profile = labels.get("rectangle.profile", "")
            if startup:
                page = startup
            elif ID.fullmatch(profile) and (saved / profile).is_file():
                fields = (saved / profile).read_text().split()
                page = fields[1] if len(fields) == 2 else ""
        helper = "rectangle-clone-" + identity
        paused = created = copy_started = published = False
        try:
            docker("volume", "create", "--label", "rectangle.role=profile", volume)
            created = True
            args = ["run", "--rm", "--name", helper, "--pull=never", "--network", "none", "--read-only", "--cap-drop=ALL",
                    "--security-opt", "no-new-privileges", "--memory", "256m", "--pids-limit", "32",
                    "--user", "10001:10001", "--mount",
                    mount("type=volume", f"source={volume}", "target=/agent/home")]
            for item in sorted(mounts, key=lambda m: len(m["Destination"])):
                relative = item["Destination"][len(root):]
                args += ["--mount", mount("type=volume", f"source={item['Name']}",
                                         f"target=/source{relative}", "readonly", "volume-nocopy")]
            paused = True
            docker("pause", container)
            copy_started = True
            docker(*args, "--entrypoint", "python3", helper_image, "-I", "-c", COPY, role, timeout=3600)
            copy_started = False
            docker("unpause", container)
            paused = False
            if role == "agent":
                memory = info["HostConfig"]["Memory"] // 1048576
                if memory >= 256:
                    (ROOT / "agent-memory").mkdir(exist_ok=True)
                    write(ROOT / "agent-memory" / identity, f"{memory}\n")
            record = title.encode().hex() + (f" {page}" if page else "") + "\n"
            write(saved / identity, record)
            published = True
        finally:
            if copy_started:
                with suppress(ValueError, subprocess.SubprocessError):
                    docker("rm", "-f", helper)
            try:
                if paused and docker("inspect", "-f", "{{.State.Paused}}", container) == "true":
                    docker("unpause", container)
            finally:
                if not published:
                    if created:
                        with suppress(ValueError, subprocess.SubprocessError):
                            docker("volume", "rm", volume)
                    (ROOT / "agent-memory" / identity).unlink(missing_ok=True)
        print("RECTSAVED2")
        print(identity, record.strip())


if __name__ == "__main__":
    os.umask(0o077)
    for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, interrupted)
    try:
        clone(*sys.argv[1:])
    except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
