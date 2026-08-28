#!/usr/bin/env python3
"""Fail-closed repository policy checks used by hosted CI.

The scanner intentionally uses only the Python standard library and Git. It reads the exact set of
tracked files from the index, never a best-effort filesystem glob.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from collections.abc import Iterator, Mapping
from pathlib import Path, PurePosixPath
from typing import Any
from urllib.parse import parse_qs, urlsplit


ROOT = Path(__file__).resolve().parents[1]
FULL_GIT_SHA = re.compile(r"[0-9a-fA-F]{40}")
CREDENTIAL_ASSIGNMENT = re.compile(
    r"""(?ix)
    \b(?:
        api[_-]?key | api[_-]?secret | client[_-]?secret | private[_-]?key |
        mnemonic | password | access[_-]?token | auth[_-]?token
    )
    \s*[:=]\s*
    (?:["'][^"'\r\n]{12,}["']|[A-Za-z0-9+/=_-]{12,})
    """
)
KNOWN_TOKEN = re.compile(
    r"""(?ix)\b(?:
        gh[pousr]_[A-Za-z0-9]{20,} |
        github_pat_[A-Za-z0-9_]{20,} |
        sk-(?:proj-)?[A-Za-z0-9_-]{20,} |
        A(?:KI|SI)A[0-9A-Z]{16} |
        0x[0-9a-f]{64}
    )\b"""
)
PRIVATE_KEY_HEADER = re.compile(r"-----BEGIN(?: [A-Z0-9]+)? PRIVATE KEY-----")
SENSITIVE_SUFFIXES = {
    ".key",
    ".pem",
    ".p12",
    ".pfx",
}
BUILD_SUFFIXES = {
    ".dll",
    ".dylib",
    ".exe",
    ".o",
    ".obj",
    ".pdb",
    ".profdata",
    ".profraw",
    ".rlib",
    ".rmeta",
    ".so",
}


class PolicyFailure(RuntimeError):
    """A policy could not be evaluated or found a violation."""


def git_output(*arguments: str) -> bytes:
    """Run Git and return stdout, treating every error as a hard failure."""
    process = subprocess.run(
        ["git", "-C", str(ROOT), *arguments],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.returncode != 0:
        stderr = process.stderr.decode("utf-8", errors="replace").strip()
        raise PolicyFailure(f"git {' '.join(arguments)} failed: {stderr}")
    return process.stdout


def tracked_files() -> tuple[PurePosixPath, ...]:
    """Return validated tracked paths from Git's NUL-delimited index output."""
    raw_paths = git_output("ls-files", "-z")
    if not raw_paths:
        raise PolicyFailure("git ls-files returned no tracked files")

    paths: list[PurePosixPath] = []
    for raw_path in raw_paths.split(b"\0"):
        if not raw_path:
            continue
        try:
            decoded = raw_path.decode("utf-8")
        except UnicodeDecodeError as error:
            raise PolicyFailure("a tracked path is not valid UTF-8") from error
        path = PurePosixPath(decoded)
        if path.is_absolute() or ".." in path.parts:
            raise PolicyFailure(f"unsafe tracked path: {path}")
        disk_path = ROOT.joinpath(*path.parts)
        if not disk_path.is_file():
            raise PolicyFailure(f"tracked file is missing from the checkout: {path}")
        paths.append(path)

    if not paths:
        raise PolicyFailure("no valid tracked files were returned by Git")
    return tuple(sorted(paths, key=str))


def read_tracked_text(path: PurePosixPath) -> str:
    """Read one tracked file as strict UTF-8 and reject binary content."""
    data = ROOT.joinpath(*path.parts).read_bytes()
    if b"\0" in data:
        raise PolicyFailure(f"tracked binary file requires explicit review: {path}")
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PolicyFailure(f"tracked file is not valid UTF-8: {path}") from error


def sensitive_path_reason(path: PurePosixPath) -> str | None:
    """Explain why a tracked filename is forbidden, if applicable."""
    lowered = PurePosixPath(str(path).lower())
    parts = lowered.parts
    name = lowered.name
    suffix = lowered.suffix

    if parts and parts[0] in {"target", "coverage", "recordings", "secrets"}:
        return "forbidden generated or sensitive directory"
    if name == ".envrc" or (name == ".env" or name.startswith(".env.")) and name != ".env.example":
        return "environment file"
    if name in {"id_rsa", "id_ed25519", "credentials", "credentials.toml"}:
        return "credential or private-key file"
    if name.startswith("credentials") and suffix == ".json":
        return "credential JSON file"
    if suffix in SENSITIVE_SUFFIXES:
        return "private-key or certificate file"
    if suffix in BUILD_SUFFIXES:
        return "build artifact"
    if len(parts) >= 2 and parts[0] == "config" and (
        name in {"live.toml", "local.toml"} or name.startswith("secrets.")
    ):
        return "local or secret configuration"
    return None


