"""File Manager operations, always run inside the selected Agent container."""

import contextlib
import os
from pathlib import Path
import shutil
import stat
import sys
import uuid


def decode(field):
    return bytes.fromhex(field).decode()


def listing(path, offset):
    with os.scandir(path) as entries:
        items = [
            (entry.name, entry.stat(follow_symlinks=False))
            for entry in entries
            if entry.name.encode("utf-8", errors="ignore").decode() == entry.name
        ]
    items.sort(key=lambda entry: (not stat.S_ISDIR(entry[1].st_mode), entry[0].casefold(), entry[0]))
    print(f"RECTFILES1 {len(items)}")
    for name, info in items[offset:offset + 150]:
        kind = "d" if stat.S_ISDIR(info.st_mode) else "f" if stat.S_ISREG(info.st_mode) else "o"
        print(kind, info.st_size, name.encode().hex())


def upload(path, name, size):
    home = Path(os.environ["HOME"])
    target = Path(path).relative_to(home)
    if size < 0 or name in ("", ".", "..") or "/" in name or ".." in target.parts:
        raise ValueError("Invalid upload destination.")
    flags = os.O_DIRECTORY | os.O_NOFOLLOW | os.O_RDONLY
    temporary = ".rectangle-upload-" + uuid.uuid4().hex
    with contextlib.ExitStack() as cleanup:
        directory = os.open(home, flags)
        cleanup.callback(os.close, directory)
        for component in target.parts:
            directory = os.open(component, flags, dir_fd=directory)
            cleanup.callback(os.close, directory)
        with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600,
                               dir_fd=directory), "wb") as output:
            cleanup.callback(os.unlink, temporary, dir_fd=directory)
            shutil.copyfileobj(sys.stdin.buffer, output, 256 * 1024)
            if output.tell() != size:
                raise ValueError("Upload was interrupted or the local file changed.")
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, name, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False)
        print("OK")


if __name__ == "__main__":
    try:
        operation, path, *arguments = sys.argv[1:]
        path = decode(path)
        if not path.startswith("/"):
            raise ValueError("Expected an absolute container path.")
        if operation == "list":
            listing(path, int(arguments[0]))
        elif operation == "upload":
            upload(path, decode(arguments[0]), int(arguments[1]))
    except (OSError, ValueError) as error:
        sys.exit(str(error))
