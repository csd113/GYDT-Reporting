#!/usr/bin/env python3
"""Run against private throwaway bare repos, never a GitHub remote."""
import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("rollover", Path(__file__).with_name("rollover-release-branches.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

EVENT = {"action":"published", "repository":{"default_branch":"main"},
         "release":{"draft":False,"prerelease":False,"tag_name":"v1.0.5"}}
OLD = "refs/heads/1.0.5-dev"
NEW = "refs/heads/1.0.6-dev"


class RolloverTests(unittest.TestCase):
    def setUp(self):
        root = tempfile.TemporaryDirectory()
        self.addCleanup(root.cleanup)
        old_cwd = Path.cwd()
        self.addCleanup(os.chdir, old_cwd)
        os.chdir(root.name)
        module.git("init", "--bare", "-q", "--initial-branch=main", "remote.git")
        module.git("init", "-q", "--initial-branch=main", "repo")
        os.chdir("repo")
        module.git("config", "user.name", "Synthetic QA")
        module.git("config", "user.email", "qa@example.invalid")
        module.git("remote", "add", "origin", "../remote.git")
        module.git("commit", "-qm", "base", "--allow-empty")
        self.base = module.git("rev-parse", "HEAD").strip()
        module.git("commit", "-qm", "released", "--allow-empty")
        self.released = module.git("rev-parse", "HEAD").strip()
        module.git("tag", "-am", "stable", "v1.0.5")
        module.git("push", "-q", "origin", "main", "v1.0.5", f"{self.base}:{OLD}")

    def refs(self):
        return module.remote_refs()

    def test_stable_then_idempotent(self):
        self.assertIn("Retired", module.rollover(EVENT))
        self.assertNotIn(OLD, self.refs())
        self.assertEqual(self.refs()[NEW], self.released)
        self.assertIn("already complete", module.rollover(EVENT))

    def test_divergence_ambiguity_and_prerelease_remain_untouched(self):
        module.git("switch", "--detach", "-q", self.base)
        module.git("commit", "-qm", "unique", "--allow-empty")
        module.git("push", "-q", "origin", f"HEAD:{OLD}")
        before = self.refs()
        with self.assertRaises(module.Hold):
            module.rollover(EVENT)
        self.assertEqual(before, self.refs())
        changed = EVENT | {"release": EVENT["release"] | {"prerelease": True}}
        self.assertIn("Skipped", module.rollover(changed))
        self.assertEqual(before, self.refs())
        module.git("push", "-q", "origin", f"{self.base}:{NEW}")
        with self.assertRaises(module.Hold):
            module.rollover(EVENT)
        module.git("push", "-q", "origin", f"{self.base}:refs/heads/1.0.5-dev-other")
        with self.assertRaisesRegex(module.Hold, "Ambiguous"):
            module.rollover(EVENT)
        self.assertIn(OLD, self.refs())

    def test_branch_move_after_final_read_is_not_deleted(self):
        original = module.git
        def race(*args, **kwargs):
            if args[0] == "push" and args[-1] == ":" + OLD:
                original("switch", "--detach", "-q", self.released)
                original("commit", "-qm", "racing work", "--allow-empty")
                original("push", "-q", "origin", f"HEAD:{OLD}")
            return original(*args, **kwargs)
        module.git = race
        self.addCleanup(setattr, module, "git", original)
        with self.assertRaises(subprocess.CalledProcessError):
            module.rollover(EVENT)
        self.assertIn(OLD, self.refs())
        self.assertEqual(self.refs()[NEW], self.released)

    def test_release_head_mismatch_holds(self):
        module.git("commit", "-qm", "unreleased", "--allow-empty")
        module.git("push", "-q", "origin", "main")
        before = self.refs()
        with self.assertRaisesRegex(module.Hold, "default-branch"):
            module.rollover(EVENT)
        self.assertEqual(before, self.refs())


if __name__ == "__main__":
    unittest.main()
