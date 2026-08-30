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
    def __curly_original_entry():
        sys.exit(main())
    import sys
    import subprocess
    from curly_expand import expand_or_literal, cartesian

    _raw_argv = sys.argv[:]
    _positions = []
    _fields = []
    for _i, _a in enumerate(_raw_argv):
        if _a == "--out" and _i + 1 < len(_raw_argv):
            _positions.append(_i + 1)
            _fields.append(expand_or_literal(_raw_argv[_i + 1]))
            break
        if _a.startswith("--out="):
            _positions.append(_i)
            _fields.append(["--out=" + v for v in expand_or_literal(_a.split("=", 1)[1])])
            break

    if not _fields or all(len(f) <= 1 for f in _fields):
        __curly_original_entry()
    else:
        _combos = cartesian(_fields)
        _failed = False
        for _combo in _combos:
            _new_argv = list(_raw_argv)
            for _pos, _val in zip(_positions, _combo):
                _new_argv[_pos] = _val
            _r = subprocess.run([sys.executable] + _new_argv)
            if _r.returncode != 0:
                _failed = True
        if _failed:
            sys.exit(1)
