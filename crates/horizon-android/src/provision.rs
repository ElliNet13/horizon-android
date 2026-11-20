//! A Java runtime on the device, fetched at run time from Termux's package repository (so the APK
//! carries none) and removed again when the game files are prepared.
//!
//! Termux builds OpenJDK against Android's own C library, which no other source offers for Java 25.
//! The packages are `.deb` archives built for Termux's install prefix; this unpacks them (and what
//! they depend on, resolved from the repository's index) into one folder that stands in for that
//! prefix. Nothing here is pinned to a version: the index names the current package and its
//! SHA-256, and a download is used only if it matches.

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const REPO: &str = "https://packages.termux.dev/apt/termux-main";
const INDEX: &str = "dists/stable/main/binary-aarch64/Packages";
/// Where Termux's packages are rooted inside their archives.
const PREFIX: &str = "data/data/com.termux/files/usr";
/// What a package's absolute links and paths say, to be rewritten to the folder it is unpacked to.
const PREFIX_ABSOLUTE: &str = "/data/data/com.termux/files/usr";
const JAVA_PACKAGE: &str = "openjdk-25";

/// One package of the repository's index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

/// The packages of an apt `Packages` index, by name.
pub fn parse_index(text: &str) -> HashMap<String, Package> {
    let mut packages = HashMap::new();
    for stanza in text.split("\n\n") {
        let mut package = Package::default();
        for line in stanza.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key {
                "Package" => package.name = value.to_owned(),
                "Depends" | "Pre-Depends" => package.depends.extend(
                    value
                        .split(',')
                        .map(|d| d.trim().to_owned())
                        .filter(|d| !d.is_empty()),
                ),
                "Provides" => package
                    .provides
                    .extend(value.split(',').map(|p| bare_name(p).to_owned())),
                "Filename" => package.filename = value.to_owned(),
                "Size" => package.size = value.parse().unwrap_or(0),
                "SHA256" => package.sha256 = value.to_ascii_lowercase(),
                _ => {}
            }
        }
        if !package.name.is_empty() && !package.filename.is_empty() {
            packages.insert(package.name.clone(), package);
        }
    }
    packages
}

/// `libfoo (>= 1.2)` and `a | b`'s first choice, as a bare package name.
fn bare_name(dependency: &str) -> &str {
    let first = dependency.split('|').next().unwrap_or(dependency);
    first.split('(').next().unwrap_or(first).trim()
}

/// `root` and everything it depends on, dependencies first. A name no package carries is skipped
/// (Android's own libraries, such as libc, are not in the repository).
pub fn resolve<'a>(
    index: &'a HashMap<String, Package>,
    root: &str,
) -> Result<Vec<&'a Package>, String> {
    let provided: HashMap<&str, &Package> = index
        .values()
        .flat_map(|p| p.provides.iter().map(move |name| (name.as_str(), p)))
        .collect();
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let start = index
        .get(root)
        .ok_or_else(|| format!("the repository has no {root}"))?;
    let mut stack = vec![(start, false)];
    while let Some((package, expanded)) = stack.pop() {
        if expanded {
            order.push(package);
            continue;
        }
        if !seen.insert(package.name.as_str()) {
            continue;
        }
        stack.push((package, true));
        for dependency in &package.depends {
            let name = bare_name(dependency);
            if let Some(next) = index.get(name).or_else(|| provided.get(name).copied()) {
                stack.push((next, false));
            }
        }
    }
    Ok(order)
}

