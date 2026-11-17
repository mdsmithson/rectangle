"""Offer named, read-only SMB shares of saved Agent volume subdirectories."""
from contextlib import contextmanager
import csv
import fcntl
from getpass import getpass
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile
import time
import uuid

CONFIG = Path.home() / ".config/rectangle"
ROOT = CONFIG / "smb"
ASSETS = Path(__file__).resolve().parent
SERVICE = "rectangle-smb"
STATE_VOLUME = "rectangle-smb-passwords"
USERNAME = "rectangle"
ID = re.compile(r"[0-9a-f]{32}")
SHARE = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,47}")
RESERVED = {"global", "homes", "printers", "ipc", "admin", "print"}
PRIVATE = tuple(map(ipaddress.IPv4Network, ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")))
LIMIT = 32
MEDIA = re.compile(r"rectangle-smb-media-[0-9a-f]{32}")


def docker(*args, stdin=None, allow_failure=False, timeout=90):
    result = subprocess.run(("docker", *args), input=stdin, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, timeout=timeout)
    if result.returncode and not allow_failure:
        raise ValueError(result.stderr.strip() or "Docker could not configure the SMB service.")
    return result.stdout.strip() if result.returncode == 0 else None


@contextmanager
def lock():
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (ROOT / "lock").open("a") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        yield


def load(filename, default):
    try:
        return json.loads((ROOT / filename).read_text())
    except FileNotFoundError:
        return default


def store(filename, data):
    fd, temp = tempfile.mkstemp(prefix=".state-", dir=ROOT)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(data, stream, ensure_ascii=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, ROOT / filename)
    finally:
        Path(temp).unlink(missing_ok=True)


def validate_network(address, cidr):
    ip = ipaddress.IPv4Address(address)
    lan = ipaddress.IPv4Network(cidr)
    if (not any(lan.subnet_of(block) for block in PRIVATE) or ip not in lan
            or ip in (lan.network_address, lan.broadcast_address)):
        raise ValueError("Use Emerald's private LAN IPv4 address and subnet, such as a /24.")
    return {"address": str(ip), "lan": str(lan)}


def validate_share(name):
    if not SHARE.fullmatch(name) or name.lower() in RESERVED:
        raise ValueError("Use 1–48 letters, numbers, underscores, or hyphens for the share name.")
    return name


def folder(value):
    if len(value.encode()) > 1024 or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ValueError("Invalid shared folder.")
    parts = value.split("/")
    if parts[:3] != ["", "agent", "home"] or len(parts) < 4 or any(p in ("", ".", "..") for p in parts[3:]):
        raise ValueError("Share a subfolder under /agent/home.")
    return "/".join(parts[3:])


def mount(*fields):
    line = io.StringIO()
    csv.writer(line, lineterminator="\n").writerow(fields)
    return line.getvalue().rstrip("\n")


def configuration(settings, shares):
    net = ipaddress.IPv4Network(settings["lan"])
    text = f"""[global]
server role = standalone server
map to guest = Never
server min protocol = SMB2_10
server signing = mandatory
smb ports = 445
disable netbios = yes
hosts allow = {net.network_address}/{net.netmask} 127.0.0.1
hosts deny = ALL
passdb backend = tdbsam
private dir = /state/private
state directory = /state
cache directory = /run/samba
lock directory = /run/samba
pid directory = /run/samba
load printers = no
disable spoolss = yes
logging = stdout
"""
    for index, item in enumerate(shares):
        name = validate_share(item["name"])
        text += f"""
[{name}]
path = /shares/{index}
read only = yes
guest ok = no
valid users = {USERNAME}
follow symlinks = no
wide links = no
"""
    return text


def ensure_image():
    recipe = ASSETS / "Dockerfile.smb"
    content = recipe.read_bytes()
    digest = hashlib.sha256(b"Dockerfile.smb\0" + hashlib.sha256(content).digest()).hexdigest()
    tag = f"rectangle-smb:build-{digest}"
    if docker("image", "inspect", "-f", "{{.Id}}", tag, allow_failure=True) is None:
        with tempfile.TemporaryDirectory(prefix="image-", dir=ROOT) as staging:
            (Path(staging) / "Dockerfile.smb").write_bytes(content)
            subprocess.run(("docker", "build", "--pull", "-f", "Dockerfile.smb", "-t", tag, "."),
                           cwd=staging, stdout=sys.stderr, check=True, timeout=1200)
    return tag


def config_file(settings, shares):
    with tempfile.NamedTemporaryFile(mode="w", prefix="smb-", suffix=".conf", dir=ROOT, delete=False) as config:
        config.write(configuration(settings, shares))
    return Path(config.name)


def state_mount():
    return mount("type=volume", f"source={STATE_VOLUME}", "target=/state")


def configuration_mount(config):
    return mount("type=bind", f"source={config}", "target=/config/smb.conf", "readonly")


def apply(settings, shares, persist):
    if any("ready" not in item for item in shares):
        raise ValueError("Turn off the remaining 0.1.13 SMB shares on this host before activating dedicated folders.")
    shares = [item for item in shares if item.get("ready") and item.get("name")]
    image = ensure_image()
    config = config_file(settings, shares)
    old = docker("inspect", "-f", '{{index .Config.Labels "rectangle.role"}}', SERVICE, allow_failure=True)
    if old is not None and old != "smb":
        config.unlink(missing_ok=True)
        raise ValueError("Another Docker container is already named rectangle-smb.")
    standby = f"{SERVICE}-old-{config.stem}"
    moved = False
    created = False
    try:
        docker("run", "--rm", "--network", "none", "--mount", configuration_mount(config),
               "--entrypoint", "testparm", image, "-s", "/config/smb.conf")
        if old is not None:
            docker("rename", SERVICE, standby)
            moved = True
            docker("stop", "-t", "5", standby)
        if shares:
            args = ["create", "--name", SERVICE, "--label", "rectangle.role=smb",
                    "--restart", "unless-stopped", "--read-only", "--cap-drop=ALL",
                    "--security-opt", "no-new-privileges", "--pids-limit", "128", "--memory", "256m",
                    "--tmpfs", "/run/samba:rw,nosuid,nodev,size=16m",
                    "--tmpfs", "/tmp:rw,nosuid,nodev,size=16m",
                    "--publish", f"{settings['address']}:445:445/tcp",
                    "--mount", state_mount(), "--mount", configuration_mount(config)]
            for cap in ("CHOWN", "DAC_OVERRIDE", "FOWNER", "SETGID", "SETUID", "NET_BIND_SERVICE"):
                args += ["--cap-add", cap]
            for index, item in enumerate(shares):
                args += ["--mount", mount("type=volume", f"source={media_volume(item)}",
                                        f"target=/shares/{index}", "readonly", "volume-nocopy")]
            docker(*args, image)
            created = True
            docker("start", SERVICE)
            time.sleep(0.5)
            if docker("inspect", "-f", "{{.State.Status}} {{.RestartCount}}", SERVICE) != "running 0":
                raise ValueError("Samba did not start. Check TCP 445 and the Docker logs.")
        persist()
    except BaseException:
        if created:
            docker("rm", "-f", SERVICE, allow_failure=True)
        if moved:
            docker("rename", standby, SERVICE)
            docker("start", SERVICE)
        config.unlink(missing_ok=True)
        raise
    if moved:
        docker("rm", standby, allow_failure=True)
    if not shares:
        config.unlink(missing_ok=True)


def setup(address, cidr):
    settings = validate_network(address, cidr)
    password = getpass(f"Set the SMB password for {USERNAME}: ")
    if len(password) < 12 or any(c in "\r\n\0" for c in password):
        raise ValueError("Use an SMB password of at least 12 characters.")
    if password != getpass("Repeat SMB password: "):
        raise ValueError("Passwords did not match.")
    with lock():
        if load("settings.json", None):
            raise ValueError("SMB is already configured on this host.")
        image = ensure_image()
        config = config_file(settings, [])
        try:
            docker("run", "--rm", "-i", "--network", "none", "--mount", state_mount(),
                   "--mount", configuration_mount(config), "--entrypoint", "sh", image, "-ec",
                   "mkdir -p /state/private; chmod 700 /state/private; exec smbpasswd -c /config/smb.conf -a -s rectangle",
                   stdin=f"{password}\n{password}\n")
        finally:
            config.unlink(missing_ok=True)
        store("settings.json", settings)
    print(f"Rectangle SMB is ready at {address}. Use Windows username {USERNAME}.")


def inspect_agent(token):
    if not ID.fullmatch(token):
        raise ValueError("Invalid Agent token.")
    container = f"rectangle-tab-{token}"
    record = docker("inspect", "-f",
                    '{{if .State.Running}}{{index .Config.Labels "rectangle.role"}} {{index .Config.Labels "rectangle.profile"}} {{index .Config.Labels "rectangle.agent-storage"}}{{end}}',
                    container)
    elements = record.split()
    if len(elements) != 3 or elements[0] != "agent" or not ID.fullmatch(elements[1]) or elements[2] != "volume":
        raise ValueError("This Agent does not have a persistent workspace.")
    return container, elements[1]


def media_volume(item):
    volume = item.get("volume", "")
    if not MEDIA.fullmatch(volume):
        raise ValueError("Invalid dedicated SMB volume.")
    return volume


def print_listing(settings, shares, profile):
    print(f"RECTSMB2 {settings['address']}")
    for item in shares:
        if item["profile"] == profile and item.get("name"):
            state = "live" if item.get("ready") else "pending"
            print(item["folder"].encode().hex(), item["name"], state)


def update_listing(token, verb, *fields):
    settings = load("settings.json", None)
    if verb == "list" and not settings:
        print("RECTSMB2 disabled")
        return
    if not settings:
        raise ValueError("Enable SMB once on the Rectangle server.")
    container, profile = inspect_agent(token)
    if verb == "list":
        shares = load("shares.json", [])
    else:
        path = bytes.fromhex(fields[0]).decode()
        folder(path)
        with lock():
            shares = load("shares.json", [])
            existing = next((item for item in shares if item["profile"] == profile and item["folder"] == path), None)
            if verb == "enable":
                if not (CONFIG / "saved" / profile).is_file():
                    raise ValueError("Save and close this Agent once, then reopen it before sharing a folder.")
                if existing and existing.get("name"):
                    raise ValueError("This folder is already shared.")
                name = validate_share(bytes.fromhex(fields[1]).decode())
                if any((item.get("name") or "").lower() == name.lower() for item in shares):
                    raise ValueError("That SMB share name is already used on this host.")
                if sum(bool(item.get("name")) for item in shares) >= LIMIT:
                    raise ValueError("This host already has 32 SMB shares.")
                helper = (ASSETS / "agent-files.py").read_text()
                docker("exec", container, "python3", "-I", "-c", helper, "validate-share", fields[0])
                if existing:
                    existing["name"] = name
                    apply(settings, shares, lambda: store("shares.json", shares))
                else:
                    requested = PurePosixPath(path)
                    for item in shares:
                        current = PurePosixPath(item["folder"])
                        if item["profile"] == profile and (requested.is_relative_to(current) or current.is_relative_to(requested)):
                            raise ValueError("This folder overlaps a folder already managed for SMB.")
                    if sum(item["profile"] == profile for item in shares) >= LIMIT:
                        raise ValueError("This Agent already has 32 dedicated SMB folders.")
                    shares.append({"profile": profile, "folder": path, "name": name, "ready": False})
                    store("shares.json", shares)
            elif verb == "disable":
                if not existing or not existing.get("name"):
                    raise ValueError("This folder is not shared.")
                if existing.get("ready"):
                    existing["name"] = None
                    apply(settings, shares, lambda: store("shares.json", shares))
                elif "ready" in existing:
                    shares.remove(existing)
                    store("shares.json", shares)
                else:
                    raise ValueError("Ask the server owner to remove the 0.1.13 shares with the SMB helper.")
            else:
                raise ValueError("Unknown SMB operation.")
    print_listing(settings, shares, profile)


def migrate(profile, item, image):
    path = folder(item["folder"])
    helper = (ASSETS / "smb-migrate.py").read_text()
    volume = f"rectangle-smb-media-{uuid.uuid4().hex}"
    docker("volume", "create", "--label", "rectangle.role=smb-media",
           "--label", f"rectangle.profile={profile}", volume)
    try:
        common = ["run", "--rm", "--network", "none", "--read-only", "--cap-drop=ALL",
                  "--security-opt", "no-new-privileges", "--memory", "256m", "--pids-limit", "32",
                  "--mount", mount("type=volume", f"source={volume}", "target=/media", "volume-nocopy")]
        docker(*common, "--user", "0:0", "--cap-add", "CHOWN", "--entrypoint", "/bin/sh", image,
               "-ec", "chmod 700 /media; chown 10001:10001 /media")
        docker(*common, "--user", "10001:10001", "--mount",
               mount("type=volume", f"source=rectangle-profile-{profile}", "target=/workspace", "readonly", "volume-nocopy"),
               "--entrypoint", "python3", image, "-I", "-c", helper, path, timeout=3600)
    except BaseException:
        docker("volume", "rm", volume, allow_failure=True)
        raise
    return volume


def prepare(profile, image):
    if not ID.fullmatch(profile):
        raise ValueError("Invalid Agent profile.")
    with lock():
        shares = load("shares.json", [])
        pending = [item for item in shares if item["profile"] == profile and not item.get("ready")]
        if not pending:
            print("RECTSMBPREP1")
            return
        settings = load("settings.json", None)
        if not settings:
            raise ValueError("SMB setup is missing. The original Agent files are unchanged.")
        volumes = []
        try:
            for item in pending:
                volume = migrate(profile, item, image)
                volumes.append(volume)
                item.update(volume=volume, ready=True)
            apply(settings, shares, lambda: store("shares.json", shares))
        except BaseException:
            for volume in volumes:
                docker("volume", "rm", volume, allow_failure=True)
            raise
    print("RECTSMBPREP1")


def agent_mounts(profile):
    if not ID.fullmatch(profile):
        raise ValueError("Invalid Agent profile.")
    for item in load("shares.json", []):
        if item["profile"] != profile or not item.get("ready"):
            continue
        folder(item["folder"])
        volume = media_volume(item)
        label = docker("volume", "inspect", "-f", '{{index .Labels "rectangle.profile"}}', volume)
        if label != profile:
            raise ValueError("The Agent's dedicated media volume is missing or does not match.")
        print(mount("type=volume", f"source={volume}", f"target={item['folder']}", "volume-nocopy"))


def protect(profile, *encoded):
    path = PurePosixPath(bytes.fromhex(encoded[0]).decode()) if encoded else None
    if len(encoded) == 2:
        path = path / bytes.fromhex(encoded[1]).decode()
    for item in load("shares.json", []):
        if item["profile"] != profile:
            continue
        if path is None and item.get("name"):
            raise ValueError("Turn off the SMB share before deleting the saved Agent.")
        if path is not None and PurePosixPath(item["folder"]).is_relative_to(path):
            raise ValueError("This folder is managed by Rectangle as a dedicated SMB volume and cannot be moved or deleted here.")


def forget_storage(profile, check_only=False):
    with lock():
        protect(profile)
        shares = load("shares.json", [])
        owned = [item for item in shares if item["profile"] == profile and item.get("ready")]
        for item in owned:
            volume = media_volume(item)
            if volume not in docker("volume", "ls", "-q", "--filter", f"name={volume}").splitlines():
                continue
            label = docker("volume", "inspect", "-f", '{{index .Labels "rectangle.profile"}}', volume)
            if label != profile:
                raise ValueError("The Agent's dedicated media volume does not match.")
            if docker("ps", "-aq", "--filter", f"volume={volume}"):
                raise ValueError("A Docker container is still using this Agent's dedicated media volume.")
            if not check_only:
                docker("volume", "rm", volume)
        if owned and not check_only:
            store("shares.json", [item for item in shares if item["profile"] != profile])


def admin_remove(name):
    validate_share(name)
    with lock():
        settings = load("settings.json", None)
        shares = load("shares.json", [])
        existing = next((item for item in shares if (item.get("name") or "").lower() == name.lower()), None)
        if not settings or not existing:
            raise ValueError("There is no SMB share with that name.")
        if existing.get("ready"):
            existing["name"] = None
            apply(settings, shares, lambda: store("shares.json", shares))
        elif "ready" in existing or any(item is not existing and "ready" not in item for item in shares):
            shares.remove(existing)
            store("shares.json", shares)
        else:
            shares.remove(existing)
            apply(settings, shares, lambda: store("shares.json", shares))


if __name__ == "__main__":
    os.umask(0o077)
    try:
        operation, *args = sys.argv[1:]
        {"setup": setup, "agent": update_listing, "prepare": prepare, "mounts": agent_mounts,
         "protect": protect, "forget-storage": forget_storage,
         "check-storage": lambda profile: forget_storage(profile, True), "remove": admin_remove}[operation](*args)
    except (OSError, ValueError, TypeError, KeyError, IndexError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