def line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def scan_credentials(paths: tuple[PurePosixPath, ...]) -> None:
    """Reject sensitive filenames and credential-shaped tracked content."""
    findings: list[str] = []
    for path in paths:
        reason = sensitive_path_reason(path)
        if reason is not None:
            findings.append(f"{path}: {reason}")
            continue

        text = read_tracked_text(path)
        for label, pattern in (
            ("credential assignment", CREDENTIAL_ASSIGNMENT),
            ("known token format", KNOWN_TOKEN),
            ("private-key header", PRIVATE_KEY_HEADER),
        ):
            match = pattern.search(text)
            if match is not None:
                findings.append(f"{path}:{line_number(text, match.start())}: {label}")

    if findings:
        details = "\n".join(f"  - {finding}" for finding in findings)
        raise PolicyFailure(f"credential policy violations:\n{details}")


def walk_git_dependencies(node: Any, location: str = "root") -> Iterator[tuple[str, Mapping[str, Any]]]:
    """Yield every TOML table that declares a Git source."""
    if isinstance(node, Mapping):
        if "git" in node:
            yield location, node
        for key, value in node.items():
            yield from walk_git_dependencies(value, f"{location}.{key}")
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from walk_git_dependencies(value, f"{location}[{index}]")


def scan_cargo_git(paths: tuple[PurePosixPath, ...]) -> None:
    """Require all Cargo Git dependencies and lock sources to use a full exact revision."""
    manifests = tuple(path for path in paths if path.name == "Cargo.toml")
    if not manifests:
        raise PolicyFailure("no tracked Cargo.toml files found")

    findings: list[str] = []
    manifest_revisions: set[str] = set()
    for manifest in manifests:
        try:
            parsed = tomllib.loads(read_tracked_text(manifest))
        except tomllib.TOMLDecodeError as error:
            raise PolicyFailure(f"failed to parse {manifest}: {error}") from error
        for location, dependency in walk_git_dependencies(parsed):
            revision = dependency.get("rev")
            if "branch" in dependency or "tag" in dependency:
                findings.append(f"{manifest}:{location}: branch/tag Git dependency is forbidden")
            if not isinstance(revision, str) or FULL_GIT_SHA.fullmatch(revision) is None:
                findings.append(f"{manifest}:{location}: full 40-character rev is required")
            else:
                manifest_revisions.add(revision.lower())

    lock_path = PurePosixPath("Cargo.lock")
    if lock_path not in paths:
        raise PolicyFailure("Cargo.lock is not tracked")
    try:
        lock = tomllib.loads(read_tracked_text(lock_path))
    except tomllib.TOMLDecodeError as error:
        raise PolicyFailure(f"failed to parse Cargo.lock: {error}") from error

    for package in lock.get("package", []):
        source = package.get("source")
        if not isinstance(source, str) or not source.startswith("git+"):
            continue
        parsed_source = urlsplit(source.removeprefix("git+"))
        revisions = parse_qs(parsed_source.query, strict_parsing=True).get("rev", [])
        revision = revisions[0] if len(revisions) == 1 else None
        locked_commit = parsed_source.fragment
        package_name = package.get("name", "<unknown>")
        if revision is None or FULL_GIT_SHA.fullmatch(revision) is None:
            findings.append(f"Cargo.lock:{package_name}: exact rev query is required")
            continue
        if FULL_GIT_SHA.fullmatch(locked_commit) is None or locked_commit.lower() != revision.lower():
            findings.append(f"Cargo.lock:{package_name}: locked commit must equal the exact rev")
        if revision.lower() not in manifest_revisions:
            findings.append(f"Cargo.lock:{package_name}: rev is absent from tracked manifests")

    if findings:
        details = "\n".join(f"  - {finding}" for finding in findings)
        raise PolicyFailure(f"Cargo Git reproducibility violations:\n{details}")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "check",
        nargs="?",
        choices=("all", "cargo-git", "credentials"),
        default="all",
    )
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    paths = tracked_files()
    if arguments.check in {"all", "cargo-git"}:
        scan_cargo_git(paths)
    if arguments.check in {"all", "credentials"}:
        scan_credentials(paths)
    print(f"repository policy checks passed: {arguments.check} ({len(paths)} tracked files)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PolicyFailure as error:
        print(f"repository policy check failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
