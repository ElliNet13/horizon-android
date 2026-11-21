//! `horizon-env.txt` in the game folder: one `NAME=VALUE` per line, `#` starts a comment. The
//! entry sets each as an environment variable before the game starts (`entry.rs`); the launcher
//! page edits it, so a setting made in the game applies at the next start.
use std::{io, path::Path};

/// The file's settings, in order.
pub fn read(path: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (name, value) = line.split_once('=')?;
            (!line.starts_with('#') && !name.trim().is_empty())
                .then(|| (name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// The value of `name`, if the file has it.
pub fn get(path: &Path, name: &str) -> Option<String> {
    read(path)
        .into_iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

/// Sets `name` to `value`, or removes it (`None`), keeping every other line, comments included.
pub fn set(path: &Path, name: &str, value: Option<&str>) -> io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut written = false;
    for line in text.lines() {
        let trimmed = line.trim();
        let is_it = !trimmed.starts_with('#')
            && trimmed
                .split_once('=')
                .is_some_and(|(n, _)| n.trim() == name);
        if !is_it {
            lines.push(line.to_owned());
        } else if let (false, Some(value)) = (written, value) {
            lines.push(format!("{name}={value}"));
            written = true;
        }
    }
    if let (false, Some(value)) = (written, value) {
        lines.push(format!("{name}={value}"));
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-envfile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        dir
    }

    #[test]
    fn setting_replaces_adds_and_removes_while_comments_stay() {
        let path = temp("edit");
        std::fs::write(&path, "# my notes\nA=1\nB = 2\n").unwrap();
        set(&path, "B", Some("3")).unwrap();
        set(&path, "C", Some("4")).unwrap();
        set(&path, "A", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# my notes\nB=3\nC=4\n"
        );
        assert_eq!(get(&path, "B").as_deref(), Some("3"));
        assert_eq!(get(&path, "A"), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_created_and_a_commented_setting_is_not_a_setting() {
        let path = temp("new");
        set(&path, "X", Some("1")).unwrap();
        assert_eq!(read(&path), vec![("X".to_owned(), "1".to_owned())]);
        std::fs::write(&path, "#X=9\n").unwrap();
        assert_eq!(get(&path, "X"), None);
        let _ = std::fs::remove_file(&path);
    }
}
