"""Accept only numbered releases reachable from the official GitHub main branch."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release

REPOSITORY = "https://github.com/mdsmithson/rectangle.git"


def update(commit, target):
    if not release.SHA.fullmatch(commit):
        raise ValueError("Invalid update revision.")
    requested = release.version(target)
    with release.lock() as lock_fd:
        active = release.installed()
        if active and release.version(active["version"]) >= requested:
            choice = "skipped"
            current = active["version"]
        else:
            if not (release.ROOT / "allow-server-updates").is_file():
                raise ValueError("Updates from Rectangle are disabled on this host. Enable them in the server installer.")
            with tempfile.TemporaryDirectory(prefix="source-update-", dir=release.ROOT) as staging:
                env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_CONFIG_NOSYSTEM="1",
                           GIT_CONFIG_GLOBAL=os.devnull)
                def git(*args):
                    release.command("git", "-c", "core.hooksPath=/dev/null", *args,
                                    cwd=staging, env=env, stdout=sys.stderr, timeout=180)

                git("init", "--quiet")
                git("fetch", "--quiet", "--no-tags", REPOSITORY, "refs/heads/main:refs/remotes/official/main")
                git("merge-base", "--is-ancestor", commit, "refs/remotes/official/main")
                git("checkout", "--quiet", "--detach", commit)
                source = Path(staging)
                if release.source_version(source) != target:
                    raise ValueError("GitHub did not confirm the requested server version.")
                # Use the install plan that ships with the verified release, while
                # this process retains the lock through activation and verification.
                release.command(sys.executable, "-I", str(source / "server/release-activate.py"),
                                commit, target, stdout=sys.stderr, timeout=9500, pass_fds=(lock_fd,))
                activated = release.installed()
                if not activated or activated["version"] != target or activated["commit"] != commit:
                    raise ValueError("The host did not activate the requested release.")
            choice, current = "updated", target
    print(f"RECTSERVERUPDATE1 {choice} {current}", flush=True)


if __name__ == "__main__":
    try:
        update(*sys.argv[1:])
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as error:
        sys.exit(f"Rectangle server update failed: {error}")
