#!/usr/bin/env python3

import os
import sys
import tempfile
import unittest
from argparse import Namespace
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from convert_onenote_backups import (
    backup_identity,
    clean_output_directory,
    converter_command,
    main,
    parse_args,
    select_latest_backups,
    stage_backups,
)


class BackupSelectionTests(unittest.TestCase):
    def test_missing_backup_directory_uses_default_onenote_location(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            with patch.object(Path, "home", return_value=home):
                with patch.object(sys, "argv", ["convert_onenote_backups.py", "-o", "out"]):
                    args = parse_args()

        self.assertEqual(
            args.backup_directory,
            home
            / "Library/Containers/com.microsoft.onenote.mac/Data/Library/Application Support"
            / "Microsoft User Data/OneNote/15.0/Backup",
        )
        self.assertEqual(args.color_mode, "dark")

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
            "dark",
            True,
        )

        self.assertEqual(command[-1], "--no-image-optimization")

    def test_color_mode_is_forwarded(self):
        command = converter_command(
            Path("/bin/one2md"),
            [Path("/staging/Notebook/Research.one")],
            Path("/staging"),
            Path("/output"),
            "light",
            False,
        )

        self.assertEqual(command[-2:], ["--color-mode", "light"])

    def test_existing_output_is_skipped_without_force(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            output.mkdir()
            existing = output / "existing.md"
            existing.write_text("keep me")
            args = Namespace(
                backup_directory=root,
                output=output,
                one2md=None,
                force=False,
                no_image_optimization=False,
                color_mode="dark",
            )

            stdout = StringIO()
            with patch("convert_onenote_backups.parse_args", return_value=args):
                with redirect_stdout(stdout):
                    returncode = main()

            self.assertEqual(returncode, 0)
            self.assertEqual(existing.read_text(), "keep me")
            self.assertIn("Skipping conversion", stdout.getvalue())

    def test_clean_output_preserves_only_obsidian_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            obsidian = output / ".obsidian"
            obsidian.mkdir()
            config = obsidian / "app.json"
            config.write_text("settings")
            (output / "page.md").write_text("page")
            (output / ".hidden").write_text("hidden")
            nested = output / "Notebook"
            nested.mkdir()
            (nested / "Section.md").write_text("section")

            clean_output_directory(output)

            self.assertEqual(list(output.iterdir()), [obsidian])
            self.assertEqual(config.read_text(), "settings")


if __name__ == "__main__":
    unittest.main()
