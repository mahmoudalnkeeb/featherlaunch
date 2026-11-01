// SPDX-License-Identifier: GPL-3.0-only
//! How much RAM this PC has, for the default game memory and the low-memory warning.

pub struct Ram {
    pub total_mb: u64,
    pub available_mb: u64,
}

#[cfg(windows)]
pub fn read() -> Option<Ram> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: the struct is plain data with its size set as the call requires.
    let status = unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return None;
        }
        status
    };
    Some(Ram { total_mb: status.ullTotalPhys / (1024 * 1024), available_mb: status.ullAvailPhys / (1024 * 1024) })
}

#[cfg(not(windows))]
pub fn read() -> Option<Ram> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(any(not(windows), test))]
fn parse_meminfo(text: &str) -> Option<Ram> {
    let mb = |key: &str| {
        let line = text.lines().find(|l| l.starts_with(key))?;
        line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb / 1024)
    };
    Some(Ram { total_mb: mb("MemTotal:")?, available_mb: mb("MemAvailable:")? })
}

/// Half the PC's RAM, between 1 and 3 GB, in steps the memory sliders can show.
pub fn default_memory_mb(total_mb: u64) -> u32 {
    let half = (total_mb / 2).clamp(1024, 3072);
    (half - half % 256) as u32
}

/// The game rarely uses its whole allowance at once, so this leaves some margin before warning.
pub fn is_low(memory_mb: u32, available_mb: u64) -> bool {
    u64::from(memory_mb) * 7 / 10 > available_mb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_memory_suits_the_pc() {
        assert_eq!(default_memory_mb(2048), 1024);
        assert_eq!(default_memory_mb(4096), 2048);
        assert_eq!(default_memory_mb(5000), 2304);
        assert_eq!(default_memory_mb(8192), 3072);
        assert_eq!(default_memory_mb(65536), 3072);
    }

    #[test]
    fn warns_only_when_free_memory_is_well_short() {
        assert!(is_low(3072, 2000));
        assert!(!is_low(3072, 2200));
    }

    #[test]
    fn reads_linux_meminfo() {
        let ram = parse_meminfo("MemTotal:       8000000 kB\nMemFree:  10 kB\nMemAvailable:   4096000 kB\n").unwrap();
        assert_eq!((ram.total_mb, ram.available_mb), (7812, 4000));
        assert!(parse_meminfo("MemTotal: 1 kB\n").is_none());
    }
}
