#!/usr/bin/env python3
"""Create a reproducible native release archive from a staged directory."""

import gzip
import pathlib
import sys
import tarfile


def add_file(archive: tarfile.TarFile, source: pathlib.Path, name: str) -> None:
    info = tarfile.TarInfo(name)
    info.size = source.stat().st_size
    info.mode = 0o755 if source.name == "work" else 0o644
    info.mtime = 0
    with source.open("rb") as content:
        archive.addfile(info, content)


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: archive-release.py <staged-directory> <output.tar.gz>")

    source = pathlib.Path(sys.argv[1])
    output = pathlib.Path(sys.argv[2])
    with output.open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
                directory = tarfile.TarInfo(f"{source.name}/")
                directory.type = tarfile.DIRTYPE
                directory.mode = 0o755
                directory.mtime = 0
                archive.addfile(directory)
                for name in ("LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "README.md", "work"):
                    add_file(archive, source / name, f"{source.name}/{name}")


if __name__ == "__main__":
    main()