/// Streams `url` to `to`, returning its SHA-256.
fn download(url: &str, to: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<String, String> {
    let response = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let mut body = response.into_reader();
    let mut file = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let n = body.read(&mut buffer).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        on_bytes(n as u64);
        file.write_all(&buffer[..n])
            .map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpacks a Termux `.deb` into `dest`, which takes the place of the package's install prefix.
pub fn extract_deb(deb: &Path, dest: &Path) -> Result<(), String> {
    extract_deb_reporting(deb, dest, &mut |_| {})
}

/// A reader that tells how many bytes pass through it.
struct Counting<R> {
    inner: R,
    count: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count
            .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(n)
    }
}

/// [`extract_deb`], reporting (as a running total) the bytes of the `.deb` consumed so far.
pub fn extract_deb_reporting(
    deb: &Path,
    dest: &Path,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let mut file = fs::File::open(deb).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"!<arch>\n" {
        return Err(format!("{}: not a deb", deb.display()));
    }
    loop {
        let mut header = [0u8; 60];
        if file.read_exact(&mut header).is_err() {
            return Err(format!("{}: no data archive", deb.display()));
        }
        // GNU `ar` ends a member name with a slash: `data.tar.xz/`.
        let name = String::from_utf8_lossy(&header[..16])
            .trim()
            .trim_end_matches('/')
            .to_owned();
        let size: u64 = String::from_utf8_lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| "bad archive header".to_owned())?;
        if !name.starts_with("data.tar") {
            file.seek(SeekFrom::Current((size + size % 2) as i64))
                .map_err(|e| e.to_string())?;
            continue;
        }
        if !name.ends_with(".xz") {
            return Err(format!("{}: unsupported archive {name}", deb.display()));
        }
        let decoder = xz4rust::XzDecoder::in_heap_with_alloc_dict_size(1 << 20, 1 << 27);
        let buffer = std::num::NonZeroUsize::new(1 << 16).expect("non-zero");
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counted = Counting {
            inner: file.take(size),
            count: count.clone(),
        };
        let reader = xz4rust::XzReader::new_with_buffer_size_and_decoder(counted, buffer, decoder);
        return untar(reader, dest, &mut || {
            on_bytes(count.load(std::sync::atomic::Ordering::Relaxed))
        });
    }
}

/// The path a data entry belongs at inside `dest`, or `None` for entries outside the prefix.
fn inside(entry: &Path) -> Option<PathBuf> {
    let mut parts = entry.components().filter_map(|c| match c {
        std::path::Component::Normal(p) => Some(p),
        _ => None,
    });
    let wanted: Vec<_> = PREFIX.split('/').collect();
    for w in &wanted {
        if parts.next()? != *w {
            return None;
        }
    }
    let rest: PathBuf = parts.collect();
    Some(rest)
}

fn untar(reader: impl Read, dest: &Path, tick: &mut dyn FnMut()) -> Result<(), String> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let absolute = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        tick();
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let Some(rel) = inside(&path) else {
            continue;
        };
        // Documentation is not needed to run Java: never unpack it.
        if ["share/doc", "share/man", "share/info"]
            .iter()
            .any(|d| rel.starts_with(d))
        {
            continue;
        }
        let target = dest.join(&rel);
        let kind = entry.header().entry_type();
        // Documentation and man pages are not needed to run Java, and some of them sit behind
        // links that point outside the folder (permission denied): skip what cannot be written.
        let optional = rel.starts_with("share");
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                if optional {
                    continue;
                }
                return Err(format!("{}: {e}", parent.display()));
            }
        }
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("symlink without a target")?
                .into_owned();
            let link = match link.to_str().and_then(|l| l.strip_prefix(PREFIX_ABSOLUTE)) {
                Some(rest) => absolute.join(rest.trim_start_matches('/')),
                None => link,
            };
            let _ = fs::remove_file(&target);
            symlink(&link, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        } else if kind.is_hard_link() {
            let from = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("hard link without a target")?
                .into_owned();
            if let Some(from) = inside(&from) {
                let _ = fs::remove_file(&target);
                fs::copy(dest.join(from), &target)
                    .map_err(|e| format!("{}: {e}", target.display()))?;
            }
        } else if kind.is_file() {
            let mode = entry.header().mode().unwrap_or(0o644);
            let _ = fs::remove_file(&target);
            let mut out = match fs::File::create(&target) {
                Ok(out) => out,
                Err(_) if optional => continue,
                Err(e) => return Err(format!("{}: {e}", target.display())),
            };
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("{}: {e}", target.display()))?;
            fs::set_permissions(&target, fs::Permissions::from_mode(mode | 0o600))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The Java runtime's folder inside an unpacked prefix (`lib/jvm/java-25-openjdk`).
