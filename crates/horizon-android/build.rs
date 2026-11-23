//! Packs what the on-device preparer needs from this checkout into the library: research-lock.json
//! and the Java sources of Horizon's own exporters. Both are Horizon's files, not Mojang's; the
//! game's jars come from Mojang's servers when the player asks to prepare.

use std::fs;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out =
        Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("exporter-sources.tar");
    let mut archive = tar::Builder::new(fs::File::create(out).expect("create the archive"));
    let lock = root.join("research-lock.json");
    let sources = root.join("mods/horizon/src/test/java");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rerun-if-changed={}", sources.display());
    if lock.is_file() && sources.is_dir() {
        archive
            .append_path_with_name(&lock, "research-lock.json")
            .expect("add research-lock.json");
        archive
            .append_dir_all("mods/horizon/src/test/java", &sources)
            .expect("add the exporter sources");
    } else {
        println!(
            "cargo:warning=research-lock.json or the exporter sources are missing: the app cannot prepare on the device"
        );
    }
    archive.finish().expect("finish the archive");
    // The game's wasm standard library, built on a PC (`cargo run -p cargo-horizon-mod --
    // wasm-std crates/horizon-android/assets/horizon-wasm-std.wasm`): packed into the app, which
    // puts it in the game folder where the wasm mods look for it.
    let std_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/horizon-wasm-std.wasm");
    let std_out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("horizon-wasm-std.wasm");
    println!("cargo:rerun-if-changed={}", std_source.display());
    if std_source.is_file() {
        fs::copy(&std_source, &std_out).expect("copy the wasm std");
    } else {
        fs::write(&std_out, []).expect("write an empty wasm std");
        println!(
            "cargo:warning=assets/horizon-wasm-std.wasm is missing: wasm mods will not load until it is built and put in the game folder"
        );
    }
}
