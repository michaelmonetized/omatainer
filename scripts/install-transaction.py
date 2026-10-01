#!/usr/bin/env python3
"""Stage and validate the Omarchy integration, then publish it with rollback."""

import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "atomic_executable", Path(__file__).with_name("install-executable.py")
)
atomic = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(atomic)

LICENSE_SPEC = importlib.util.spec_from_file_location(
    "license_manifest", Path(__file__).with_name("license-manifest.py")
)
licenses = importlib.util.module_from_spec(LICENSE_SPEC)
LICENSE_SPEC.loader.exec_module(licenses)

BINARY = ".local/bin/omatainer"
PREVIOUS = BINARY + ".previous"
PLUGIN = ".config/omarchy/plugins/omatainer"
STATIC = {
    ".local/share/applications/org.omarchy.omatainer.desktop": "contrib/org.omarchy.omatainer.desktop",
    ".local/share/icons/hicolor/scalable/apps/org.omarchy.omatainer.svg": "contrib/org.omarchy.omatainer.svg",
    ".config/hypr/apps/omatainer.lua": "contrib/omatainer.lua",
    **{f"{PLUGIN}/{name}": f"plugin/{name}"
       for name in ("manifest.json", "BarWidget.qml", "Service.qml")},
}
HYPR = ".config/hypr/hyprland.lua"
BINDINGS = ".config/hypr/bindings.lua"
SHELL = ".config/omarchy/shell.json"
MENU = ".config/omarchy/extensions/omarchy-menu.jsonc"
HOOK = ".config/omarchy/hooks/theme-set.d/omatainer-reload"
TARGETS = [BINARY, PREVIOUS, *STATIC, *licenses.INSTALLED, licenses.RECEIPT, HYPR, BINDINGS, SHELL, MENU, HOOK]
EMPTY_DIRECTORIES = [".config/omatainer"]


class InstallError(Exception):
    pass


def sha(data):
    return hashlib.sha256(data).hexdigest()


def command_result(command, purpose):
    try:
        return subprocess.run(command, text=True, capture_output=True)
    except OSError as error:
        raise InstallError(f"{purpose}: {error}") from error


def command_diagnostic(command, purpose, result):
    details = [f"{purpose}: {shlex.join(map(str, command))} exited {result.returncode}"]
    # A validator can write its actionable error to either stream, including
    # stdout alongside an unrelated stderr message. Preserve both.
    for name in ("stdout", "stderr"):
        output = getattr(result, name).strip()
        if output:
            details.append(f"{name}: {output}")
    return "\n".join(details)


def run(command, purpose):
    result = command_result(command, purpose)
    if result.returncode:
        raise InstallError(command_diagnostic(command, purpose, result))
    return result.stdout.strip()


def validate_desktop(purpose):
    command = ["hyprctl", "configerrors"]
    result = command_result(command, f"validate {purpose}")
    if result.returncode or result.stdout.strip() or result.stderr.strip():
        raise InstallError(
            command_diagnostic(command, f"validate {purpose}", result)
            + "\nHyprland configuration validation failed. Correct the reported "
              "configuration or connection problem, inspect with `hyprctl configerrors`, then retry."
        )


def reload_desktop(purpose):
    run(["hyprctl", "reload"], f"reload {purpose}")
    validate_desktop(purpose)


def recovery_command(root, journal):
    return shlex.join(["bash", str(Path(__file__).resolve().with_name("install-omarchy.sh")),
                       "--user-root", str(root), "--recover", str(journal)])


def checked_relative(relative):
    path = Path(relative)
    if path.is_absolute() or not path.parts or any(part in ("..", ".") for part in path.parts):
        raise InstallError(f"unsafe journal path: {relative}")
    return path


def regular_target(root, relative):
    path = root / checked_relative(relative)
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise InstallError(f"refusing symlink installation path: {parent}")
    if path.exists():
        info = path.stat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
            raise InstallError(f"target must be a regular file owned by this user: {path}")
    return path


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def mkdirs(path, mode=0o755):
    missing = []
    parent = path
    while not parent.exists():
        missing.append(parent)
        parent = parent.parent
    for parent in reversed(missing):
        parent.mkdir(mode=mode)
        sync_directory(parent.parent)