pub fn java_home(prefix: &Path) -> Option<PathBuf> {
    let jvm = prefix.join("lib/jvm");
    fs::read_dir(&jvm)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("bin/java").exists())
}

/// Files fetched at the same time.
const DOWNLOADS: usize = 6;

/// Downloads and unpacks Java 25 and its dependencies into `prefix`, reporting each step.
pub fn install_java(
    prefix: &Path,
    progress: &dyn Fn(&str, Option<f32>),
) -> Result<PathBuf, String> {
    let work = prefix.join(".downloads");
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    progress("the package index", None);
    let index_path = work.join("Packages");
    download(&format!("{REPO}/{INDEX}"), &index_path, &mut |_| {})?;
    let text = fs::read_to_string(&index_path).map_err(|e| e.to_string())?;
    let index = parse_index(&text);
    let packages = resolve(&index, JAVA_PACKAGE)?;
    // The bar follows the bytes: a download is most of the time, unpacking the rest of it.
    let all_bytes: u64 = packages.iter().map(|p| p.size.max(1)).sum();
    // A package unpacked on an earlier try (the app was closed or killed) is not fetched again.
    let installed = prefix.join(".installed");
    fs::create_dir_all(&installed).map_err(|e| e.to_string())?;
    let mut done_bytes = 0u64;
    let mut pending = Vec::new();
    for (i, package) in packages.iter().enumerate() {
        if installed.join(&package.name).exists() {
            done_bytes += package.size.max(1);
        } else {
            pending.push(i);
        }
    }
    // Several downloads at once (like `nala`): a few connections fill the line a single one
    // cannot. Bytes and finished files come back to this thread, which reports and unpacks.
    enum Message {
        Bytes(u64),
        Finished(usize, Result<String, String>),
    }
    let queue = std::sync::Mutex::new(pending.clone());
    let (send, receive) = std::sync::mpsc::channel::<Message>();
    let workers = DOWNLOADS.min(pending.len());
    let mut failure = None;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (queue, send, packages, work) = (&queue, send.clone(), &packages, &work);
            scope.spawn(move || {
                loop {
                    let Some(i) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                        break;
                    };
                    let package = &packages[i];
                    let deb = work.join(format!("{}.deb", package.name));
                    let mut pending_bytes = 0u64;
                    let result = download(&format!("{REPO}/{}", package.filename), &deb, &mut |n| {
                        pending_bytes += n;
                        if pending_bytes >= 1 << 17 {
                            let _ = send.send(Message::Bytes(pending_bytes));
                            pending_bytes = 0;
                        }
                    });
                    if pending_bytes > 0 {
                        let _ = send.send(Message::Bytes(pending_bytes));
                    }
                    let failed = result.is_err();
                    let _ = send.send(Message::Finished(i, result));
                    if failed {
                        break;
                    }
                }
            });
        }
        drop(send);
        let mut got = 0u64;
        let mut unpacked = 0u64;
        let mut finished = 0usize;
        let mut abort = false;
        for message in receive {
            match message {
                Message::Bytes(n) => {
                    got += n;
                    let fraction = (2 * done_bytes + got + unpacked) as f32 / (2 * all_bytes) as f32;
                    let label = format!("{}/{} downloading", finished + 1, pending.len());
                    progress(&label, Some(fraction.min(1.0) * 0.95));
                }
                Message::Finished(i, result) => {
                    if abort {
                        continue;
                    }
                    let package = &packages[i];
                    let deb = work.join(format!("{}.deb", package.name));
                    let step = result.and_then(|hash| {
                        if hash != package.sha256 {
                            return Err(format!(
                                "{}: checksum does not match the index",
                                package.name
                            ));
                        }
                        let name = format!("unpacking {}", package.name);
                        let mut read = 0u64;
                        let mut last = 0u64;
                        extract_deb_reporting(&deb, prefix, &mut |n| {
                            read = n;
                            if read - last >= 1 << 16 {
                                last = read;
                                let total = (2 * done_bytes + got + unpacked + read) as f32
                                    / (2 * all_bytes) as f32;
                                progress(&name, Some(total.min(1.0) * 0.95));
                            }
                        })?;
                        unpacked += package.size.max(1);
                        let _ = fs::remove_file(&deb);
                        fs::write(installed.join(&package.name), &package.sha256)
                            .map_err(|e| e.to_string())
                    });
                    finished += 1;
                    if let Err(e) = step {
                        failure = Some(e);
                        abort = true;
                        // Nothing more is taken from the queue.
                        if let Ok(mut q) = queue.lock() {
                            q.clear();
                        }
                    }
                }
            }
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&installed);
    java_home(prefix).ok_or_else(|| "the Java package has no bin/java".to_owned())
}

