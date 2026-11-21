//! Embeds the app icon into `batune.exe`, so Explorer and shortcuts
//! show it rather than the default one. Other targets build nothing.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // embed-resource / winresource would also cover MSVC's rc.exe, but the
    // release builds are mingw only, and windres is already in the CI images.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=the exe icon is embedded only for the gnu target");
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest.join("packaging/windows/batune.ico");
    println!("cargo:rerun-if-changed={}", ico.display());

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc = out.join("icon.rc");
    // windres reads `\` in a string as an escape, so Windows paths need `/`.
    let ico = ico.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{ico}\"\n")).unwrap();

    let obj = out.join("icon.o");
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        format!("{arch}-w64-mingw32-windres")
    });
    let status = Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("{windres}: {e} (set WINDRES to its path)"));
    assert!(status.success(), "{windres} failed: {status}");
    // An object (not an archive) given to the linker is always kept, so the
    // .rsrc section survives even though no symbol refers to it.
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
