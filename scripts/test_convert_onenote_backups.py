#!/usr/bin/env python3

import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from convert_onenote_backups import (
    backup_identity,
    converter_command,
    select_latest_backups,
    stage_backups,
)


class BackupSelectionTests(unittest.TestCase):
    def test_latest_filesystem_modification_time_wins(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            notebook = root / "Notebook"
            notebook.mkdir()
            newer_name_older_mtime = notebook / "Section.one (On 02-09-2026).one"
            older_name_newer_mtime = notebook / "Section.one (On 01-09-2026).one"
            newer_name_older_mtime.touch()
            older_name_newer_mtime.touch()
            os.utime(newer_name_older_mtime, ns=(100, 100))
            os.utime(older_name_newer_mtime, ns=(200, 200))

            selected = select_latest_backups(root)

            self.assertEqual([backup.path for backup in selected], [older_name_newer_mtime])

    def test_same_section_name_in_different_folders_is_not_deduplicated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "Notebook A" / "General.one"
            second = root / "Notebook B" / "General.one"
            first.parent.mkdir()
            second.parent.mkdir()
            first.touch()
            second.touch()

            selected = select_latest_backups(root)

            self.assertEqual([backup.path for backup in selected], [first, second])

    def test_backup_identity_uses_the_name_before_the_first_one_extension(self):
        root = Path("/backup")
        path = root / "Notebook" / "Research.one (On 02-09-2026).one"
        self.assertEqual(backup_identity(path, root), (Path("Notebook"), "Research"))

    def test_staged_backup_uses_the_logical_section_name(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source" / "Notebook" / "Research.one (On 02-09-2026).one"
            source.parent.mkdir(parents=True)
            source.write_bytes(b"section")
            backup = select_latest_backups(root / "source")[0]

            staged = stage_backups([backup], root / "staging")

            self.assertEqual(staged, [root / "staging" / "Notebook" / "Research.one"])
            self.assertEqual(staged[0].read_bytes(), b"section")

    def test_image_optimization_flag_is_forwarded(self):
        command = converter_command(
            Path("/bin/one2md"),
            [Path("/staging/Notebook/Research.one")],
            Path("/staging"),
            Path("/output"),
            True,
        )

        self.assertEqual(command[-1], "--no-image-optimization")


if __name__ == "__main__":
    unittest.main()
