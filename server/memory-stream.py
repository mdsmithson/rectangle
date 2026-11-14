"""Report only the browser container's cgroup v2 memory metrics."""
from pathlib import Path
import select
import struct
import sys

CGROUP = Path("/sys/fs/cgroup")
NO_PRESSURE = (1 << 64) - 1


def sample():
    used = int((CGROUP / "memory.current").read_text())
    limit = int((CGROUP / "memory.max").read_text())
    events = dict(line.split() for line in (CGROUP / "memory.events").read_text().splitlines())
    try:
        pressure = (CGROUP / "memory.pressure").read_text().splitlines()
        some = next(line.split() for line in pressure if line.startswith("some "))
        avg10 = dict(field.split("=") for field in some[1:])["avg10"]
        stall = round(float(avg10) * 100)
    except (OSError, ValueError, KeyError, StopIteration):
        stall = NO_PRESSURE
    return struct.pack(">6Q", used, limit, int(events["high"]), int(events["max"]),
                       int(events["oom_kill"]), stall)


def main():
    out = sys.stdout.buffer
    out.write(b"RECTMEM1" + sample())
    out.flush()
    # This is a read-only channel. Any input or EOF ends the exec process.
    while not select.select([sys.stdin], [], [], 1.0)[0]:
        out.write(sample())
        out.flush()


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError):
        pass
