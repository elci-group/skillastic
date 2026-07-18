"""buildtool — project build CLI.

Runs a "build" into an output directory: writes DIR/BUILD.txt recording
the build mode.
"""

import argparse
import pathlib
import sys


def main(argv=None):
    parser = argparse.ArgumentParser(prog="buildtool")
    parser.add_argument("--out", required=True, metavar="DIR",
                        help="directory the build output is written to")
    parser.add_argument("--fast", action="store_true",
                        help="fast build (default: full build)")
    args = parser.parse_args(argv)

    mode = "fast" if args.fast else "full"
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "BUILD.txt").write_text(f"mode={mode}\n")
    print(f"built mode={mode} -> {out}/BUILD.txt")
    return 0


if __name__ == "__main__":
    sys.exit(main())
