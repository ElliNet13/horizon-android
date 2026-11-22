//! How much memory the phone has free, for the warning while playing (`/proc/meminfo`).
use horizon_client::entry::MemoryInfo;

/// Free and total memory in megabytes, `None` when the system will not say.
pub fn info() -> Option<MemoryInfo> {
    parse(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

fn parse(text: &str) -> Option<MemoryInfo> {
    let kilobytes = |name: &str| -> Option<u64> {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))?
            .trim()
            .strip_suffix("kB")?
            .trim()
            .parse()
            .ok()
    };
    Some(MemoryInfo {
        available_mb: kilobytes("MemAvailable")? / 1024,
        total_mb: kilobytes("MemTotal")? / 1024,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn available_and_total_come_from_meminfo() {
        let text = "MemTotal:       11800000 kB\nMemFree:          200000 kB\nMemAvailable:    2048000 kB\n";
        let info = super::parse(text).unwrap();
        assert_eq!(info.total_mb, 11523);
        assert_eq!(info.available_mb, 2000);
    }
}
