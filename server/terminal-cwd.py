"""Read the current directory of one Rectangle xterm's program."""
import os
from pathlib import Path
import select
import sys


def fields(process):
    # The process name can contain spaces or closing parentheses.
    return (process / "stat").read_bytes().rsplit(b")", 1)[1].split()


def xterm_pid(proc, display):
    display = b"DISPLAY=:" + display.encode("ascii")
    for process in proc.iterdir():
        if not process.name.isdecimal():
            continue
        try:
            if (process / "comm").read_bytes().strip() == b"xterm" and display in (
                process / "environ"
            ).read_bytes().split(b"\0"):
                return process
        except OSError:
            continue
    return None


def child(proc, terminal):
    try:
        pids = (terminal / "task" / terminal.name / "children").read_text().split()
    except FileNotFoundError:
        # Some Linux kernels omit the optional children file.
        pids = (p.name for p in proc.iterdir() if p.name.isdecimal())
    for pid in pids:
        process = proc / pid
        try:
            stat = fields(process)
            if stat[1].decode("ascii") == terminal.name and int(stat[4]):
                return process
        except (OSError, IndexError, ValueError):
            continue
    return None


def directory(proc, program, shell):
    if shell:
        stat = fields(program)
        foreground = int(stat[5])
        if foreground > 0 and foreground != int(program.name):
            leader = proc / str(foreground)
            try:
                # Follow a nested foreground shell on this same PTY. Commands
                # such as git, ssh and editors do not replace the shell path.
                if fields(leader)[4] == stat[4] and (leader / "comm").read_bytes().strip() in {
                    b"sh", b"ash", b"bash", b"dash", b"zsh", b"fish", b"ksh", b"nu",
                }:
                    program = leader
            except (OSError, IndexError):
                pass
    path = os.readlink(os.fsencode(program / "cwd"))
    return path if path.startswith(b"/") and len(path) <= 4096 else None


def stream(display, shell):
    proc = Path("/proc")
    terminal = program = None
    last = None
    while True:
        try:
            if terminal is None or not terminal.exists():
                terminal = xterm_pid(proc, display)
                program = None
            if terminal is not None and (program is None or not program.exists()):
                program = child(proc, terminal)
            path = directory(proc, program, shell) if program is not None else None
        except (OSError, IndexError, ValueError):
            terminal = program = path = None
        current = path.hex() if path is not None else "-"
        if current != last:
            print(current, flush=True)
            last = current
        if select.select([sys.stdin], [], [], 0.5 if program is not None else 1)[0]:
            return


if __name__ == "__main__":
    stream(sys.argv[1], sys.argv[2] == "sh")