@contextmanager
def installation_lock(state_root):
    mkdirs(state_root, mode=0o700)
    lock = state_root / "install.lock"
    if lock.is_symlink():
        raise InstallError(f"refusing symlink lock file: {lock}")
    with lock.open("a") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise InstallError(f"another installer or recovery owns {lock}") from error
        yield


def write_file(path, data, mode):
    mkdirs(path.parent)
    with path.open("wb") as stream:
        stream.write(data)
        stream.flush()
        os.fchmod(stream.fileno(), mode)
        os.fsync(stream.fileno())
    sync_directory(path.parent)


def json_object(text, path):
    try:
        value = json.loads(text)
    except ValueError as error:
        raise InstallError(f"invalid JSON in {path}: {error}") from error
    if not isinstance(value, dict):
        raise InstallError(f"{path} must contain a JSON object")
    return value


def parse_jsonc(text):
    # Replace comments with whitespace, preserving string literals and offsets.
    pattern = r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*[\s\S]*?\*/'
    clean = re.sub(pattern, lambda m: m[0] if m[0].startswith('"') else
                   ''.join('\n' if c == '\n' else ' ' for c in m[0]), text)
    # Strip only commas outside strings immediately before a closing container.
    clean = re.sub(r'"(?:\\.|[^"\\])*"|,(\s*[}\]])',
                   lambda m: m[1] if m[1] is not None else m[0], clean)
    value = json.loads(clean)
    if not isinstance(value, dict):
        raise InstallError("menu JSONC must contain an object")
    return value, clean.find("{")


