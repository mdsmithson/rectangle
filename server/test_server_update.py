"""Exercise update locking, release selection and failure isolation."""
import concurrent.futures
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

if os.name == "posix":
    import release
    spec = importlib.util.spec_from_file_location("server_update", Path(__file__).with_name("server-update.py"))
    updater = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(updater)

COMMIT = "a" * 40


@unittest.skipUnless(os.name == "posix", "requires POSIX file locking")
class ServerUpdateTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.current = self.root / "current-release"
        self.enterContext(patch.multiple(release, ROOT=self.root, CURRENT=self.current))

    def record(self, current):
        self.current.mkdir(exist_ok=True)
        (self.current / "release.json").write_text(json.dumps({"version": current, "commit": COMMIT}))

    def test_same_or_newer_does_not_read_github_or_build_images(self):
        self.record("0.1.12")
        with patch.object(release, "command", side_effect=AssertionError("unexpected command")):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                updater.update(COMMIT, "0.1.11")
                updater.update(COMMIT, "0.1.12")
            self.assertEqual(output.getvalue(), "RECTSERVERUPDATE1 skipped 0.1.12\n" * 2)

    def test_only_one_client_applies_an_update(self):
        self.record("0.1.9")
        (self.root / "allow-server-updates").touch()
        calls = []

        def fake_command(*args, **kwargs):
            if args[0] == "git":
                source = Path(kwargs["cwd"])
                (source / "Cargo.toml").write_text('[package]\nversion = "0.1.10"\n')
            else:
                calls.append(args)
                self.assertTrue(kwargs["pass_fds"])
                time.sleep(0.05)
                self.record("0.1.10")

        output = io.StringIO()
        with patch.object(release, "command", side_effect=fake_command), contextlib.redirect_stdout(output):
            with concurrent.futures.ThreadPoolExecutor(2) as pool:
                futures = [pool.submit(updater.update, COMMIT, "0.1.10") for _ in range(2)]
                for future in futures:
                    future.result(timeout=5)
        self.assertEqual(len(calls), 1)
        self.assertCountEqual(output.getvalue().splitlines(),
                              ["RECTSERVERUPDATE1 updated 0.1.10", "RECTSERVERUPDATE1 skipped 0.1.10"])

    def test_failed_build_keeps_old_release_and_no_opt_in_blocks_fetch(self):
        old = self.root / "old"
        old.mkdir()
        (old / "release.json").write_text(json.dumps({"version": "0.1.9", "commit": COMMIT}))
        self.current.symlink_to(old, target_is_directory=True)
        with patch.object(release, "command", side_effect=AssertionError("unexpected fetch")):
            with self.assertRaisesRegex(ValueError, "disabled"):
                updater.update(COMMIT, "0.1.10")
        source = self.root / "source"
        source.mkdir()
        (source / "Cargo.toml").write_text('version = "0.1.10"\n')
        with patch.object(release, "command"), patch.object(release, "stage_images", side_effect=OSError("test failure")):
            with self.assertRaisesRegex(OSError, "test failure"):
                release.activate(source, COMMIT, "0.1.10")
        self.assertEqual(self.current.resolve(), old.resolve())

    def test_activation_publishes_one_coherent_release(self):
        source = Path(__file__).resolve().parents[1]
        target = release.source_version(source)
        images = {key: f"rectangle-{key}:fixture" for key in release.IMAGES}
        with patch.object(release, "stage_images", return_value=images):
            new = release.activate(source, COMMIT, target)
        self.assertEqual(self.current.resolve(), new.resolve())
        self.assertEqual(release.installed(), {"version": target, "commit": COMMIT})
        self.assertIn("rectangle_agent_image=rectangle-agent:fixture", (self.current / "images.sh").read_text())
        self.assertTrue((self.current / "agent-context/agent-entrypoint").is_file())

    def test_only_images_with_changed_inputs_get_new_tags(self):
        for inputs in release.IMAGES.values():
            for entry in inputs:
                (self.root / entry).write_text("before")
        tags = lambda: {key: release.image_tag(self.root, key, inputs)
                        for key, inputs in release.IMAGES.items()}
        before = tags()
        (self.root / "Dockerfile.agent").write_text("after")
        after = tags()
        self.assertEqual([key for key in before if before[key] != after[key]], ["agent"])

    def test_images_set_permissions_independently_of_private_checkout(self):
        server = Path(__file__).resolve().parent
        for recipe, *_ in release.IMAGES.values():
            for line in (server / recipe).read_text().splitlines():
                if line.startswith("COPY "):
                    self.assertRegex(line, r"^COPY --chmod=0[45][45][45] ",
                                     f"{recipe} inherits permissions from the private source checkout")
