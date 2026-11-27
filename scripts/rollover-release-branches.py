#!/usr/bin/env python3
"""After a verified stable publication, retire only its fully merged exact dev ref."""

import json
import os
import re
import subprocess
import release_preflight
from pathlib import Path

TAG = re.compile(r"v(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})")
SHA = re.compile(r"[0-9a-f]{40}")


class Hold(Exception):
    """The refs did not prove a safe rollover."""


def git(*args, check=True):
    return subprocess.run(["git", *args], check=check, text=True, stdout=subprocess.PIPE).stdout


def remote_refs():
    refs = {}
    for line in git("ls-remote", "origin", "refs/heads/*", "refs/tags/*").splitlines():
        sha, name = line.split("\t", 1)
        if not SHA.fullmatch(sha) or name in refs:
            raise Hold("Unexpected remote ref response.")
        refs[name] = sha
    return refs


def variants(refs, version, expected, prefix):
    check = re.compile(rf"(?:^|[/v-]){re.escape(version)}(?:$|[/-])")
    return any(name.startswith(prefix) and name != expected and check.search(name[len(prefix):])
               for name in refs if not name.endswith("^{}"))


def rollover(event):
    release = event.get("release", {})
    if event.get("action") != "published" or release.get("draft") is not False or release.get("prerelease") is not False:
        return "Skipped non-stable publication."
    version_tag = release.get("tag_name", "")
    match = TAG.fullmatch(version_tag) if isinstance(version_tag, str) else None
    if not match:
        return "Skipped malformed or non-stable version tag."
    # This repository uses only main as its release branch.
    main = "refs/heads/main"
    if event.get("repository", {}).get("default_branch") != "main":
        raise Hold("Unexpected default branch.")
    if f"ref: {main}\tHEAD" not in git("ls-remote", "--symref", "origin", "HEAD").splitlines():
        raise Hold("Remote HEAD no longer points to main.")
    major, minor, patch = map(int, match.groups())
    retiring = f"{major}.{minor}.{patch}"
    upcoming = f"{major}.{minor}.{patch + 1}"
    tag = f"refs/tags/{version_tag}"
    old = f"refs/heads/{retiring}-dev"
    new = f"refs/heads/{upcoming}-dev"
    refs = remote_refs()
    tag_object = refs.get(tag)
    released = refs.get(f"{tag}^{{}}", tag_object)
    if not released or refs.get(main) != released:
        raise Hold("Release tag does not resolve to the current default-branch commit.")
    if variants(refs, retiring, old, "refs/heads/") or variants(refs, upcoming, new, "refs/heads/") or variants(refs, retiring, tag, "refs/tags/"):
        raise Hold("Ambiguous same-version reference.")
    old_sha = refs.get(old)
    new_sha = refs.get(new)
    if not old_sha:
        if new_sha == released:
            return "Rollover already complete."
        raise Hold("No exact development branch to retire.")
    if new_sha and new_sha != released:
        raise Hold("Replacement branch exists at a different commit. Neither branch was changed.")
    git("fetch", "--no-tags", "--no-write-fetch-head", "origin", main, old)
    result = subprocess.run(["git", "merge-base", "--is-ancestor", old_sha, released], check=False)
    if result.returncode != 0:
        raise Hold("Development branch has unique or unverifiable work.")

    if not new_sha:
        # Require absence on the server. A racing creation cannot be advanced.
        git("push", f"--force-with-lease={new}:", "origin", f"{released}:{new}")
    recheck = remote_refs()
    if (recheck.get(new), recheck.get(old), recheck.get(main), recheck.get(tag)) != (released, old_sha, released, tag_object):
        raise Hold("A verified ref moved. The retiring branch was not touched.")
    if (variants(recheck, retiring, old, "refs/heads/") or variants(recheck, upcoming, new, "refs/heads/")
            or variants(recheck, retiring, tag, "refs/tags/")):
        raise Hold("An ambiguous same-version reference appeared. The retiring branch was not touched.")
    # The explicit lease is a compare-and-delete against the exact old tip.
    # It never rewrites a branch and fails if another writer advances that tip.
    git("push", f"--force-with-lease={old}:{old_sha}", "origin", f":{old}")
    after = remote_refs()
    if old in after or after.get(new) != released:
        raise Hold("Could not certify the final branch state.")
    return f"Retired {retiring}-dev and verified {upcoming}-dev at {released}."


if __name__ == "__main__":
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        release_preflight.check_published_event(event)
        print(rollover(event))
    except (Hold, release_preflight.Hold, OSError) as error:
        print(f"Rollover not completed. {error}")
        raise SystemExit(1) from None
    except subprocess.CalledProcessError:
        print("A Git command failed. Inspect both refs before retrying.")
        raise SystemExit(1) from None