class Transaction:
    def __init__(self, root, state_root, after_mutation=None):
        self.root = root.resolve(strict=True)
        self.after_mutation = after_mutation or (lambda event: None)
        self.state_parents = []
        parent = state_root
        while not parent.exists():
            self.state_parents.append(parent)
            parent = parent.parent
        mkdirs(state_root, mode=0o700)
        self.directory = Path(tempfile.mkdtemp(prefix="install-", dir=state_root))
        sync_directory(state_root)
        self.journal = {"version": 1, "root": str(self.root), "state": "staging",
                        "entries": {}, "attempted": [], "created_dirs": []}
        self.flush()

    def flush(self):
        temporary = self.directory / "journal.next"
        write_file(temporary, json.dumps(self.journal, indent=2).encode() + b"\n", 0o600)
        os.replace(temporary, self.directory / "journal.json")
        sync_directory(self.directory)

    def snapshot(self):
        for relative in TARGETS:
            path = regular_target(self.root, relative)
            entry = {"existed": path.exists()}
            if path.exists():
                data = path.read_bytes()
                entry.update(mode=stat.S_IMODE(path.stat().st_mode), old_sha=sha(data))
                write_file(self.directory / "old" / relative, data, entry["mode"])
            self.journal["entries"][relative] = entry
        self.flush()

    def original_text(self, relative):
        if not self.journal["entries"][relative]["existed"]:
            raise InstallError(f"required Omarchy configuration is missing: {self.root / relative}")
        return (self.directory / "old" / relative).read_text()

    def stage(self, relative, data, mode=0o644, preserve_mode=False):
        entry = self.journal["entries"][relative]
        if preserve_mode and entry["existed"]:
            mode = entry["mode"]
        write_file(self.directory / "new" / relative, data, mode)
        entry.update(new_sha=sha(data), new_mode=mode)

    def prepare(self):
        # Refuse a cross-filesystem rename before any destination is modified.
        device = self.directory.stat().st_dev
        for relative, entry in self.journal["entries"].items():
            if entry["existed"] and not self.matches(self.directory / "old" / relative, entry, "old"):
                raise InstallError(f"backup failed verification before publication: {relative}")
            if "new_sha" in entry and relative != PREVIOUS:
                if not self.matches(self.directory / "new" / relative, entry, "new"):
                    raise InstallError(f"staged artifact changed after validation: {relative}")
        destinations = [relative for relative, entry in self.journal["entries"].items()
                        if "new_sha" in entry]
        destinations += [f"{directory}/.directory-check" for directory in EMPTY_DIRECTORIES]
        for relative in destinations:
            path = regular_target(self.root, relative)
            parent = path.parent
            while not parent.exists():
                parent = parent.parent
            if not parent.is_dir():
                raise InstallError(f"target parent is not a directory: {parent}")
            if parent.stat().st_dev != device:
                raise InstallError(f"staging and target must share a filesystem: {path}")
            entry = self.journal["entries"].get(relative)
            if entry is not None and not self.matches(path, entry, "old"):
                raise InstallError(f"configuration changed during staging: {path}")
        self.journal["state"] = "prepared"
        self.flush()

    @staticmethod
    def matches(path, entry, version):
        if version == "old" and not entry["existed"]:
            return not path.exists()
        return (path.is_file() and not path.is_symlink()
                and sha(path.read_bytes()) == entry.get(f"{version}_sha")
                and stat.S_IMODE(path.stat().st_mode) == entry.get("mode" if version == "old" else "new_mode"))

    def parents(self, path):
        missing = []
        parent = path.parent
        while not parent.exists():
            missing.append(parent)
            parent = parent.parent
        for parent in reversed(missing):
            relative = str(parent.relative_to(self.root))
            self.journal["created_dirs"].append(relative)
            self.flush()
            parent.mkdir()
            sync_directory(parent.parent)
            self.after_mutation(f"mkdir:{relative}")

    def attempt(self, relative):
        path = regular_target(self.root, relative)
        if not self.matches(path, self.journal["entries"][relative], "old"):
            raise InstallError(f"configuration changed before publication: {path}")
        self.parents(path)
        self.journal["attempted"].append(relative)
        self.flush()
        return path

    def commit(self):
        self.journal["state"] = "committing"
        self.flush()
        destination = self.attempt(BINARY)
        if self.journal["entries"][BINARY]["existed"]:
            self.attempt(PREVIOUS)
        atomic.install_executable(self.directory / "new" / BINARY, destination)
        self.after_mutation("publish:binary-and-previous")
        for directory in EMPTY_DIRECTORIES:
            self.parents(self.root / directory / ".directory-check")
        for relative in TARGETS:
            if relative in (BINARY, PREVIOUS):
                continue
            path = self.attempt(relative)
            os.replace(self.directory / "new" / relative, path)
            sync_directory(path.parent)
            self.after_mutation(f"publish:{relative}")

    def rollback(self):
        errors = []
        for relative in reversed(self.journal["attempted"]):
            try:
                path = regular_target(self.root, relative)
                entry = self.journal["entries"][relative]
                if self.matches(path, entry, "old"):
                    continue
                if path.exists() and not self.matches(path, entry, "new"):
                    raise InstallError(f"target changed outside this transaction; preserving it: {path}")
                if entry["existed"]:
                    backup = self.directory / "old" / relative
                    if sha(backup.read_bytes()) != entry["old_sha"]:
                        raise InstallError(f"rollback backup failed verification: {backup}")
                    fd, temporary = tempfile.mkstemp(prefix=".omatainer-restore-", dir=path.parent)
                    os.close(fd)
                    try:
                        write_file(Path(temporary), backup.read_bytes(), entry["mode"])
                        os.replace(temporary, path)
                        sync_directory(path.parent)
                    finally:
                        Path(temporary).unlink(missing_ok=True)
                else:
                    path.unlink(missing_ok=True)
                    sync_directory(path.parent)
            except (OSError, InstallError) as error:
                errors.append(f"{relative}: {error}")
        for relative in reversed(self.journal["created_dirs"]):
            path = self.root / checked_relative(relative)
            try:
                if path.exists():
                    path.rmdir()
                    sync_directory(path.parent)
            except OSError as error:
                errors.append(f"remove new directory {path}: {error}")
        self.journal["state"] = "rollback_failed" if errors else "rolled_back"
        self.journal["rollback_errors"] = errors
        self.flush()
        if errors:
            raise InstallError(f"rollback incomplete: {'; '.join(errors)}; recover with "
                               f"{recovery_command(self.root, self.directory / 'journal.json')}")

    def discard(self):
        shutil.rmtree(self.directory)
        for parent in self.state_parents:
            try:
                parent.rmdir()
            except OSError:
                break


