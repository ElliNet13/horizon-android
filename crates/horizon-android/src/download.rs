//! Several downloads at once, each checked against its SHA-1: Minecraft's files for the
//! on-device preparer. Files already there with the right size are kept, so an interrupted
//! download carries on.

use sha1::{Digest, Sha1};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// One file to fetch.
#[derive(Clone, Debug)]
pub struct Job {
    pub url: String,
    pub to: PathBuf,
    /// Lowercase hex; empty skips the check.
    pub sha1: String,
    pub size: u64,
}

/// Files fetched at the same time.
const WORKERS: usize = 8;
const ATTEMPTS: usize = 3;

enum Message {
    /// Bytes received (negative when an attempt starts over).
    Bytes(i64),
    Finished(Result<(), String>),
}

fn fetch_one(job: &Job, send: &mpsc::Sender<Message>) -> Result<(), String> {
    if job.size > 0 && std::fs::metadata(&job.to).is_ok_and(|m| m.len() == job.size) {
        let _ = send.send(Message::Bytes(job.size as i64));
        return Ok(());
    }
    if let Some(parent) = job.to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let part = PathBuf::from(format!("{}.part", job.to.display()));
    let mut last = String::new();
    for _ in 0..ATTEMPTS {
        let mut sent = 0i64;
        let attempt = (|| -> Result<(), String> {
            let response = ureq::get(&job.url)
                .call()
                .map_err(|e| format!("{}: {e}", job.url))?;
            let mut body = response.into_reader();
            let mut file =
                std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
            let mut hash = Sha1::new();
            let mut buffer = vec![0u8; 1 << 16];
            let mut pending = 0i64;
            loop {
                let n = body
                    .read(&mut buffer)
                    .map_err(|e| format!("{}: {e}", job.url))?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
                file.write_all(&buffer[..n])
                    .map_err(|e| format!("{}: {e}", part.display()))?;
                pending += n as i64;
                if pending >= 1 << 17 {
                    sent += pending;
                    let _ = send.send(Message::Bytes(pending));
                    pending = 0;
                }
            }
            sent += pending;
            let _ = send.send(Message::Bytes(pending));
            let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if !job.sha1.is_empty() && got != job.sha1 {
                return Err(format!("{}: checksum does not match", job.url));
            }
            std::fs::rename(&part, &job.to).map_err(|e| format!("{}: {e}", job.to.display()))
        })();
        match attempt {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                let _ = send.send(Message::Bytes(-sent));
                let _ = std::fs::remove_file(&part);
            }
        }
    }
    Err(last)
}

/// Fetches every job, `WORKERS` at a time. `progress` gets the bytes received and the files
/// finished so far, on the calling thread. The first failure stops the rest and is returned.
pub fn fetch_all(jobs: Vec<Job>, progress: &mut dyn FnMut(u64, usize)) -> Result<(), String> {
    let count = jobs.len();
    let queue = Mutex::new(jobs);
    let stop = AtomicBool::new(false);
    let (send, receive) = mpsc::channel::<Message>();
    let mut failure = None;
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(count) {
            let (queue, stop, send) = (&queue, &stop, send.clone());
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some(job) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                        break;
                    };
                    let result = fetch_one(&job, &send);
                    let failed = result.is_err();
                    let _ = send.send(Message::Finished(result));
                    if failed {
                        break;
                    }
                }
            });
        }
        drop(send);
        let (mut bytes, mut finished) = (0i64, 0usize);
        for message in receive {
            match message {
                Message::Bytes(n) => bytes += n,
                Message::Finished(Ok(())) => finished += 1,
                Message::Finished(Err(e)) => {
                    failure.get_or_insert(e);
                    stop.store(true, Ordering::Relaxed);
                }
            }
            progress(bytes.max(0) as u64, finished);
        }
    });
    match failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// A server that answers `/<name>` with the body `<name>` repeated, for any request.
    fn serve(connections: usize) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(connections).flatten() {
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let name = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .trim_start_matches('/');
                    let body = name.repeat(1000);
                    let mut stream = stream;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                });
            }
        });
        port
    }

    fn sha1_of(text: &str) -> String {
        Sha1::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn fetches_many_files_in_parallel_and_checks_them() {
        let port = serve(40);
        let dir =
            std::env::temp_dir().join(format!("horizon-download-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<String> = (0..12).map(|i| format!("file{i}")).collect();
        let jobs: Vec<Job> = names
            .iter()
            .map(|n| Job {
                url: format!("http://127.0.0.1:{port}/{n}"),
                to: dir.join("a/b").join(n),
                sha1: sha1_of(&n.repeat(1000)),
                size: (n.len() * 1000) as u64,
            })
            .collect();
        let total: u64 = jobs.iter().map(|j| j.size).sum();
        let mut last = (0, 0);
        fetch_all(jobs.clone(), &mut |bytes, files| last = (bytes, files)).unwrap();
        assert_eq!(last, (total, 12));
        for n in &names {
            let text = std::fs::read_to_string(dir.join("a/b").join(n)).unwrap();
            assert_eq!(text, n.repeat(1000));
        }
        // A wrong checksum fails, and leaves nothing under the real name.
        let bad = Job {
            sha1: "0".repeat(40),
            to: dir.join("bad"),
            ..jobs[0].clone()
        };
        assert!(fetch_all(vec![bad], &mut |_, _| {}).is_err());
        assert!(!dir.join("bad").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
