//! Network throughput and disk IO.
//!
//! - **Windows**: `GetIfTable2` (IP Helper API) for network,
//!   `IOCTL_DISK_PERFORMANCE` sent directly to each `\\.\PhysicalDriveN`
//!   for disk — both native WinAPI, no PDH/external library.
//! - **Linux**: `/proc/net/dev` for network, `/proc/diskstats` for disk
//!   — two standard kernel text files, always present on every distro, no
//!   root privileges needed to read them.

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::time::Instant;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
    use windows::Win32::System::IO::DeviceIoControl;

    /// "Software loopback" interface type (RFC 2863 `ifType`) — always
    /// skipped so it isn't counted as real network traffic.
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    /// Only these two types are considered PHYSICAL adapters (Ethernet cable
    /// & WiFi). Virtual adapters (Hyper-V Default Switch, WSL, VMware, VPN,
    /// etc.) often do NOT have a loopback Type but still aren't a real
    /// internet connection — if that adapter's internal traffic (e.g. WSL
    /// <-> host sync) happens to be larger than internet traffic, the
    /// "largest delta" heuristic below could pick the wrong adapter,
    /// producing a number far above the real ISP limit.
    const PHYSICAL_IF_TYPES: [u32; 2] = [6, 71]; // IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211
    /// Keywords in the adapter's `Description` that get it discarded even if
    /// its Type happens to match Ethernet/WiFi (many virtual switches also
    /// use the Ethernet Type).
    const VIRTUAL_IF_KEYWORDS: [&str; 11] = [
        "virtual", "hyper-v", "vethernet", "wsl", "vmware", "virtualbox",
        "tunnel", "bluetooth", "tap-windows", "npcap", "pseudo",
    ];

    pub struct NetDiskMonitor {
        // The LAST byte counter per interface (key: InterfaceIndex), NOT
        // summed into one total. Windows often has several virtual adapters
        // (Hyper-V Default Switch, WSL, etc.) that just "mirror" the traffic
        // of the same physical adapter — if all interfaces were summed
        // directly, real traffic could be counted multiple times. So on each
        // sample we take the SINGLE interface with the largest delta
        // (assuming that's the physical adapter actually in use), not the
        // total of all interfaces.
        net_prev: HashMap<u32, (u64, u64)>,
        prev_disk_read: u64,
        prev_disk_write: u64,
        prev_time: Instant,
        primed: bool,
    }

    impl NetDiskMonitor {
        pub fn new() -> Self {
            Self {
                net_prev: HashMap::new(),
                prev_disk_read: 0,
                prev_disk_write: 0,
                prev_time: Instant::now(),
                primed: false,
            }
        }

        /// Returns `(net_down_kb_s, net_up_kb_s, disk_read_mb_s, disk_write_mb_s)`.
        pub fn sample(&mut self) -> (f64, f64, f64, f64) {
            let net_rows = read_network_rows();
            let (disk_read, disk_write) = read_disk_totals();

            let now = Instant::now();
            let elapsed = now.duration_since(self.prev_time).as_secs_f64().max(0.001);

            let mut best_down = 0u64;
            let mut best_up = 0u64;
            if self.primed {
                for &(idx, in_bytes, out_bytes) in &net_rows {
                    if let Some(&(prev_in, prev_out)) = self.net_prev.get(&idx) {
                        let d_in = in_bytes.saturating_sub(prev_in);
                        let d_out = out_bytes.saturating_sub(prev_out);
                        if d_in + d_out > best_down + best_up {
                            best_down = d_in;
                            best_up = d_out;
                        }
                    }
                }
            }

            self.net_prev = net_rows
                .into_iter()
                .map(|(idx, i, o)| (idx, (i, o)))
                .collect();

            let result = if !self.primed {
                (0.0, 0.0, 0.0, 0.0)
            } else {
                (
                    best_down as f64 / elapsed / 1024.0,
                    best_up as f64 / elapsed / 1024.0,
                    disk_read.saturating_sub(self.prev_disk_read) as f64 / elapsed / 1_048_576.0,
                    disk_write.saturating_sub(self.prev_disk_write) as f64
                        / elapsed
                        / 1_048_576.0,
                )
            };

            self.prev_disk_read = disk_read;
            self.prev_disk_write = disk_write;
            self.prev_time = now;
            self.primed = true;

            result
        }
    }

    /// `(InterfaceIndex, InOctets, OutOctets)` per physical/active interface
    /// (not loopback), since boot — compared across two calls PER INTERFACE
    /// (see the comment on `net_prev`) to get throughput.
    fn read_network_rows() -> Vec<(u32, u64, u64)> {
        unsafe {
            let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table_ptr).is_err() || table_ptr.is_null() {
                return Vec::new();
            }

            let table = &*table_ptr;
            let count = table.NumEntries as usize;
            // `Table` is a flexible array member at the end of the original
            // C struct; in the Rust binding it's represented as a 1-element
            // array, so it has to be read manually as a slice of length
            // `NumEntries` via a pointer to the first element.
            let rows = std::slice::from_raw_parts(table.Table.as_ptr(), count);

            let mut out = Vec::with_capacity(count);
            for row in rows {
                if row.Type == IF_TYPE_SOFTWARE_LOOPBACK {
                    continue;
                }
                if row.OperStatus != IfOperStatusUp {
                    continue;
                }
                if !PHYSICAL_IF_TYPES.contains(&row.Type) {
                    continue;
                }
                let description = wide_to_string(&row.Description).to_ascii_lowercase();
                if VIRTUAL_IF_KEYWORDS.iter().any(|kw| description.contains(kw)) {
                    continue;
                }
                out.push((row.InterfaceIndex, row.InOctets, row.OutOctets));
            }

            FreeMibTable(table_ptr as *const _);
            out
        }
    }

    /// Convert a null-terminated `WCHAR` array from a WinAPI struct into a `String`.
    fn wide_to_string(chars: &[u16]) -> String {
        let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
        String::from_utf16_lossy(&chars[..len])
    }

    /// Total bytes read/written across all physical drives that could be
    /// opened (index 0..15), since boot.
    fn read_disk_totals() -> (u64, u64) {
        let mut total_read = 0u64;
        let mut total_write = 0u64;

        for i in 0..16u32 {
            let path = format!(r"\\.\PhysicalDrive{i}");
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

            unsafe {
                let handle = CreateFileW(
                    windows::core::PCWSTR(wide.as_ptr()),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                );
                let Ok(handle) = handle else {
                    // This drive index doesn't exist — the next index might
                    // still be valid (they aren't always tightly sequential),
                    // so keep going to the next iteration instead of `break`.
                    continue;
                };

                let mut perf = DISK_PERFORMANCE::default();
                let mut bytes_returned = 0u32;
                let ok = DeviceIoControl(
                    handle,
                    IOCTL_DISK_PERFORMANCE,
                    None,
                    0,
                    Some(&mut perf as *mut _ as *mut _),
                    std::mem::size_of::<DISK_PERFORMANCE>() as u32,
                    Some(&mut bytes_returned),
                    None,
                );
                let _ = CloseHandle(handle);

                if ok.is_ok() {
                    total_read = total_read.saturating_add(perf.BytesRead as u64);
                    total_write = total_write.saturating_add(perf.BytesWritten as u64);
                }
            }
        }

        (total_read, total_write)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::fs;
    use std::time::Instant;

    /// Virtual interface name prefixes that are discarded so they don't
    /// confuse the "largest delta" heuristic (Docker/Podman bridges, veth
    /// container pairs, WireGuard, PPP, etc.) — analogous to
    /// `VIRTUAL_IF_KEYWORDS` on the Windows path.
    const VIRTUAL_IF_PREFIXES: [&str; 9] =
        ["veth", "docker", "br-", "virbr", "tun", "tap", "wg", "ppp", "vnet"];

    pub struct NetDiskMonitor {
        // Same as Windows: store the LAST counter per interface (key:
        // interface name, e.g. "enp3s0", "wlan0"), take the LARGEST delta
        // (not the sum of all interfaces) on each sample — see the long
        // comment in the Windows version for why.
        net_prev: HashMap<String, (u64, u64)>,
        prev_disk_read: u64,
        prev_disk_write: u64,
        prev_time: Instant,
        primed: bool,
    }

    impl NetDiskMonitor {
        pub fn new() -> Self {
            Self {
                net_prev: HashMap::new(),
                prev_disk_read: 0,
                prev_disk_write: 0,
                prev_time: Instant::now(),
                primed: false,
            }
        }

        /// Returns `(net_down_kb_s, net_up_kb_s, disk_read_mb_s, disk_write_mb_s)`.
        pub fn sample(&mut self) -> (f64, f64, f64, f64) {
            let net_rows = read_network_rows();
            let (disk_read, disk_write) = read_disk_totals();

            let now = Instant::now();
            let elapsed = now.duration_since(self.prev_time).as_secs_f64().max(0.001);

            let mut best_down = 0u64;
            let mut best_up = 0u64;
            if self.primed {
                for (iface, in_bytes, out_bytes) in &net_rows {
                    if let Some(&(prev_in, prev_out)) = self.net_prev.get(iface) {
                        let d_in = in_bytes.saturating_sub(prev_in);
                        let d_out = out_bytes.saturating_sub(prev_out);
                        if d_in + d_out > best_down + best_up {
                            best_down = d_in;
                            best_up = d_out;
                        }
                    }
                }
            }

            self.net_prev = net_rows
                .into_iter()
                .map(|(name, i, o)| (name, (i, o)))
                .collect();

            let result = if !self.primed {
                (0.0, 0.0, 0.0, 0.0)
            } else {
                (
                    best_down as f64 / elapsed / 1024.0,
                    best_up as f64 / elapsed / 1024.0,
                    disk_read.saturating_sub(self.prev_disk_read) as f64 / elapsed / 1_048_576.0,
                    disk_write.saturating_sub(self.prev_disk_write) as f64
                        / elapsed
                        / 1_048_576.0,
                )
            };

            self.prev_disk_read = disk_read;
            self.prev_disk_write = disk_write;
            self.prev_time = now;
            self.primed = true;

            result
        }
    }

    /// `(interface_name, rx_bytes, tx_bytes)` from `/proc/net/dev`, since
    /// boot — compared across two calls PER INTERFACE for throughput.
    fn read_network_rows() -> Vec<(String, u64, u64)> {
        let Ok(content) = fs::read_to_string("/proc/net/dev") else {
            return Vec::new();
        };

        let mut out = Vec::new();
        // First 2 header lines ("Inter-|   Receive ..." and "face |bytes ...").
        for line in content.lines().skip(2) {
            let Some((iface, rest)) = line.split_once(':') else { continue };
            let iface = iface.trim();
            if iface.is_empty() || iface == "lo" {
                continue;
            }
            let lname = iface.to_ascii_lowercase();
            if VIRTUAL_IF_PREFIXES.iter().any(|p| lname.starts_with(p)) {
                continue;
            }

            let fields: Vec<&str> = rest.split_whitespace().collect();
            // Columns (0-based) per the /proc/net/dev format:
            // 0=rx_bytes ... 8=tx_bytes.
            if fields.len() < 9 {
                continue;
            }
            let rx_bytes: u64 = fields[0].parse().unwrap_or(0);
            let tx_bytes: u64 = fields[8].parse().unwrap_or(0);
            out.push((iface.to_string(), rx_bytes, tx_bytes));
        }
        out
    }

    /// Check whether a device name in `/proc/diskstats` is a WHOLE DISK (not
    /// a partition) — so it isn't double-counted (e.g. "sda" AND "sda1" both
    /// summed, even though "sda1"'s sectors are already included in "sda").
    fn is_whole_disk(name: &str) -> bool {
        if name.starts_with("loop")
            || name.starts_with("dm-")
            || name.starts_with("md")
            || name.starts_with("zram")
            || name.starts_with("sr")
            || name.starts_with("fd")
        {
            return false;
        }
        if let Some(rest) = name.strip_prefix("nvme") {
            // "0n1" (whole disk) vs "0n1p1" (partition)
            return !rest.contains('p');
        }
        if let Some(rest) = name.strip_prefix("mmcblk") {
            // "0" (whole disk) vs "0p1" (partition)
            return !rest.contains('p');
        }
        for prefix in ["sd", "vd", "xvd", "hd"] {
            if let Some(rest) = name.strip_prefix(prefix) {
                // "a" (whole disk) vs "a1" (partition)
                return !rest.chars().any(|c| c.is_ascii_digit());
            }
        }
        false
    }

    /// Total bytes read/written (sectors × 512) across all WHOLE disks (not
    /// individual partitions) in `/proc/diskstats`, since boot.
    fn read_disk_totals() -> (u64, u64) {
        const SECTOR_BYTES: u64 = 512;
        let Ok(content) = fs::read_to_string("/proc/diskstats") else {
            return (0, 0);
        };

        let mut total_read_sectors = 0u64;
        let mut total_write_sectors = 0u64;
        for line in content.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // Field (0-based): 2=device name, 5=sectors read, 9=sectors written.
            if fields.len() < 10 {
                continue;
            }
            let name = fields[2];
            if !is_whole_disk(name) {
                continue;
            }
            let read_sectors: u64 = fields[5].parse().unwrap_or(0);
            let write_sectors: u64 = fields[9].parse().unwrap_or(0);
            total_read_sectors = total_read_sectors.saturating_add(read_sectors);
            total_write_sectors = total_write_sectors.saturating_add(write_sectors);
        }

        (total_read_sectors * SECTOR_BYTES, total_write_sectors * SECTOR_BYTES)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub struct NetDiskMonitor;

    impl NetDiskMonitor {
        pub fn new() -> Self {
            Self
        }

        pub fn sample(&mut self) -> (f64, f64, f64, f64) {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}

pub use imp::NetDiskMonitor;
