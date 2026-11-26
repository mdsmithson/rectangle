"""Per-profile UI preferences shared by Rectangle clients. Never execute paths."""
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path.home() / ".config/rectangle"
ID = re.compile(r"[0-9a-f]{32}")
TILE_RESOLUTIONS = {"tile", "half-tile", "third-tile"}
RESOLUTIONS = TILE_RESOLUTIONS | {"host", "uhd", "qhd", "full-hd", "dci-2k", "one-k"}


def resolve(identity):
    if not ID.fullmatch(identity):
        raise ValueError("Invalid tab identity.")
    saved = ROOT / "saved" / identity
    if saved.is_file():
        fields = saved.read_text().split()
        return identity, "agent" if len(fields) > 1 and fields[1].startswith("agent:") else "browser"
    data = subprocess.check_output([
        "docker", "inspect", "-f",
        '{{if .State.Running}}{{index .Config.Labels "rectangle.role"}} {{index .Config.Labels "rectangle.profile"}}{{end}}',
        "rectangle-tab-" + identity,
    ], text=True, stderr=subprocess.DEVNULL, timeout=8).split()
    if len(data) != 2 or data[0] not in {"agent", "browser"} or not ID.fullmatch(data[1]):
        raise ValueError("This tab is no longer available.")
    return data[1], data[0]


def validate(field, value):
    if field == "resolution" and value in RESOLUTIONS:
        return value
    if field == "smooth" and value in {"on", "off"}:
        return value
    if field == "folder" and len(value) <= 8192 and re.fullmatch(r"(?:[0-9a-f]{2})+", value):
        path = bytes.fromhex(value).decode("utf-8")
        if path.startswith("/") and "\0" not in path and not any(p in {".", ".."} for p in path.split("/")):
            return path
    raise ValueError("Invalid tab preference.")


def load(identity):
    try:
        data = json.loads((ROOT / "tab-preferences" / (identity + ".json")).read_text())
    except FileNotFoundError:
        return {}
    if not isinstance(data, dict) or set(data) - {"resolution", "smooth", "folder"}:
        raise ValueError("Invalid saved tab preferences.")
    for field, value in data.items():
        validate(field, value.encode().hex() if field == "folder" else value)
    return data


def write(identity, data):
    directory = ROOT / "tab-preferences"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".preferences-", dir=directory)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, ensure_ascii=True)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, directory / (identity + ".json"))
    finally:
        Path(temporary).unlink(missing_ok=True)


def request(identity, field, value=""):
    identity, role = resolve(identity)
    directory = ROOT / "tab-preferences"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    with (directory / "preferences.lock").open("a") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        data = load(identity)
        if field == "get" and not value:
            print("RECTPREF1")
            print(data.get("resolution", "-"))
            print(data.get("smooth", "off"))
            print(data["folder"].encode().hex() if "folder" in data else "-")
            return
        if (
            (field == "smooth" and role != "browser")
            or (field == "folder" and role != "agent")
            or (field == "resolution" and role == "browser" and value not in TILE_RESOLUTIONS)
        ):
            raise ValueError("This preference is not available for this tab type.")
        data[field] = validate(field, value)
        write(identity, data)
        print("OK")


def copy_profile(source, target):
    if not ID.fullmatch(source) or not ID.fullmatch(target):
        raise ValueError("Invalid tab identity.")
    directory = ROOT / "tab-preferences"
    if not directory.is_dir():
        return
    with (directory / "preferences.lock").open("a") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        if data := load(source):
            write(target, data)


if __name__ == "__main__":
    os.umask(0o077)
    try:
        request(*sys.argv[1:])
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as error:
        sys.exit(f"Could not access tab preferences: {error}")
