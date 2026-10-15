use std::{env, path::PathBuf, process::Command};

/// Build the pinned native loudness meter within Cargo's owned output directory.
/// Args: Cargo environment and optional explicit CC/AR executables. Returns through Cargo link directives; compiler failure stops the build without a runtime fallback.
fn main() {
    let root = PathBuf::from("vendor/ebur128");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    for file in [
        "ebur128.c",
        "ebur128.h",
        "queue/sys/queue.h",
        "COPYING",
        "notices/R128Scan.txt",
        "notices/queue.txt",
        "SOURCE.sha256",
        "UPSTREAM.json",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }
    for name in ["CC", "AR"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let object = output.join("ebur128.o");
    let archive = output.join("libomatainer_ebur128.a");
    let status = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args(["-std=gnu11", "-O2", "-fPIC", "-ffp-contract=off", "-I"])
        .arg(root.join("queue"))
        .arg("-c")
        .arg(root.join("ebur128.c"))
        .arg("-o")
        .arg(&object)
        .status()
        .expect("C compiler is required to build the pinned loudness meter");
    assert!(status.success(), "Bundled libebur128 compilation failed");
    let status = Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
        .arg("crs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("Native archiver is required for the pinned meter");
    assert!(status.success(), "Bundled libebur128 archive failed");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=omatainer_ebur128");
    println!("cargo:rustc-link-lib=m");
    println!(
        "cargo:rustc-env=EBUR128_SOURCE_SHA256={}",
        std::fs::read_to_string(root.join("SOURCE.sha256"))
            .expect("Pinned source digest")
            .trim()
    );
}
