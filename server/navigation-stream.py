"""Bridge the restricted SSH channel to the packaged Firefox AutoConfig."""
import json
import os
from pathlib import Path
import select
import sys
import time
from urllib.parse import urlsplit

ROOT = Path("/tmp/rectangle-navigation")
LIMIT = 8192
if not ROOT.is_dir():
    sys.exit("Browser controls need a new container. Save and close, then reopen the tab.")

previous = None
pending = bytearray()
while True:
    try:
        current = (ROOT / "state").read_bytes()
        if len(current) <= LIMIT and current != previous:
            sys.stdout.buffer.write(current)
            sys.stdout.buffer.flush()
            previous = current
    except FileNotFoundError:
        pass
    if not select.select([0], [], [], 0.12)[0]:
        continue
    chunk = os.read(0, LIMIT)
    if not chunk:
        break
    pending.extend(chunk)
    while b"\n" in pending:
        raw, _, pending = pending.partition(b"\n")
        if len(raw) > LIMIT:
            sys.exit("Browser command too large")
        action, _, target = raw.decode().partition(" ")
        request = {"action": action}
        if action == "navigate":
            url = urlsplit(target)
            if url.scheme not in ("http", "https") or not url.hostname or url.username is not None:
                sys.exit("Invalid browser address")
            request["url"] = target
        elif action not in ("back", "forward", "reload"):
            sys.exit("Invalid browser command")
        for _ in range(40):
            if not (ROOT / "command").exists():
                break
            time.sleep(0.05)
        else:
            sys.exit("Firefox is not responding to browser controls")
        temporary = ROOT / f"command-{os.getpid()}"
        temporary.write_text(json.dumps(request))
        temporary.replace(ROOT / "command")
    if len(pending) > LIMIT:
        sys.exit("Browser command too large")
