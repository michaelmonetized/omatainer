#!/usr/bin/env python3
"""Run scene boundary regressions in an isolated panic=abort process.

The temporary executable contains the real CLI, IPC, and engine implementations
but starts only the regression checks: no desktop, device, or live socket opens.
Cargo dependencies use the normal cache; set CARGO_TARGET_DIR to reuse a build.
"""

import os
from pathlib import Path
import resource
import shutil
import subprocess
import tempfile


def disable_core_dumps():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


repo = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="omatainer-scene-inputs-") as temporary:
    work = Path(temporary)
    shutil.copytree(repo / "src", work / "src")
    # The actual native implementation embeds the release's offline records.
    # Retain those exact inputs when compiling the isolated production probe.
    shutil.copytree(repo / "licenses", work / "licenses")
    for name in ("Cargo.toml", "Cargo.lock"):
        shutil.copy2(repo / name, work / name)
    source = work / "src/scene_input_probe.rs"
    text = (work / "src/main.rs").read_text()
    old_module = "#[cfg(test)]\nmod scene_index_tests;"
    old_main = "fn main() -> anyhow::Result<()> {"
    assert text.count(old_module) == 1 and text.count(old_main) == 1
    text = text.replace(old_module, "mod scene_index_tests;", 1)
    text = text.replace(old_main, "fn desktop_main() -> anyhow::Result<()> {", 1)
    text += '''
#[cfg(not(panic = "abort"))]
compile_error!("scene boundary probe requires panic=abort");

fn main() {
    scene_index_tests::check_cli_scene_arguments();
    scene_index_tests::check_engine_rejects_invalid_scenes();
    scene_index_tests::check_valid_scene_operations();
    scene_index_tests::check_ipc_scene_requests();
    println!("scene CLI, IPC and engine regressions passed with panic=abort");
}
'''
    source.write_text(text)
    with (work / "Cargo.toml").open("a") as manifest:
        manifest.write('\n[[bin]]\nname = "scene-input-probe"\npath = "src/scene_input_probe.rs"\n')
    target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target")).resolve()
    subprocess.run([
        "cargo", "rustc", "--offline", "--locked",
        "--manifest-path", str(work / "Cargo.toml"),
        "--target-dir", str(target), "--bin", "scene-input-probe", "--",
        "-C", "panic=abort",
    ], check=True)
    subprocess.run([str(target / "debug/scene-input-probe")], check=True,
                   preexec_fn=disable_core_dumps)
