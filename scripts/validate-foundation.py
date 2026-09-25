#!/usr/bin/env python3
"""Validate the portable Product Mission Control foundation artifacts.

This script intentionally uses only the Python standard library so a fresh
contributor can run it without a machine-specific skill installation.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import unicodedata
from pathlib import Path
from urllib.parse import unquote


REQUIRED_FILES = {
    "README.md": ("# Product Mission Control", "## Build from source"),
    "SECURITY.md": ("# Security", "## Reporting a vulnerability"),
    "CONTRIBUTING.md": ("# Contributing", "## Verification"),
    "docs/architecture.md": ("# Architecture", "## Authority boundaries"),
    "docs/domain-glossary.md": ("# Domain glossary", "Product Ledger"),
    "docs/design-system.md": ("# Design system", "## Accessibility baseline"),
}

PRIVATE_PATH_PATTERNS = (
    re.compile(r"[A-Za-z]:\\Users\\[^\\\s`]+", re.IGNORECASE),
    re.compile(r"/(?:Users|home)/[^/\s`]+"),
)

MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
MARKDOWN_HEADING = re.compile(r"^\s{0,3}#{1,6}\s+(.+?)\s*$")
SETEXT_HEADING = re.compile(r"^\s{0,3}(?:=+|-+)\s*$")
FENCE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root",
        nargs="?",
        default=".",
        help="Repository root (default: current directory)",
    )
    return parser.parse_args()


def validate_required_files(root: Path, errors: list[str]) -> list[Path]:
    checked: list[Path] = []
    for relative, markers in REQUIRED_FILES.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"missing required file: {relative}")
            continue
        checked.append(path)
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            errors.append(f"not valid UTF-8: {relative}")
            continue
        if not content.strip():
            errors.append(f"required file is empty: {relative}")
        for marker in markers:
            if marker not in content:
                errors.append(f"{relative}: missing required marker: {marker}")
    return checked


def markdown_files(root: Path, errors: list[str]) -> list[Path]:
    """Return tracked and not-ignored Markdown owned by the repository."""
    try:
        result = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "--", "*.md"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        errors.append(f"cannot read tracked Markdown manifest from Git: {exc}")
        return []

    return sorted(root / relative for relative in result.stdout.splitlines() if relative)


def github_heading_slug(heading: str) -> str:
    """Approximate GitHub's documented heading-ID normalization."""
    heading = re.sub(r"\s+#+\s*$", "", heading)
    heading = re.sub(r"!?(?:\[([^\]]*)\])\([^)]*\)", r"\1", heading)
    heading = re.sub(r"<[^>]+>", "", heading)
    heading = heading.replace("`", "").replace("*", "").replace("~", "")
    normalized: list[str] = []
    for character in heading.casefold():
        category = unicodedata.category(character)
        if category.startswith(("P", "S")) and character not in ("-", "_"):
            continue
        normalized.append("-" if character.isspace() else character)
    return "".join(normalized)


def markdown_anchors(content: str) -> set[str]:
    """Collect GitHub-style heading anchors, including duplicate suffixes."""
    anchors: set[str] = set()
    occurrences: dict[str, int] = {}
    fence_marker: str | None = None
    previous_line = ""

    for line in content.splitlines():
        fence = FENCE.match(line)
        if fence:
            marker = fence.group(1)
            if fence_marker is None:
                fence_marker = marker[0]
            elif marker[0] == fence_marker:
                fence_marker = None
            previous_line = ""
            continue
        if fence_marker is not None:
            continue

        match = MARKDOWN_HEADING.match(line)
        heading = match.group(1) if match else ""
        if (
            not heading
            and previous_line.strip()
            and not MARKDOWN_HEADING.match(previous_line)
            and SETEXT_HEADING.match(line)
        ):
            heading = previous_line.strip()
        previous_line = line
        if not heading:
            continue
        base = github_heading_slug(heading)
        if not base:
            continue
        occurrence = occurrences.get(base, 0)
        anchors.add(base if occurrence == 0 else f"{base}-{occurrence}")
        occurrences[base] = occurrence + 1

    return anchors


def split_link_target(raw_target: str) -> tuple[str, str]:
    """Split a Markdown destination into its decoded path and fragment."""
    target = raw_target.strip().strip("<>")
    path_part, separator, fragment = target.partition("#")
    return unquote(path_part), unquote(fragment) if separator else ""


def validate_markdown(root: Path, errors: list[str], warnings: list[str]) -> int:
    files = markdown_files(root, errors)
    content_by_path: dict[Path, str] = {}
    anchors_by_path: dict[Path, set[str]] = {}

    for path in files:
        relative = path.relative_to(root).as_posix()
        try:
            content = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            errors.append(f"not valid UTF-8: {relative}")
            continue
        resolved_path = path.resolve()
        content_by_path[resolved_path] = content
        anchors_by_path[resolved_path] = markdown_anchors(content)

    for path in files:
        resolved_source = path.resolve()
        content = content_by_path.get(resolved_source)
        if content is None:
            continue
        relative = path.relative_to(root).as_posix()

        for pattern in PRIVATE_PATH_PATTERNS:
            if pattern.search(content):
                errors.append(f"{relative}: contains a private user-home path")
                break

        for raw_target in MARKDOWN_LINK.findall(content):
            target = raw_target.strip().strip("<>")
            if not target or target.startswith(("http://", "https://", "mailto:")):
                continue
            target_path, fragment = split_link_target(raw_target)
            resolved = resolved_source if not target_path else (path.parent / target_path).resolve()
            if not resolved.exists():
                errors.append(f"{relative}: broken relative link: {raw_target}")
                continue
            if fragment:
                if resolved.suffix.lower() != ".md":
                    errors.append(f"{relative}: anchor target is not Markdown: {raw_target}")
                    continue
                target_content = content_by_path.get(resolved)
                if target_content is None:
                    try:
                        target_content = resolved.read_text(encoding="utf-8")
                    except (OSError, UnicodeDecodeError):
                        errors.append(f"{relative}: cannot read anchor target: {raw_target}")
                        continue
                    anchors_by_path[resolved] = markdown_anchors(target_content)
                if fragment not in anchors_by_path[resolved]:
                    errors.append(f"{relative}: broken Markdown anchor: {raw_target}")

        if "\t" in content:
            warnings.append(f"{relative}: contains tab characters")
    return len(files)


def main() -> int:
    root = Path(parse_args().root).resolve()
    errors: list[str] = []
    warnings: list[str] = []

    if not (root / ".git").exists():
        errors.append(f"not a Git repository root: {root}")

    checked_required = validate_required_files(root, errors)
    markdown_count = validate_markdown(root, errors, warnings)

    print("Product Mission Control foundation validation")
    print("Required files checked:")
    for path in checked_required:
        print(f"  - {path.relative_to(root).as_posix()}")
    print(f"Markdown files checked: {markdown_count}")
    print("Checks: repository Markdown manifest, required files/markers, UTF-8, relative links/anchors, private user-home paths")

    for warning in warnings:
        print(f"WARNING: {warning}")
    for error in errors:
        print(f"ERROR: {error}")

    print(f"Result: {len(errors)} error(s), {len(warnings)} warning(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