def artifacts(transaction, source):
    document = licenses.validate(source)
    if {dest:src for src,dest in document['package'].items()} != STATIC:
        raise InstallError("installer payload differs from the reviewed license inventory")
    licenses.verify_binary(source, source / "target/release/omatainer")
    for relative, source_path in licenses.INSTALLED.items():
        transaction.stage(relative, (source / source_path).read_bytes())
    receipt = licenses.release_record(source, source / "target/release/omatainer", document)
    transaction.stage(licenses.RECEIPT, licenses.encoded(receipt))
    transaction.stage(BINARY, (source / "target/release/omatainer").read_bytes(), 0o755)
    binary = transaction.journal["entries"][BINARY]
    if binary["existed"]:
        previous = transaction.journal["entries"][PREVIOUS]
        previous.update(new_sha=binary["old_sha"], new_mode=binary["mode"])
    # Validate the final prospective plugin, including user files that remain
    # untouched by publication. Never follow symlinks while making this copy.
    plugin = transaction.root / PLUGIN
    if plugin.exists():
        for path in plugin.rglob("*"):
            if path.is_symlink():
                raise InstallError(f"refusing symlink in existing plugin: {path}")
            relative = path.relative_to(transaction.root)
            staged = transaction.directory / "new" / relative
            if path.is_dir():
                mkdirs(staged)
            elif path.is_file():
                write_file(staged, path.read_bytes(), stat.S_IMODE(path.stat().st_mode))
            else:
                raise InstallError(f"existing plugin has a non-regular entry: {path}")
    for relative, source_path in STATIC.items():
        transaction.stage(relative, (source / source_path).read_bytes())

    hypr = transaction.original_text(HYPR)
    active_hypr = re.sub(r"--\[(=*)\[[\s\S]*?\]\1\]", "", hypr)
    if not re.search(r'^\s*require\s*\(\s*[\'\"]hypr\.apps\.omatainer[\'\"]\s*\)', active_hypr, re.MULTILINE):
        hypr = hypr.rstrip() + '\nrequire("hypr.apps.omatainer")\n'
    transaction.stage(HYPR, hypr.encode(), preserve_mode=True)
    bindings = transaction.original_text(BINDINGS)
    active_bindings = re.sub(r"--\[(=*)\[[\s\S]*?\]\1\]", "", bindings)
    active_bindings = "\n".join(line for line in active_bindings.splitlines()
                                if not line.lstrip().startswith("--"))
    additions = []
    if 'o.launch_sole("org.omarchy.omatainer", "omatainer")' not in active_bindings:
        additions.append('o.bind("SUPER + O", "Omatainer", o.launch_sole("org.omarchy.omatainer", "omatainer"))')
    if '"omatainer ctl togglePlay"' not in active_bindings:
        additions.append('o.bind("SUPER + SHIFT + SPACE", "Omatainer play/stop", "omatainer ctl togglePlay")')
    if additions:
        bindings = bindings.rstrip() + "\n\n-- omatainer\n" + "\n".join(additions) + "\n"
    transaction.stage(BINDINGS, bindings.encode(), preserve_mode=True)

    shell = json_object(transaction.original_text(SHELL), SHELL)
    plugins = shell.setdefault("plugins", [])
    bar = shell.setdefault("bar", {})
    if not isinstance(bar, dict) or not isinstance(bar.setdefault("layout", {}), dict):
        raise InstallError("shell.json bar and bar.layout must be objects")
    right = bar["layout"].setdefault("right", [])
    if not isinstance(plugins, list) or not isinstance(right, list):
        raise InstallError("shell.json plugins and bar.layout.right must be arrays")
    if not any(isinstance(item, dict) and item.get("id") == "omatainer" for item in plugins):
        plugins.append({"id": "omatainer"})
    if not any(isinstance(item, dict) and item.get("id") == "omatainer" for item in right):
        index = next((index + 1 for index, item in enumerate(right)
                      if isinstance(item, dict) and item.get("id") == "omarchy.audio"), len(right))
        right.insert(index, {"id": "omatainer", "chipColor": "#89b4fa"})
    transaction.stage(SHELL, (json.dumps(shell, indent=2) + "\n").encode(), preserve_mode=True)

    menu = transaction.original_text(MENU)
    try:
        parsed, opening = parse_jsonc(menu)
    except ValueError as error:
        raise InstallError(f"invalid JSONC in {MENU}: {error}") from error
    if "apps.omatainer" not in parsed:
        entry = {"icon": "󰝚", "label": "Omatainer",
                 "action": "omarchy-launch-or-focus org.omarchy.omatainer 'uwsm-app -- omatainer'",
                 "description": "DAW + dual-deck DJ"}
        menu = menu[:opening + 1] + '\n  "apps.omatainer": ' + json.dumps(entry, ensure_ascii=False) + ',\n' + menu[opening + 1:]
    parse_jsonc(menu)
    transaction.stage(MENU, menu.encode(), preserve_mode=True)
    transaction.stage(HOOK, b'#!/bin/bash\n# Theme files are also watched by omatainer.\nomatainer ctl reload-theme >/dev/null 2>&1 || true\n', 0o755)
    # Publication moves staged configuration files. Keep an independent complete
    # release payload so this installation's notices and sources remain paired
    # with its binary even after later upgrades or rollback.
    retained = transaction.directory / "release"
    staged = transaction.directory / "new"
    for relative in [*receipt["files"], licenses.RECEIPT]:
        write_file(retained / relative, (staged / relative).read_bytes(),
                   transaction.journal["entries"][relative]["new_mode"])
    licenses.verify_package(retained)



