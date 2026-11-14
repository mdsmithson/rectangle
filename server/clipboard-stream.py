"""UTF-8 text for the private SSH channel. Never use the X11 PRIMARY selection."""
from ctypes import CDLL, byref, c_int, c_long, c_ulong, c_void_p
import os
import select
import struct
import subprocess
import sys
import time

LIMIT = 1024 * 1024
XCLIP = ("xclip", "-selection", "clipboard", "-target", "UTF8_STRING")


def get_text():
    with subprocess.Popen((*XCLIP, "-out"), stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL) as source:
        data = bytearray()
        deadline = time.monotonic() + 0.5
        try:
            while len(data) <= LIMIT:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not select.select([source.stdout], [], [], remaining)[0]:
                    break
                chunk = os.read(source.stdout.fileno(), min(65536, LIMIT + 1 - len(data)))
                if not chunk:
                    if data and b"\0" not in data and source.wait(timeout=0.1) == 0:
                        data.decode("utf-8")
                        return bytes(data)
                    break
                data.extend(chunk)
        except (UnicodeError, subprocess.TimeoutExpired):
            pass
        finally:
            if source.poll() is None:
                source.kill()
    return None


def write(packet):
    sys.stdout.buffer.write(packet)
    sys.stdout.buffer.flush()


class SelectionEvents:
    def __init__(self):
        x11 = CDLL("libX11.so.6")
        fixes = CDLL("libXfixes.so.3")
        x11.XOpenDisplay.restype = c_void_p
        x11.XDefaultRootWindow.restype = c_ulong
        x11.XInternAtom.restype = c_ulong
        self.display = c_void_p(x11.XOpenDisplay(None))
        if not self.display:
            raise OSError("No X11 display for the clipboard")
        self.x11 = x11
        self.event = (c_long * 24)()
        root = x11.XDefaultRootWindow(self.display)
        clipboard = x11.XInternAtom(self.display, b"CLIPBOARD", 0)
        # Notify when a client takes ownership, including when Firefox copies the same text again.
        fixes.XFixesSelectSelectionInput(self.display, c_ulong(root), c_ulong(clipboard), c_ulong(1))
        self.fd = x11.XConnectionNumber(self.display)
        x11.XSync(self.display, 0)

    def drain(self):
        changed = False
        while self.x11.XPending(self.display):
            self.x11.XNextEvent(self.display, byref(self.event))
            changed = True
        return changed


def main():
    selection = SelectionEvents()
    # Unbuffered reads let select see the next command even if it arrives with a paste.
    with os.fdopen(sys.stdin.fileno(), "rb", buffering=0, closefd=False) as pipe:
        write(b"RECTCLP1")
        while True:
            if selection.drain():
                current = get_text()
                if current is not None:
                    write(b"C" + struct.pack(">I", len(current)) + current)
            if pipe in select.select([pipe, selection.fd], [], [])[0]:
                header = pipe.read(4)
                while header and len(header) < 4:
                    chunk = pipe.read(4 - len(header))
                    if not chunk:
                        return
                    header += chunk
                if not header:
                    return
                length, = struct.unpack(">I", header)
                if not 0 < length <= LIMIT:
                    return
                data = bytearray()
                while len(data) < length:
                    chunk = pipe.read(length - len(data))
                    if not chunk:
                        return
                    data.extend(chunk)
                try:
                    data.decode("utf-8")
                    if b"\0" in data:
                        return
                    subprocess.run((*XCLIP, "-in", "-silent"), input=data, check=True,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   timeout=0.75)
                except (UnicodeError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
                    write(b"E")
                else:
                    selection.x11.XSync(selection.display, 0)
                    selection.drain()  # Do not send the client's own paste back to Windows.
                    write(b"P")


if __name__ == "__main__":
    try:
        main()
    except BrokenPipeError:
        pass
