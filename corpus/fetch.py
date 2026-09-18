"""Download the corpus packages and unpack them for analysis.

Written in Python because pip already solves downloading, dependency-free
resolution, and unpacking both wheels and sdists. Reimplementing that in Rust
would be a week of zip and tar handling that teaches nothing and tests nothing.

The package list is pinned in packages.txt so a scan is reproducible. Packages
are downloaded into packages/, which is not committed - the list and the results
are, so anyone can reproduce the run without the repository carrying a few
hundred megabytes of third-party code.

    python corpus/fetch.py            # download and unpack
    python corpus/fetch.py --clean    # remove what was downloaded
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys
import tarfile
import zipfile

HERE = pathlib.Path(__file__).parent
PACKAGES = HERE / "packages"
WHEELS = HERE / "wheels"
LIST = HERE / "packages.txt"


def requirements() -> list[str]:
    lines = LIST.read_text(encoding="utf-8").splitlines()
    return [line.strip() for line in lines if line.strip() and not line.startswith("#")]


def download(specs: list[str]) -> list[str]:
    """Downloads each package, returning the ones that could not be fetched.

    One at a time rather than in a batch: a single unavailable package should
    cost that package, not the whole corpus.
    """
    WHEELS.mkdir(parents=True, exist_ok=True)
    failed = []

    for index, spec in enumerate(specs, start=1):
        print(f"  [{index}/{len(specs)}] {spec}", flush=True)
        # --no-deps keeps the corpus exactly the pinned list. Dependencies
        # would make the scan irreproducible as the wider ecosystem moves.
        result = subprocess.run(
            [
                sys.executable, "-m", "pip", "download",
                "--no-deps", "--quiet",
                "--dest", str(WHEELS),
                spec,
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            reason = (result.stderr or result.stdout).strip().splitlines()
            print(f"      unavailable: {reason[-1] if reason else 'unknown'}")
            failed.append(spec)

    return failed


def unpack() -> int:
    PACKAGES.mkdir(parents=True, exist_ok=True)
    count = 0

    for archive in sorted(WHEELS.iterdir()):
        target = PACKAGES / archive.name.split("-")[0]
        if target.exists():
            continue

        try:
            if archive.suffix == ".whl" or archive.suffix == ".zip":
                with zipfile.ZipFile(archive) as zf:
                    zf.extractall(target)
            elif archive.name.endswith((".tar.gz", ".tgz")):
                with tarfile.open(archive) as tf:
                    tf.extractall(target, filter="data")
            else:
                continue
        except (zipfile.BadZipFile, tarfile.TarError, OSError) as error:
            print(f"  skipped {archive.name}: {error}")
            continue

        count += 1

    return count


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clean", action="store_true", help="remove downloads")
    args = parser.parse_args()

    if args.clean:
        for path in (PACKAGES, WHEELS):
            shutil.rmtree(path, ignore_errors=True)
        print("removed packages/ and wheels/")
        return

    specs = requirements()
    failed = download(specs)
    unpacked = unpack()

    files = sum(1 for _ in PACKAGES.rglob("*.py"))
    print(f"unpacked {unpacked} archives, {files} Python files under {PACKAGES}")
    if failed:
        print(f"{len(failed)} unavailable: {', '.join(failed)}")


if __name__ == "__main__":
    main()
