//! System / environment info collected once at startup and pushed to the server
//! after auth, so the admin monitoring page can show WHO is connected and WHERE.
//!
//! Deliberately dependency-free: OS family + arch + core count come from `std`;
//! the richer bits (OS version string, hostname, total memory, a stable machine
//! fingerprint) come from small, best-effort platform shell-outs that degrade to
//! empty/zero rather than failing. Nothing here blocks or panics — partial info
//! is fine, the server fields are all optional.

use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The system-info payload sent to the server (matches the server's `ClientInfo`,
/// serde field names line up).
#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub cpu_cores: u32,
    pub mem_mb: u64,
    pub hostname: String,
    pub machine_id: String,
    pub client_version: String,
}

/// Collect system info. Cheap enough to call once at connect time. Each field is
/// independently best-effort.
pub fn collect() -> SystemInfo {
    SystemInfo {
        os: os_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_cores: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(0),
        mem_mb: total_mem_mb(),
        hostname: hostname(),
        machine_id: machine_fingerprint(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// OS family + a human version string, e.g. "macOS 15.5" / "Windows 11" /
/// "Linux 6.8". Falls back to just the family if the version probe fails.
fn os_string() -> String {
    let family = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let ver = os_version();
    if ver.is_empty() {
        family.to_string()
    } else {
        format!("{family} {ver}")
    }
}

#[cfg(target_os = "macos")]
fn os_version() -> String {
    run("sw_vers", &["-productVersion"])
}

#[cfg(target_os = "windows")]
fn os_version() -> String {
    // `cmd /c ver` yields e.g. "Microsoft Windows [Version 10.0.22631.3527]".
    // Extract the build to distinguish 10 vs 11 (11 is build >= 22000).
    let raw = run("cmd", &["/c", "ver"]);
    if let Some(start) = raw.find("Version ") {
        let rest = &raw[start + 8..];
        let ver = rest.trim_end_matches(']').trim();
        // Map build to a friendly major.
        if let Some(build) = ver.split('.').nth(2).and_then(|b| b.parse::<u32>().ok()) {
            return if build >= 22000 { "11".into() } else { "10".into() };
        }
        return ver.to_string();
    }
    String::new()
}

#[cfg(target_os = "linux")]
fn os_version() -> String {
    // Kernel release is a reliable, dependency-free signal.
    run("uname", &["-r"])
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn os_version() -> String {
    String::new()
}

fn hostname() -> String {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let h = run("hostname", &[]);
        if !h.is_empty() {
            return h;
        }
    }
    // Portable fallback via env (Windows sets COMPUTERNAME; *nix sometimes HOSTNAME).
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn total_mem_mb() -> u64 {
    // sysctl hw.memsize → bytes.
    run("sysctl", &["-n", "hw.memsize"])
        .parse::<u64>()
        .map(|b| b / 1024 / 1024)
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn total_mem_mb() -> u64 {
    // /proc/meminfo MemTotal is in kB.
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn total_mem_mb() -> u64 {
    // wmic is deprecated but still present; PowerShell CIM is the robust path.
    let out = run(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ],
    );
    out.parse::<u64>().map(|b| b / 1024 / 1024).unwrap_or(0)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn total_mem_mb() -> u64 {
    0
}

/// A stable per-machine fingerprint: the platform machine id hashed (so the raw
/// hardware id never leaves the device), truncated to 16 hex chars. Empty if no
/// id source is available.
fn machine_fingerprint() -> String {
    let raw = raw_machine_id();
    if raw.is_empty() {
        return String::new();
    }
    let mut h = Sha256::new();
    h.update(raw.as_bytes());
    let digest = hex::encode(h.finalize());
    digest[..16].to_string()
}

#[cfg(target_os = "macos")]
fn raw_machine_id() -> String {
    // IOPlatformUUID is stable per machine.
    let out = run("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"]);
    for line in out.lines() {
        if line.contains("IOPlatformUUID") {
            if let Some(eq) = line.find('=') {
                return line[eq + 1..].trim().trim_matches('"').to_string();
            }
        }
    }
    String::new()
}

#[cfg(target_os = "linux")]
fn raw_machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn raw_machine_id() -> String {
    // MachineGuid under HKLM is stable per Windows install.
    let out = run(
        "reg",
        &[
            "query",
            "HKLM\\SOFTWARE\\Microsoft\\Cryptography",
            "/v",
            "MachineGuid",
        ],
    );
    for line in out.lines() {
        if line.contains("MachineGuid") {
            if let Some(tok) = line.split_whitespace().last() {
                return tok.to_string();
            }
        }
    }
    String::new()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn raw_machine_id() -> String {
    String::new()
}

/// Run a command, return trimmed stdout, or "" on any failure. Never panics.
fn run(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_fills_basics() {
        let info = collect();
        // arch + cores + version come from std → always present on a test host.
        assert!(!info.arch.is_empty());
        assert!(info.cpu_cores >= 1);
        assert!(!info.client_version.is_empty());
        assert!(!info.os.is_empty());
    }

    #[test]
    fn fingerprint_is_hashed_and_bounded() {
        let info = collect();
        // Either empty (no id source) or exactly 16 hex chars (never the raw id).
        assert!(info.machine_id.is_empty() || info.machine_id.len() == 16);
        assert!(info.machine_id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
