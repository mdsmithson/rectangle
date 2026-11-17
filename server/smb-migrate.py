"""Runs inside a networkless throwaway container, with the original volume read only."""
import hashlib
import os
from pathlib import Path
import shutil
import stat
import sys

FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW


def digest(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").digest()


def checked_copy(source, target):
    status = os.stat(source, follow_symlinks=False)
    if not stat.S_ISREG(status.st_mode):
        raise ValueError("A shared folder can only contain regular files, directories, and links.")
    shutil.copy2(source, target, follow_symlinks=False)
    if digest(source) != digest(target):
        raise ValueError("A file changed during the copy; no share was activated.")
    return target


def copy_folder(source_root, target, relative):
    parts = relative.split("/")
    if not parts or any(part in ("", ".", "..") for part in parts):
        raise ValueError("Invalid media folder.")
    fd = os.open(source_root, FLAGS)
    try:
        for part in parts:
            child = os.open(part, FLAGS, dir_fd=fd)
            os.close(fd)
            fd = child
        # Keep the selected directory pinned while copying; never traverse a link
        # in the workspace path. Links inside the tree are copied as links.
        shutil.copytree(Path(f"/proc/self/fd/{fd}"), target, symlinks=True,
                        dirs_exist_ok=True, copy_function=checked_copy)
    finally:
        os.close(fd)


if __name__ == "__main__":
    try:
        copy_folder("/workspace", "/media", sys.argv[1])
    except (OSError, ValueError, shutil.Error) as error:
        sys.exit(f"Could not safely copy the SMB folder. The original is unchanged: {error}")
