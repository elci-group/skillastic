import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent


class BuildCliTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, str(ROOT / "buildtool.py"), *args],
            capture_output=True,
            text=True,
        )

    def test_fast_mode_writes_build_file(self):
        with tempfile.TemporaryDirectory() as d:
            proc = self.run_cli("--output-dir", d, "--mode", "fast")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertEqual((pathlib.Path(d) / "BUILD.txt").read_text(), "mode=fast\n")

    def test_full_mode_writes_build_file(self):
        with tempfile.TemporaryDirectory() as d:
            proc = self.run_cli("--output-dir", d, "--mode", "full")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertEqual((pathlib.Path(d) / "BUILD.txt").read_text(), "mode=full\n")

    def test_missing_arguments_fail(self):
        proc = self.run_cli()
        self.assertNotEqual(proc.returncode, 0)

    def test_invalid_mode_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            proc = self.run_cli("--output-dir", d, "--mode", "turbo")
            self.assertNotEqual(proc.returncode, 0)


if __name__ == "__main__":
    unittest.main()
