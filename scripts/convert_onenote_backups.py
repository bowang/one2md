#!/usr/bin/env python3
"""Convert only the newest filesystem backup of each OneNote section."""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path


ONENOTE_EXTENSION = re.compile(r"\.one", re.IGNORECASE)
TRAILING_BACKUP_LABEL = re.compile(r"\s*\(\s*On\b.*\)\s*$", re.IGNORECASE)


@dataclass(frozen=True)
class Backup:
    path: Path
    relative_parent: Path
    section: str
    modified_ns: int


def backup_identity(path: Path, root: Path) -> tuple[Path, str] | None:
    if path.suffix.casefold() != ".one":
        return None
    marker = ONENOTE_EXTENSION.search(path.name)
    if marker is None:
        return None
    section = TRAILING_BACKUP_LABEL.sub("", path.name[: marker.start()]).strip()
    if not section:
        return None
    return path.parent.relative_to(root), section


def select_latest_backups(root: Path) -> list[Backup]:
    selected: dict[tuple[str, str], Backup] = {}

    for path in root.rglob("*"):
        if not path.is_file():
            continue
        identity = backup_identity(path, root)
        if identity is None:
            continue
        relative_parent, section = identity
        backup = Backup(path, relative_parent, section, path.stat().st_mtime_ns)
        key = (relative_parent.as_posix().casefold(), section.casefold())
        previous = selected.get(key)
        if previous is None or (backup.modified_ns, str(backup.path)) > (
            previous.modified_ns,
            str(previous.path),
        ):
            selected[key] = backup

    return sorted(
        selected.values(),
        key=lambda backup: (
            backup.relative_parent.as_posix().casefold(),
            backup.section.casefold(),
        ),
    )


def find_converter(explicit: Path | None) -> Path:
    if explicit is not None:
        return explicit
    project_binary = Path(__file__).resolve().parents[1] / "target" / "release" / "one2md"
    if project_binary.is_file():
        return project_binary
    installed = shutil.which("one2md")
    if installed:
        return Path(installed)
    raise FileNotFoundError(
        "one2md was not found; run 'cargo build --release' or pass --one2md"
    )


def stage_backups(backups: list[Backup], staging_root: Path) -> list[Path]:
    staged = []
    for backup in backups:
        destination = staging_root / backup.relative_parent / f"{backup.section}.one"
        destination.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.link(backup.path, destination)
        except OSError:
            shutil.copy2(backup.path, destination)
        staged.append(destination)
    return staged


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Select the newest filesystem backup of every OneNote section and "
            "convert the selected sections in one hierarchy-aware batch."
        )
    )
    parser.add_argument("backup_directory", type=Path)
    parser.add_argument("-o", "--output", type=Path, required=True)
    parser.add_argument("--one2md", type=Path, help="path to the one2md executable")
    parser.add_argument(
        "--no-image-optimization",
        action="store_true",
        help="skip pngquant and jpegoptim size optimization",
    )
    return parser.parse_args()


def converter_command(
    converter: Path,
    staged: list[Path],
    source_root: Path,
    output: Path,
    no_image_optimization: bool,
) -> list[str]:
    command = [
        str(converter),
        *(str(path) for path in staged),
        "--source-root",
        str(source_root),
        "-o",
        str(output),
    ]
    if no_image_optimization:
        command.append("--no-image-optimization")
    return command


def main() -> int:
    started_at = time.perf_counter()
    args = parse_args()
    root = args.backup_directory.resolve()
    if not root.is_dir():
        print(f"error: backup directory does not exist: {root}", file=sys.stderr)
        return 2

    backups = select_latest_backups(root)
    if not backups:
        print(f"error: no .one backup files found under {root}", file=sys.stderr)
        return 1

    try:
        converter = find_converter(args.one2md)
    except FileNotFoundError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"Selected {len(backups)} latest section backup(s):")
    for backup in backups:
        modified = datetime.fromtimestamp(backup.modified_ns / 1_000_000_000).astimezone()
        print(f"  {modified.isoformat(timespec='seconds')}  {backup.path.relative_to(root)}")
    sys.stdout.flush()

    with tempfile.TemporaryDirectory(prefix="one2md-selected-") as staging_directory:
        staged = stage_backups(backups, Path(staging_directory))
        command = converter_command(
            converter,
            staged,
            Path(staging_directory),
            args.output,
            args.no_image_optimization,
        )
        returncode = subprocess.run(command, check=False).returncode

    print(f"Total runtime: {time.perf_counter() - started_at:.2f} seconds")
    return returncode


if __name__ == "__main__":
    raise SystemExit(main())