def validate(transaction):
    staged = transaction.directory / "new"
    licenses.verify_package(transaction.directory / "release")
    binary = staged / BINARY
    licenses.verify_embedded(binary, {name:(staged / licenses.LICENSE_ROOT / name).read_bytes()
                                      for name in licenses.RECORD_FILES})
    receipt = json.loads((staged / licenses.RECEIPT).read_text())
    manifest = json.loads((staged / licenses.LICENSE_ROOT / "manifest.json").read_text())
    for source_path, relative in manifest["package"].items():
        if receipt["files"][relative] != manifest["source_files"][source_path]:
            raise InstallError(f"licensed source changed during staging: {source_path}")
    for relative, digest in receipt["files"].items():
        if sha((staged / relative).read_bytes()) != digest:
            raise InstallError(f"staged licensed artifact changed: {relative}")
    if binary.read_bytes()[:4] != b"\x7fELF":
        raise InstallError(f"staged binary is not ELF: {binary}")
    json_object((staged / SHELL).read_text(), SHELL)
    json_object((staged / PLUGIN / "manifest.json").read_text(), f"{PLUGIN}/manifest.json")
    parse_jsonc((staged / MENU).read_text())
    import xml.etree.ElementTree as ET
    ET.parse(staged / ".local/share/icons/hicolor/scalable/apps/org.omarchy.omatainer.svg")
    for relative in (HYPR, BINDINGS, ".config/hypr/apps/omatainer.lua"):
        run(["luac", "-p", str(staged / relative)], f"validate Lua {relative}")
    run(["bash", "-n", str(staged / HOOK)], "validate theme hook")
    run(["desktop-file-validate", str(staged / ".local/share/applications/org.omarchy.omatainer.desktop")], "validate desktop entry")
    run(["omarchy", "plugin", "validate", str(staged / PLUGIN)], "validate staged plugin")
    validate_desktop("current desktop")


