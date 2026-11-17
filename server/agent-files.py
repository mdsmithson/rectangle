"""Agent file operations; mutable paths are confined to the Agent home."""
import contextlib
import ctypes
from datetime import datetime, timezone
import os
import platform
from pathlib import Path
import stat
import struct
import sys
import uuid
import re
import unicodedata

FLAGS = os.O_DIRECTORY | os.O_NOFOLLOW | os.O_RDONLY
CHUNK = 256 * 1024


def decode(field):
    return bytes.fromhex(field).decode()


def name(value):
    if value in ("", ".", "..") or "/" in value or "\0" in value:
        raise ValueError("Invalid file name.")
    return value


def portable_name(value):
    name(value)
    stem = value.split(".", 1)[0].rstrip().upper()
    if (not value.strip() or value != value.strip() or value.endswith(".")
            or len(value.encode()) > 255
            or any(unicodedata.category(c) == "Cc" or c in '<>:"\\|?*' for c in value)
            or re.fullmatch(r"CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|(?:COM|LPT)[1-9¹²³]", stem)):
        raise ValueError("Choose a name that is valid on both Linux and Windows.")
    return value


def create(path, filename, folder):
    entry = portable_name(filename)
    with directory_fd(path) as directory:
        if folder:
            os.mkdir(entry, 0o700, dir_fd=directory)
        else:
            os.close(os.open(entry, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=directory))
    print("OK")


def stamp(seconds):
    return datetime.fromtimestamp(seconds, timezone.utc).strftime("%Y-%m-%d %H:%M") if seconds is not None else "-"


def created(directory, entry, info):
    birth = getattr(info, "st_birthtime", None)
    if birth is None:
        buf = ctypes.create_string_buffer(256)
        args = (directory, os.fsencode(entry), 0x100 | 0x4000, 0x800, buf)
        if hasattr(LIBC, "statx"):
            result = LIBC.statx(*args)
        elif (number := {"x86_64": 332, "aarch64": 291}.get(ARCH)) is not None:
            result = LIBC.syscall(ctypes.c_long(number), *args)
        else:
            result = -1
        if result == 0:
            if struct.unpack_from("=I", buf)[0] & 0x800:
                birth = struct.unpack_from("=q", buf, 80)[0]
    return stamp(birth)


LIBC = ctypes.CDLL(None, use_errno=True)
ARCH = platform.machine()


def listing(path, offset, descending=False):
    if offset < 0:
        raise ValueError("Invalid page.")
    # File Manager may browse the container, but never follows directory links.
    with directory_fd(path, Path("/")) as directory:
        with os.scandir(directory) as entries:
            items = []
            for entry in entries:
                try:
                    entry.name.encode()
                    info = entry.stat(follow_symlinks=False)
                except (UnicodeError, FileNotFoundError):
                    continue
                items.append((entry.name, info))
        items.sort(key=lambda entry: (entry[0].casefold(), entry[0]), reverse=descending)
        items.sort(key=lambda entry: not stat.S_ISDIR(entry[1].st_mode))
        print(f"RECTFILES2 {len(items)}")
        for entry, info in items[offset:offset + 150]:
            kind = "d" if stat.S_ISDIR(info.st_mode) else "f" if stat.S_ISREG(info.st_mode) else "o"
            print(kind, info.st_size, entry.encode().hex(),
                  created(directory, entry, info).encode().hex(),
                  stamp(info.st_mtime).encode().hex())


@contextlib.contextmanager
def directory_fd(path, root=None):
    path = Path(path)
    root = root if root is not None else Path(os.environ["HOME"])
    parts = path.relative_to(root).parts
    if ".." in parts:
        raise ValueError("Invalid folder.")
    fd = os.open(root, FLAGS)
    try:
        for part in parts:
            child = os.open(part, FLAGS, dir_fd=fd)
            os.close(fd)
            fd = child
        yield fd
    finally:
        os.close(fd)


