#!/usr/bin/env python3
"""Publish a complete executable without writing into an executing inode."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import stat
import tempfile


def digest(path: Path) -> bytes:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").digest()


def temporary_link(source: Path, prefix: str) -> Path:
    fd, name = tempfile.mkstemp(prefix=prefix, dir=source.parent)
    os.close(fd)
    linked = Path(name)
    linked.unlink()
    os.link(source, linked)
    return linked


def install_executable(source: Path, destination: Path) -> Path | None:
    source = source.resolve(strict=True)
    source_mode = source.stat().st_mode
    if not stat.S_ISREG(source_mode) or not source_mode & 0o111:
        raise ValueError(f"source is not a regular executable: {source}")
    with source.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            raise ValueError(f"source is not an ELF executable: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_symlink():
        raise ValueError(f"refusing to replace a symlink: {destination}")
    previous = destination.with_name(destination.name + ".previous")
    exists = destination.exists()
    if exists and (not destination.is_file() or destination.stat().st_uid != os.geteuid()):
        raise ValueError(f"destination must be a regular file owned by this user: {destination}")
    if previous.is_symlink() or (previous.exists() and not previous.is_file()):
        raise ValueError(f"rollback path is not a regular file: {previous}")

    fd, name = tempfile.mkstemp(prefix=f".{destination.name}.stage-", dir=destination.parent)
    staged = Path(name)
    backup_stage = None
    original_current = None
    original_previous = None
    published = False
    backup_published = False
    preserve_recovery = False
    try:
        with os.fdopen(fd, "wb") as output, source.open("rb") as input_stream:
            shutil.copyfileobj(input_stream, output)
            output.flush()
            os.fchmod(output.fileno(), 0o755)
            os.fsync(output.fileno())
        if staged.stat().st_size != source.stat().st_size or digest(staged) != digest(source):
            raise ValueError("staged executable does not match build output")

        if exists:
            # Keep both pre-install states until the entire publication has
            # succeeded. In particular, a failed rollback whose source is
            # .previous must not destroy its only remaining executable.
            original_current = temporary_link(destination, f".{destination.name}.current-")
            if previous.exists():
                original_previous = temporary_link(previous, f".{destination.name}.previous-")
            backup_stage = temporary_link(original_current, f".{destination.name}.backup-")
            os.replace(backup_stage, previous)
            backup_published = True
            # rename is a no-op when both paths already link the same inode.
            backup_stage.unlink(missing_ok=True)
            backup_stage = None

        directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
            os.replace(staged, destination)
            published = True
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
        return previous if exists else None
    except BaseException as error:
        rollback_errors = []
        if published:
            try:
                if exists:
                    os.replace(original_current, destination)
                else:
                    destination.unlink()
            except OSError as rollback_error:
                rollback_errors.append(rollback_error)
        if backup_published:
            try:
                if original_previous is not None:
                    os.replace(original_previous, previous)
                else:
                    previous.unlink()
            except OSError as rollback_error:
                rollback_errors.append(rollback_error)
        if rollback_errors:
            # A filesystem that also rejects recovery cannot be made atomic by
            # further cleanup. Keep original inode links for manual recovery.
            preserve_recovery = True
            recovery = [str(path) for path in (original_current, original_previous)
                        if path is not None and path.exists()]
            raise OSError(
                f"installation failed ({error}); rollback failed ({rollback_errors}); "
                f"original executables retained at {recovery}"
            ) from error
        raise
    finally:
        staged.unlink(missing_ok=True)
        if backup_stage is not None:
            backup_stage.unlink(missing_ok=True)
        if not preserve_recovery:
            for path in (original_current, original_previous):
                if path is not None:
                    path.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    try:
        backup = install_executable(args.source, args.destination)
    except (OSError, ValueError) as error:
        parser.exit(1, f"executable installation failed: {error}\n")
    print(f"Installed {args.destination} atomically.")
    if backup is not None:
        print(f"Previous executable: {backup}")
    print("Running instances keep their current executable; close and relaunch to use this version.")


if __name__ == "__main__":
    main()