/// Android's program loader, which runs the downloaded Java.
pub const LOADER: &str = "/system/bin/linker64";

/// What a command needs to run Java from an unpacked prefix: Android's loader runs the program (an
/// app may not execute files from its own folder directly), the libraries are found in the
/// prefix, and the launcher is told where it lives (its own lookup, `/proc/self/exe`, would name
/// the loader: `crate::preload` answers that read).
pub fn java_command(prefix: &Path, home: &Path, preload: Option<&Path>) -> std::process::Command {
    let mut command = std::process::Command::new(LOADER);
    command.arg(home.join("bin/java"));
    let libs = format!(
        "{}:{}:{}",
        prefix.join("lib").display(),
        home.join("lib").display(),
        home.join("lib/server").display()
    );
    command
        .env("LD_LIBRARY_PATH", libs)
        .env("JAVA_HOME", home)
        .env("TMPDIR", prefix.join("tmp"))
        .env("HORIZON_PROC_SELF_EXE", home.join("bin/java"));
    if let Some(preload) = preload {
        command.env("LD_PRELOAD", preload);
    }
    let _ = fs::create_dir_all(prefix.join("tmp"));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX_TEXT: &str = "Package: openjdk-25\nVersion: 25-1\nDepends: libandroid-shmem, zlib (>= 1.2) | libz, libjpeg\nFilename: pool/main/o/openjdk-25/openjdk-25_25-1_aarch64.deb\nSize: 10\nSHA256: ABCD\n\nPackage: libandroid-shmem\nDepends: libc++\nFilename: pool/a.deb\nSize: 1\nSHA256: 01\n\nPackage: libc++\nFilename: pool/b.deb\nSize: 1\nSHA256: 02\n\nPackage: zlib\nProvides: libz\nFilename: pool/z.deb\nSize: 1\nSHA256: 03\n";

    #[test]
    fn resolves_dependencies_first_skipping_unknown_names() {
        let index = parse_index(INDEX_TEXT);
        let names: Vec<_> = resolve(&index, "openjdk-25")
            .unwrap()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        // libjpeg is not in this index and is skipped; zlib is taken by its first alternative.
        assert_eq!(names.last(), Some(&"openjdk-25"));
        let at = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(at("libc++") < at("libandroid-shmem"));
        assert!(names.contains(&"zlib"));
        assert_eq!(index["openjdk-25"].sha256, "abcd");
    }

    /// A real Termux-style `.deb` made with GNU ar naming (`data.tar.xz/`): a file under the prefix,
    /// a link to an absolute path in it, and a file outside it.
    #[test]
    fn unpacks_a_deb_into_the_prefix() {
        let dir = std::env::temp_dir().join(format!("horizon-deb-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let deb = dir.join("x.deb");
        fs::write(&deb, include_bytes!("fixture.deb")).unwrap();
        let dest = dir.join("usr");
        extract_deb(&deb, &dest).unwrap();
        assert_eq!(fs::read(dest.join("lib/libx.so")).unwrap(), b"hello");
        assert_eq!(fs::read(dest.join("lib/liby.so")).unwrap(), b"hello");
        assert!(!dest.join("etc").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_outside_the_prefix_are_skipped() {
        assert_eq!(
            inside(Path::new("./data/data/com.termux/files/usr/lib/a.so")),
            Some(PathBuf::from("lib/a.so"))
        );
        assert_eq!(inside(Path::new("./etc/passwd")), None);
    }
}