def install(source, root, state_root, after_mutation=None):
    with installation_lock(state_root):
        for journal in state_root.glob("install-*/journal.json"):
            state = json.loads(journal.read_text()).get("state")
            if state not in ("committed", "rolled_back"):
                raise InstallError(f"unfinished installation: recover with --user-root {root} --recover {journal}")
        return install_locked(source, root, state_root, after_mutation)


def install_locked(source, root, state_root, after_mutation=None):
    run(["cargo", "build", "--locked", "--release", "--manifest-path", str(source / "Cargo.toml")], "build executable")
    transaction = Transaction(root, state_root, after_mutation)
    reloaded = False
    try:
        transaction.snapshot()
        artifacts(transaction, source)
        validate(transaction)
        transaction.prepare()
        transaction.commit()
        reloaded = True
        reload_desktop("installed desktop")
        transaction.journal["state"] = "committed"
        transaction.flush()
    except BaseException as error:
        try:
            transaction.rollback()
            if reloaded:
                reload_desktop("restored desktop")
            transaction.discard()
        except (OSError, InstallError) as recovery_error:
            if transaction.journal["state"] == "rolled_back":
                transaction.journal["state"] = "reload_failed"
                transaction.journal["recovery_error"] = str(recovery_error)
                transaction.flush()
            raise InstallError(f"installation failed: {error}; recovery: {recovery_error}; "
                               f"journal: {transaction.directory / 'journal.json'}; recover with "
                               f"{recovery_command(root, transaction.directory / 'journal.json')}") from error
        raise InstallError(f"installation failed and prior files restored: {error}") from error

    warnings = []
    for command in (["update-desktop-database", str(root / ".local/share/applications")],
                    ["gtk-update-icon-cache", "-f", str(root / ".local/share/icons/hicolor")]):
        if shutil.which(command[0]):
            try:
                run(command, "refresh optional desktop cache")
            except InstallError as error:
                warnings.append(str(error))
    return transaction.directory / "journal.json", warnings


def recover(journal_path, root):
    journal_path = journal_path.resolve(strict=True)
    with installation_lock(journal_path.parent.parent):
        return recover_locked(journal_path, root)


def recover_locked(journal_path, root):
    journal = json.loads(journal_path.read_text())
    if journal.get("version") != 1 or journal.get("root") != str(root.resolve(strict=True)):
        raise InstallError("recovery journal version or user root does not match")
    transaction = Transaction.__new__(Transaction)
    transaction.root = root.resolve(strict=True)
    transaction.directory = journal_path.parent
    transaction.journal = journal
    transaction.state_parents = []
    transaction.rollback()
    try:
        reload_desktop("restored desktop")
    except InstallError as error:
        transaction.journal["state"] = "reload_failed"
        transaction.journal["recovery_error"] = str(error)
        transaction.flush()
        raise InstallError(f"prior files restored but desktop recovery is incomplete: {error}; "
                           f"recover with {recovery_command(root, journal_path)}") from error
    return journal_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--user-root", type=Path, default=Path.home())
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--state-root", type=Path)
    parser.add_argument("--recover", type=Path)
    args = parser.parse_args()
    try:
        if args.recover:
            journal = recover(args.recover, args.user_root)
            print(f"Restored prior files; recovery journal: {journal}")
        else:
            journal, warnings = install(args.source_root.resolve(), args.user_root.resolve(),
                                        args.state_root or args.user_root / ".local/state/omatainer/installations")
            print(f"Installed Omatainer. Prior files and recovery journal: {journal}")
            print("Running instances keep their executable; close and relaunch when ready.")
            for warning in warnings:
                print(f"Warning: {warning}", file=sys.stderr)
    except (OSError, ValueError, TypeError, AttributeError, InstallError) as error:
        parser.exit(1, f"Omatainer installer: {error}\n")


if __name__ == "__main__":
    main()
