#!/usr/bin/env python3
"""Exercise the production color parser under panic=abort, without user config."""
import json
from pathlib import Path
import subprocess
import tempfile

source = Path(__file__).resolve().parents[1] / "src/theme/color.rs"
with tempfile.TemporaryDirectory(prefix="omatainer-theme-probe-") as directory:
    work = Path(directory)
    program = work / "probe.rs"
    program.write_text(
        f"#[path = {json.dumps(str(source))}] mod color;\n"
        + r'''
#[cfg(not(panic = "abort"))]
compile_error!("theme probe requires panic=abort");
fn main() {
    assert_eq!(color::parse_rgb(" #01aBff "), Some([1, 171, 255]));
    for scalar in 128..=0x10ffff {
        let Some(ch) = char::from_u32(scalar) else { continue; };
        if ch.is_whitespace() { continue; }
        let padding = 6 - ch.len_utf8();
        for prefix in 0..=padding {
            let input = format!("{}{}{}", "a".repeat(prefix), ch, "f".repeat(padding-prefix));
            assert_eq!(color::parse_rgb(&input), None);
        }
    }
    println!("production theme color parser passed exhaustive Unicode boundary checks with panic=abort");
}
'''
    )
    binary = work / "probe"
    subprocess.run(["rustc", "--edition=2021", "-O", "-C", "panic=abort", str(program), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
