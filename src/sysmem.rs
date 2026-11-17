//! Memory, CPU, disk and OS facts about the PC, for the header and the health pane.

/// (used, total) physical memory in bytes; `None` where it cannot be read.
#[cfg(windows)]
pub fn ram() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; zeroed with dwLength set is the documented input.
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `m` is a valid, initialised MEMORYSTATUSEX that outlives the call.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    Some((m.ullTotalPhys - m.ullAvailPhys, m.ullTotalPhys))
}

#[cfg(not(windows))]
pub fn ram() -> Option<(u64, u64)> {
    None
}

/// This process's working set (resident RAM) in bytes.
#[cfg(windows)]
pub fn process_ram() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS::default();
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the current-process pseudo handle needs no closing; `c` is valid for `cb` bytes.
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, cb) } == 0 {
        return None;
    }
    Some(c.WorkingSetSize as u64)
}

#[cfg(not(windows))]
pub fn process_ram() -> Option<u64> {
    None
}

/// Logical CPU cores (1 if unknown).
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// (free, total) bytes on the volume holding `path` (free = available to this user).
#[cfg(windows)]
pub fn disk(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: `wide` is NUL-terminated; the out pointers are valid u64s (the third may be null).
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((free, total))
}

#[cfg(not(windows))]
pub fn disk(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}

/// OS name and build, e.g. "Windows 11 (build 26200)". Uses RtlGetVersion, which (unlike the
/// deprecated GetVersionExW) reports the real version without an application manifest.
#[cfg(windows)]
pub fn os_version() -> Option<String> {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut v = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `v` is a valid OSVERSIONINFOW with its size field set; RtlGetVersion only fills it.
    if unsafe { RtlGetVersion(&mut v) } != 0 {
        return None;
    }
    Some(windows_name(v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber))
}

#[cfg(not(windows))]
pub fn os_version() -> Option<String> {
    None
}

/// Windows 11 still reports version 10.0; it is told apart by build number 22000+.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_name(major: u32, minor: u32, build: u32) -> String {
    match (major, minor) {
        (10, 0) if build >= 22000 => format!("Windows 11 (build {build})"),
        (10, 0) => format!("Windows 10 (build {build})"),
        _ => format!("Windows {major}.{minor} (build {build})"),
    }
}

/// System-wide CPU usage from the change in GetSystemTimes between two samples.
#[derive(Default)]
pub struct CpuMeter {
    prev: Option<CpuTimes>,
}

/// Cumulative (idle, kernel, user) time in 100 ns units; kernel includes idle.
#[derive(Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

impl CpuMeter {
    /// Busy percent (0-100) since the previous sample; `None` on the first call or where unsupported.
    pub fn sample(&mut self) -> Option<f64> {
        let now = cpu_times()?;
        let prev = self.prev.replace(now)?;
        busy_percent(prev, now)
    }
}

/// Busy share of the elapsed CPU time between two samples; `None` if no time passed.
fn busy_percent(prev: CpuTimes, now: CpuTimes) -> Option<f64> {
    let idle = now.idle.saturating_sub(prev.idle);
    let total = now.kernel.saturating_sub(prev.kernel) + now.user.saturating_sub(prev.user);
    if total == 0 {
        return None;
    }
    Some(100.0 * total.saturating_sub(idle) as f64 / total as f64)
}

#[cfg(windows)]
fn cpu_times() -> Option<CpuTimes> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: three valid FILETIME out pointers that outlive the call.
    if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
        return None;
    }
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Some(CpuTimes { idle: t(i), kernel: t(k), user: t(u) })
}

#[cfg(not(windows))]
fn cpu_times() -> Option<CpuTimes> {
    None
}

#[cfg(test)]
mod tests {
    use super::{busy_percent, windows_name, CpuTimes};

    #[test]
    #[cfg(windows)]
    fn reads_physical_memory() {
        let (used, total) = super::ram().unwrap();
        assert!(total > 0 && used <= total);
    }

    #[test]
    #[cfg(windows)]
    fn reads_process_disk_and_os() {
        assert!(super::process_ram().unwrap() > 0);
        let (free, total) = super::disk(std::path::Path::new(".")).unwrap();
        assert!(total > 0 && free <= total);
        assert!(super::os_version().unwrap().starts_with("Windows"));
        assert!(super::cores() >= 1);
    }

    fn t(idle: u64, kernel: u64, user: u64) -> CpuTimes {
        CpuTimes { idle, kernel, user }
    }

    #[test]
    fn cpu_busy_percent() {
        // 100 elapsed (kernel 60 incl. idle 50, user 40): busy = 100 - 50.
        assert_eq!(busy_percent(t(0, 0, 0), t(50, 60, 40)), Some(50.0));
        assert_eq!(busy_percent(t(10, 20, 30), t(110, 120, 30)), Some(0.0));
        assert_eq!(busy_percent(t(10, 20, 30), t(10, 70, 80)), Some(100.0));
        assert_eq!(busy_percent(t(5, 5, 5), t(5, 5, 5)), None);
    }

    #[test]
    fn names_windows_versions() {
        assert_eq!(windows_name(10, 0, 26200), "Windows 11 (build 26200)");
        assert_eq!(windows_name(10, 0, 19045), "Windows 10 (build 19045)");
        assert_eq!(windows_name(6, 3, 9600), "Windows 6.3 (build 9600)");
    }
}
