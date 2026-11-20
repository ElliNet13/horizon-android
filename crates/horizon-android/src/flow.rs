//! Preparing the game files on the device, end to end: a Java runtime from Termux's repository,
//! Minecraft's files from Mojang, Horizon's own preparer run on them, the result copied into the
//! game folder, and everything that was only needed for the work deleted.
//!
//! The preparer (`horizon-prepare`) is the desktop one, linked in: it starts Java through the
//! system loader because `HORIZON_JAVA_LOADER` and `HORIZON_EXEC_PREFIX` are set for the
//! duration (see its `java.rs`).

use crate::{mojang, provision};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What `bundle` unpacks: research-lock.json and the exporters' Java sources, packed by build.rs.
const SOURCES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/exporter-sources.tar"));

pub type Progress<'a> = &'a dyn Fn(&str, Option<f32>);

/// Where the stages sit on the bar.
const JAVA: (f32, f32) = (0.0, 0.15);
const FILES: (f32, f32) = (0.15, 0.40);
const PREPARE: (f32, f32) = (0.40, 0.96);

fn within((from, to): (f32, f32), fraction: f32) -> Option<f32> {
    Some(from + (to - from) * fraction.clamp(0.0, 1.0))
}

fn unpack_sources(dest: &Path) -> Result<(), String> {
    if SOURCES.len() < 1024 {
        return Err("this build has no exporter sources (research-lock.json was missing)".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    tar::Archive::new(SOURCES)
        .unpack(dest)
        .map_err(|e| format!("{}: {e}", dest.display()))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

/// Starts Java with `-version`, with the preload and without it; the error says which broke.
pub fn check_java(prefix: &Path, home: &Path, preload: Option<&Path>) -> Result<String, String> {
    let attempt = |preload: Option<&Path>| -> Result<String, String> {
        let output = provision::java_command(prefix, home, preload)
            .arg("-version")
            .output()
            .map_err(|e| format!("cannot start {}: {e}", provision::LOADER))?;
        let text = String::from_utf8_lossy(&[output.stdout, output.stderr].concat()).into_owned();
        let lines: Vec<_> = text.lines().take(4).collect();
        if output.status.success() {
            Ok(lines.first().copied().unwrap_or_default().to_owned())
        } else {
            Err(format!("{}: {}", output.status, lines.join(" | ")))
        }
    };
    match attempt(preload) {
        Ok(first) => Ok(first),
        Err(with) => match attempt(None) {
            Ok(first) => Err(format!(
                "Java only starts without the preload ({first}); with it: {with}"
            )),
            Err(without) => Err(format!(
                "Java did not start. With preload: {with}. Without: {without}"
            )),
        },
    }
}

/// Prepares the game files into `resources` (the folder the game looks in). `private` is the
/// app's own storage, where Java is unpacked and the work is done.
pub fn run(
    resources: &Path,
    private: &Path,
    preload: Option<PathBuf>,
    progress: Progress,
) -> Result<String, String> {
    let java_prefix = private.join("java-runtime");
    let work = private.join("prepare");

    // Java.
    let finished = !java_prefix.join(".installed").exists();
    let home = match provision::java_home(&java_prefix).filter(|_| finished) {
        Some(home) => home,
        None => provision::install_java(&java_prefix, &|text, f| {
            progress(text, f.and_then(|f| within(JAVA, f)))
        })?,
    };
    progress("starting Java", within(JAVA, 1.0));
    let version = check_java(&java_prefix, &home, preload.as_deref())?;
    log::info!("java: {version}");

    // Horizon's side of the work: the lock and the exporter sources.
    let root = work.join("root");
    if !root.join("research-lock.json").is_file() {
        unpack_sources(&root)?;
    }
    let lock = horizon_prepare::artifacts::Lock::read(&root)?;

    // Minecraft's files.
    let files = mojang::download(&lock.version, &work.join("mojang"), &|text, f| {
        progress(&format!("downloading Minecraft {text}"), within(FILES, f))
    })?;

    // The preparer, with Java started through the loader.
    let output = work.join("out");
    if output.exists() {
        std::fs::remove_dir_all(&output).map_err(|e| e.to_string())?;
    }
    for (key, value) in provision::java_env(&java_prefix, &home, preload.as_deref()) {
        // SAFETY: this thread starts the preparer; nothing else in the app reads or writes the
        // environment while it runs, and the values are removed again below.
        unsafe { std::env::set_var(key, value) };
    }
    let options = horizon_prepare::Options {
        root: root.clone(),
        import: true,
        minecraft_dir: Some(files.minecraft.clone()),
        server_jar: Some(files.server_jar.clone()),
        java_home: Some(home.clone()),
        inputs: Default::default(),
        output: Some(output.clone()),
    };
    let last = Mutex::new(String::from("starting"));
    let log = |line: &str| {
        log::info!("prepare: {line}");
        if let Ok(mut last) = last.lock() {
            *last = line.to_owned();
        }
    };
    let step = |fraction: f32| {
        let text = last.lock().map(|l| l.clone()).unwrap_or_default();
        progress(&text, within(PREPARE, fraction));
    };
    let prepared = horizon_prepare::prepare_with_progress(&options, &log, &step);
    for (key, _) in provision::java_env(&java_prefix, &home, None) {
        // SAFETY: as above.
        unsafe { std::env::remove_var(key) };
    }
    unsafe { std::env::remove_var("LD_PRELOAD") };
    let prepared = prepared?;

    // Into the game folder, in one move so a half copy is never taken for the real thing.
    progress("copying the prepared files", Some(0.97));
    let name = resources
        .file_name()
        .ok_or("no folder name")?
        .to_string_lossy()
        .into_owned();
    let partial = resources.with_file_name(format!("{name}.partial"));
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
    }
    if let Some(parent) = partial.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    copy_tree(&prepared, &partial)?;
    if resources.exists() {
        std::fs::remove_dir_all(resources).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&partial, resources).map_err(|e| e.to_string())?;

    // Java and the downloads were only needed for this.
    progress("removing Java and the downloads", Some(0.99));
    let _ = std::fs::remove_dir_all(&java_prefix);
    let _ = std::fs::remove_dir_all(&work);
    Ok("Done. Close the game and open it again.".to_owned())
}