def upload(path, filename, size):
    name(filename)
    if size < 0:
        raise ValueError("Invalid upload size.")
    temporary = ".rectangle-upload-" + uuid.uuid4().hex
    with directory_fd(path) as directory:
        stream = os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                                   0o600, dir_fd=directory), "wb")
        try:
            with stream as output:
                left = size
                while left:
                    chunk = sys.stdin.buffer.read(min(left, CHUNK))
                    if not chunk:
                        raise ValueError("Upload was interrupted.")
                    output.write(chunk)
                    left -= len(chunk)
                if sys.stdin.buffer.read(1):
                    raise ValueError("Local file grew during upload.")
                output.flush()
                os.fsync(output.fileno())
            os.link(temporary, filename, src_dir_fd=directory,
                    dst_dir_fd=directory, follow_symlinks=False)
        finally:
            os.unlink(temporary, dir_fd=directory)
    print("OK")


def regular(directory, entry):
    fd = os.open(entry, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW, dir_fd=directory)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode):
        os.close(fd)
        raise ValueError("Choose a regular file.")
    return os.fdopen(fd, "rb"), info


def download(path, filename):
    with directory_fd(path) as directory:
        source, info = regular(directory, name(filename))
        with source:
            out = sys.stdout.buffer
            out.write(f"RECTDOWNLOAD1 {info.st_size}\n".encode())
            out.flush()
            left = info.st_size
            while left:
                chunk = source.read(min(left, CHUNK))
                if not chunk:
                    raise ValueError("The remote file shrank during download.")
                out.write(chunk)
                left -= len(chunk)


def rename(path, filename, target_path, target):
    # renameat2's NOREPLACE protects existing files even when another process races us.
    with directory_fd(path) as directory, directory_fd(target_path) as destination:
        arguments = (directory, os.fsencode(name(filename)), destination, os.fsencode(name(target)), 1)
        if hasattr(LIBC, "renameat2"):
            result = LIBC.renameat2(*arguments)
        elif (number := {"x86_64": 316, "aarch64": 276}.get(ARCH)) is not None:
            result = LIBC.syscall(ctypes.c_long(number), *arguments)
        else:
            raise ValueError("Safe no-overwrite rename is unavailable on this image.")
        if result:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
    print("OK")


def tree(directory, entry, removing, counter):
    info = os.stat(entry, dir_fd=directory, follow_symlinks=False)
    if stat.S_ISDIR(info.st_mode):
        fd = os.open(entry, FLAGS, dir_fd=directory)
        try:
            with os.scandir(fd) as entries:
                for child in entries:
                    tree(fd, child.name, removing, counter)
        finally:
            os.close(fd)
        if removing:
            os.rmdir(entry, dir_fd=directory)
    elif removing:
        os.unlink(entry, dir_fd=directory)
    counter[0] += 1
    if removing and counter[0] % 32 == 0:
        print(f"PROGRESS {counter[0]}", flush=True)


def delete(path, filename):
    with directory_fd(path) as directory:
        entry = name(filename)
        total = [0]
        tree(directory, entry, False, total)
        print(f"RECTDELETE1 {total[0]}", flush=True)
        done = [0]
        tree(directory, entry, True, done)
    print("OK", flush=True)


def edit(path, filename):
    with directory_fd(path) as directory:
        filename = name(filename)
        source, _ = regular(directory, filename)
        source.close()
        os.fchdir(directory)
    os.execvp("vim", ("vim", "--", filename))


if __name__ == "__main__":
    try:
        operation, path, *arguments = sys.argv[1:]
        path = decode(path)
        if not path.startswith("/"):
            raise ValueError("Expected an absolute container path.")
        if operation == "list":
            listing(path, int(arguments[0]), arguments[1:] == ["desc"])
        elif operation == "validate-share":
            with directory_fd(path):
                pass
        elif operation == "upload":
            upload(path, decode(arguments[0]), int(arguments[1]))
        elif operation in ("download", "delete", "edit"):
            {"download": download, "delete": delete, "edit": edit}[operation](path, decode(arguments[0]))
        elif operation in ("mkdir", "new-file"):
            create(path, decode(arguments[0]), operation == "mkdir")
        elif operation == "rename":
            rename(path, *(decode(value) for value in arguments))
        else:
            raise ValueError("Unknown file operation.")
    except (OSError, ValueError, RecursionError, IndexError) as error:
        sys.exit(str(error))
