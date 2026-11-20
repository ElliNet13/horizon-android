//! Minecraft's own files for the on-device preparer, fetched from Mojang's servers into the layout
//! of an official launcher's folder (`versions/<v>/<v>.json` and `.jar`, `libraries/`,
//! `assets/indexes` and `assets/objects`), which `horizon-prepare --prepare` reads. Everything
//! is checked against the SHA-1 Mojang publishes, and then against Horizon's research-lock.json by
//! the preparer. Sounds are left out: the game is silent on Android.

use crate::download::{Job, fetch_all};
use horizon_prepare::pyjson::{self, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const VERSION_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const PACKAGES: &str = "https://piston-meta.mojang.com/v1/packages";
const OBJECTS: &str = "https://resources.download.minecraft.net";

/// Where the files went.
pub struct Files {
    /// The official-launcher-style folder (`--minecraft-dir`).
    pub minecraft: PathBuf,
    /// The server jar (`--server-jar`).
    pub server_jar: PathBuf,
}

fn get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_string()
        .map_err(|e| format!("{url}: {e}"))
}

fn text<'a>(value: &'a Value, key: &str, what: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what} has no {key}"))
}

/// The assets the game plays and the preparer does not copy: sounds.
fn is_sound(name: &str) -> bool {
    name == "minecraft/sounds.json" || name.starts_with("minecraft/sounds/")
}

fn sha1_hex(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The version JSON Mojang's list names for `version`: its url, sha1 and text.
fn listed(version: &str) -> Result<(String, String, String), String> {
    let list = pyjson::loads(&get_text(VERSION_MANIFEST)?)?;
    let entry = list
        .get("versions")
        .and_then(Value::as_array)
        .and_then(|all| {
            all.iter()
                .find(|v| v.get("id").and_then(Value::as_str) == Some(version))
        })
        .ok_or_else(|| format!("Mojang's version list has no {version}"))?;
    let url = text(entry, "url", "the version list entry")?.to_owned();
    let sha1 = text(entry, "sha1", "the version list entry")?.to_owned();
    let body = get_text(&url)?;
    Ok((url, sha1, body))
}

/// Fetches `version`'s files into `work`. `progress` gets what it is doing and the fraction
/// done (0 to 1).
pub fn download(
    version: &str,
    pinned_sha1: Option<&str>,
    work: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Files, String> {
    // The version JSON Horizon's lock pins, by its hash: Mojang's list names its newest one, which
    // may be a republished file the preparer does not know. The list is the fallback.
    progress("Minecraft's version file", 0.0);
    let pinned = pinned_sha1
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|sha| (format!("{PACKAGES}/{sha}/{version}.json"), sha.to_owned()));
    let (manifest_url, manifest_sha1, manifest_text) = match pinned {
        Some((url, sha)) => match get_text(&url) {
            Ok(body) => (url, sha, body),
            Err(_) => listed(version)?,
        },
        None => listed(version)?,
    };
    if sha1_hex(manifest_text.as_bytes()) != manifest_sha1 {
        return Err(format!("{manifest_url}: checksum does not match"));
    }
    let minecraft = work.join("minecraft");
    let manifest_path = minecraft.join(format!("versions/{version}/{version}.json"));
    std::fs::create_dir_all(manifest_path.parent().ok_or("no folder")?)
        .map_err(|e| e.to_string())?;
    std::fs::write(&manifest_path, &manifest_text).map_err(|e| e.to_string())?;
    let manifest = pyjson::loads(&manifest_text)?;

    let mut jobs = Vec::new();
    let mut job = |url: &str, to: PathBuf, sha1: &str, size: u64| {
        jobs.push(Job {
            url: url.to_owned(),
            to,
            sha1: sha1.to_owned(),
            size,
        });
    };
    let downloads = manifest
        .get("downloads")
        .ok_or("the version JSON has no downloads")?;
    let file = |key: &str, to: PathBuf, job: &mut dyn FnMut(&str, PathBuf, &str, u64)| {
        let d = downloads
            .get(key)
            .ok_or_else(|| format!("the version has no {key} download"))?;
        let size = d.get("size").and_then(Value::as_i64).unwrap_or(0) as u64;
        job(text(d, "url", key)?, to, text(d, "sha1", key)?, size);
        Ok::<(), String>(())
    };
    file(
        "client",
        minecraft.join(format!("versions/{version}/{version}.jar")),
        &mut job,
    )?;
    let server_jar = work.join("server.jar");
    file("server", server_jar.clone(), &mut job)?;
    for (path, sha1, url) in horizon_prepare::artifacts::client_library_downloads(&manifest)? {
        if path.contains("..") || path.starts_with('/') {
            return Err(format!("unsafe library path {path}"));
        }
        job(&url, minecraft.join("libraries").join(&path), &sha1, 0);
    }

    // The asset index, then the objects it lists.
    let index = manifest
        .get("assetIndex")
        .ok_or("the version JSON has no assetIndex")?;
    let id = text(index, "id", "the asset index")?;
    if id.contains(['/', '\\']) || id.contains("..") {
        return Err(format!("unsafe asset index id {id}"));
    }
    let index_text = get_text(text(index, "url", "the asset index")?)?;
    if sha1_hex(index_text.as_bytes()) != text(index, "sha1", "the asset index")? {
        return Err("the asset index does not match its checksum".into());
    }
    let index_path = minecraft.join(format!("assets/indexes/{id}.json"));
    std::fs::create_dir_all(index_path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    std::fs::write(&index_path, &index_text).map_err(|e| e.to_string())?;
    let objects = pyjson::loads(&index_text)?;
    let objects = objects
        .get("objects")
        .and_then(Value::as_object)
        .ok_or("the asset index has no objects")?;
    let mut seen = HashSet::new();
    for (name, info) in objects {
        let hash = text(info, "hash", name)?;
        if is_sound(name) || hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if !seen.insert(hash.to_owned()) {
            continue;
        }
        let size = info.get("size").and_then(Value::as_i64).unwrap_or(0) as u64;
        job(
            &format!("{OBJECTS}/{}/{hash}", &hash[..2]),
            minecraft.join(format!("assets/objects/{}/{hash}", &hash[..2])),
            hash,
            size,
        );
    }

    let count = jobs.len();
    let known: u64 = jobs.iter().map(|j| j.size).sum::<u64>().max(1);
    progress(&format!("0/{count} files"), 0.02);
    fetch_all(jobs, &mut |bytes, done| {
        // Libraries have no size in the list, so the bar follows the files and the bytes together.
        let by_bytes = (bytes as f32 / known as f32).min(1.0);
        let by_files = done as f32 / count.max(1) as f32;
        progress(
            &format!("{done}/{count} files"),
            0.02 + 0.98 * by_bytes.max(by_files),
        );
    })?;
    Ok(Files {
        minecraft,
        server_jar,
    })
}
