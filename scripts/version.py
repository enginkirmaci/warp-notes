#!/usr/bin/env python3
"""This app's version and build counter.

`version.toml` is the source of truth: `version` (MAJOR.MINOR.PATCH) and
`build` (a monotonic counter). Every commit bumps PATCH through the
pre-commit hook; every build-script run bumps build. `Cargo.toml` (and the
path-dependency manifests) and `Cargo.lock` are derived from this file.

Usage:
  version.py show [--json] [--next]
  version.py bump {patch|minor|major}
  version.py build bump
  version.py sync
  version.py check
  version.py pre-commit [--dry-run]
  version.py install-hooks
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_FILE = ROOT / "version.toml"
LOCK_FILE = ROOT / "Cargo.lock"
HOOKS_DIR = ".githooks"
SKIP_DIRS = {".git", "target", "out", "dist", ".build"}

SEMVER = re.compile(
    r"^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)"
    r"(?:-(?P<pre>[0-9A-Za-z.-]+))?$"
)


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"version: {message}")


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=check,
    )


def in_git() -> bool:
    return git("rev-parse", "--git-dir", check=False).returncode == 0


def read_facts() -> dict:
    if not VERSION_FILE.is_file():
        fail(f"{VERSION_FILE} is missing")
    data = tomllib.loads(VERSION_FILE.read_text(encoding="utf-8"))
    version = str(data.get("version", "")).strip()
    build = data.get("build", 0)
    if not SEMVER.match(version):
        fail(f"version.toml version {version!r} is not MAJOR.MINOR.PATCH")
    if not isinstance(build, int) or build < 0:
        fail(f"version.toml build {build!r} is not a non-negative integer")
    return {"version": version, "build": build}


@contextmanager
def version_lock():
    fh = open(VERSION_FILE, "r+", encoding="utf-8")
    try:
        try:
            import fcntl

            fcntl.flock(fh.fileno(), fcntl.LOCK_EX)
            unlock = lambda: fcntl.flock(fh.fileno(), fcntl.LOCK_UN)  # noqa: E731
        except ImportError:
            import msvcrt

            fh.seek(0)
            msvcrt.locking(fh.fileno(), msvcrt.LK_LOCK, 1)

            def unlock():
                fh.seek(0)
                msvcrt.locking(fh.fileno(), msvcrt.LK_UNLCK, 1)

        try:
            yield fh
        finally:
            unlock()
    finally:
        fh.close()


def write_version_text(text: str, version: str, build: int) -> str:
    text, n = re.subn(
        r'(?m)^(\s*version\s*=\s*")[^"]*(")',
        rf"\g<1>{version}\2",
        text,
        count=1,
    )
    if n != 1:
        fail("version.toml has no `version = ` line")
    text, n = re.subn(r"(?m)^(\s*build\s*=\s*)\d+", rf"\g<1>{build}", text, count=1)
    if n != 1:
        fail("version.toml has no `build = ` line")
    return text


def mutate_version(change):
    with version_lock() as fh:
        raw = fh.read()
        data = tomllib.loads(raw)
        version = str(data["version"]).strip()
        build = int(data.get("build", 0))
        new_version, new_build = change(version, build)
        fh.seek(0)
        fh.truncate()
        fh.write(write_version_text(raw, new_version, new_build))
        fh.flush()
        os.fsync(fh.fileno())
    return new_version, new_build


def bump_semver(version: str, kind: str) -> str:
    match = SEMVER.match(version)
    assert match is not None
    major, minor, patch = (int(match.group(k)) for k in ("major", "minor", "patch"))
    if match.group("pre"):
        fail(f"cannot bump {version}: drop the pre-release suffix first")
    if kind == "patch":
        patch += 1
    elif kind == "minor":
        minor += 1
    elif kind == "major":
        major += 1
    else:
        fail(f"unknown bump kind {kind!r}")
    return f"{major}.{minor}.{patch}"


def git_facts(version: str) -> dict:
    if not in_git():
        return {"commit": "unknown", "channel": "dev", "released": ""}
    head = git("rev-parse", "--short=7", "HEAD").stdout.strip()
    dirty = any(
        line and not line.endswith(" version.toml")
        for line in git("status", "--porcelain", "--untracked-files=normal").stdout.splitlines()
    )
    if dirty:
        head = f"{head}-dirty"
    tags = git("tag", "--points-at", "HEAD", "--list", "v*").stdout.split()
    tagged = f"v{version}" in tags or os.environ.get("GITHUB_REF") == f"refs/tags/v{version}"
    if tagged:
        pre = SEMVER.match(version).group("pre") if SEMVER.match(version) else None
        if pre:
            channel = next((name for name in ("alpha", "beta", "rc") if name in pre), "pre")
        else:
            channel = "stable"
        released = git("log", "-1", "--format=%cs", "HEAD").stdout.strip()
    else:
        channel = "dev"
        released = ""
    return {"commit": head, "channel": channel, "released": released}


def facts(version: str | None = None) -> dict:
    data = read_facts()
    if version is None:
        version = data["version"]
    return {"version": version, "build": data["build"], **git_facts(version)}


# ---------------------------------------------------------------------------
# Cargo sync


def manifests() -> list[Path]:
    return sorted(
        path
        for path in ROOT.rglob("Cargo.toml")
        if SKIP_DIRS.isdisjoint(path.relative_to(ROOT).parts)
    )


def replace_package_version(text: str, version: str) -> str:
    lines = text.splitlines(keepends=True)
    in_package = False
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_package = stripped == "[package]"
            continue
        if in_package and re.match(r"^version\s*=", stripped):
            lines[index] = re.sub(r'"[^"]*"', f'"{version}"', line, count=1)
            return "".join(lines)
    fail("a manifest has no [package] version line")


def find_cargo() -> str:
    cargo = shutil.which("cargo")
    if cargo:
        return cargo
    local = Path.home() / ".cargo" / "bin" / ("cargo.exe" if os.name == "nt" else "cargo")
    if local.is_file():
        return str(local)
    fail("cargo is not on PATH; a version bump must refresh Cargo.lock")


def sync_cargo(version: str) -> None:
    for path in manifests():
        text = path.read_text(encoding="utf-8")
        new = replace_package_version(text, version)
        if new != text:
            path.write_text(new, encoding="utf-8")

    cargo = find_cargo()
    for extra in (["--offline"], []):
        run = subprocess.run(
            [cargo, "update", "--workspace", *extra],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        if run.returncode == 0:
            break
    else:
        fail(f"cargo update --workspace failed:\n{run.stderr.strip()}")
    check_locked()


def check_locked() -> dict:
    cargo = find_cargo()
    run = subprocess.run(
        [cargo, "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if run.returncode != 0:
        fail(f"Cargo.lock is stale; run `cargo update --workspace`:\n{run.stderr.strip()}")
    return json.loads(run.stdout)


def stage(paths: list[Path]) -> None:
    if not in_git():
        return
    relative = [str(path.relative_to(ROOT)) for path in paths]
    run = git("add", "--", *relative, check=False)
    if run.returncode != 0:
        fail(f"git add failed: {run.stderr.strip()}")


# ---------------------------------------------------------------------------
# commands


def cmd_show(argv: list[str]) -> int:
    as_json = "--json" in argv
    next_only = "--next" in argv
    current = read_facts()
    version = current["version"]
    if next_only:
        version = bump_semver(version, "patch")
    data = facts(version)
    if next_only:
        print(data["version"])
        return 0
    if as_json:
        print(json.dumps(data, sort_keys=True))
    else:
        released = f" · released {data['released']}" if data["released"] else ""
        print(f"{data['version']} build {data['build']} · {data['channel']} · {data['commit']}{released}")
    return 0


def cmd_bump(argv: list[str]) -> int:
    if not argv or argv[0] not in {"patch", "minor", "major"}:
        fail("usage: version.py bump {patch|minor|major}")
    kind = argv[0]
    no_stage = "--no-stage" in argv
    old, _ = read_facts()
    new, _ = mutate_version(lambda version, build: (bump_semver(version, kind), build))
    sync_cargo(new)
    if not no_stage:
        stage([VERSION_FILE, LOCK_FILE, *manifests()])
    print(f"version: {old} -> {new}")
    return 0


def cmd_build(argv: list[str]) -> int:
    if argv != ["bump"]:
        fail("usage: version.py build bump")
    _, build = mutate_version(lambda version, current: (version, current + 1))
    print(build)
    return 0


def cmd_sync(argv: list[str]) -> int:
    if argv:
        fail("usage: version.py sync")
    current = read_facts()
    sync_cargo(current["version"])
    print(f"version: {current['version']}")
    return 0


def cmd_pre_commit(argv: list[str]) -> int:
    dry_run = "--dry-run" in argv
    if os.environ.get("WARP_VERSION_SKIP") == "1":
        print("version: skipped (WARP_VERSION_SKIP=1)")
        return 0
    current = read_facts()
    new = bump_semver(current["version"], "patch")
    if dry_run:
        print(f"version: {current['version']} -> {new}; would stage version.toml, Cargo.toml files, Cargo.lock")
        return 0
    mutate_version(lambda version, build: (new, build))
    sync_cargo(new)
    stage([VERSION_FILE, LOCK_FILE, *manifests()])
    print(f"version: {current['version']} -> {new} (build {current['build']})")
    return 0


def check_errors() -> list[str]:
    data = read_facts()
    version = data["version"]
    errors: list[str] = []
    metadata = check_locked()
    members = set(metadata.get("workspace_members", []))
    packages = [p for p in metadata.get("packages", []) if p.get("id") in members]
    if not packages:
        errors.append("cargo metadata returned no workspace member")
    for package in packages:
        if package.get("version") != version:
            errors.append(
                f"{package.get('manifest_path')}: {package.get('name')} "
                f"is {package.get('version')}, version.toml is {version}"
            )
        manifest = Path(package["manifest_path"])
        text = manifest.read_text(encoding="utf-8")
        if re.search(r"(?m)^\s*version\.workspace\s*=\s*true", text):
            errors.append(f"{manifest.relative_to(ROOT)}: version.workspace inherited; use a literal version")

    app_file = ROOT / "app.toml"
    if app_file.is_file():
        app = tomllib.loads(app_file.read_text(encoding="utf-8")).get("app", {})
        package = app.get("package")
        names = {p.get("name") for p in packages}
        if package and package not in names:
            errors.append(f"app.toml package {package!r} is not a cargo package")
    return errors


def cmd_check(argv: list[str]) -> int:
    errors = check_errors()
    if errors:
        for error in errors:
            print(f"version: {error}", file=sys.stderr)
        return 1
    data = read_facts()
    print(f"version: {data['version']} build {data['build']} is consistent")
    return 0


def cmd_install_hooks(argv: list[str]) -> int:
    if argv:
        fail("usage: version.py install-hooks")
    if not in_git():
        fail("not a git checkout")
    hook = ROOT / HOOKS_DIR / "pre-commit"
    if not hook.is_file():
        fail(f"{hook.relative_to(ROOT)} is missing")
    hook.chmod(0o755)
    git("config", "core.hooksPath", HOOKS_DIR)
    print(f"version: core.hooksPath -> {HOOKS_DIR}")
    return 0


COMMANDS = {
    "show": cmd_show,
    "bump": cmd_bump,
    "build": cmd_build,
    "sync": cmd_sync,
    "check": cmd_check,
    "pre-commit": cmd_pre_commit,
    "install-hooks": cmd_install_hooks,
}


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in COMMANDS:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    return COMMANDS[argv[0]](argv[1:])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
