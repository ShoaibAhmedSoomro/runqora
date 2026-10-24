//! The Unix twin of the Windows `win` module: the same small set of helpers (who listens on a port, is a process alive,
//! end it) built on signals, /proc and lsof, so the rest of Runqora does not care which system it runs on.

#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

/// Every TCP listener (IPv4 and IPv6) as (port, pid). The pid is 0 for a listener of another user that we may not look into.
pub fn listeners() -> Vec<(u16, u32)> {
    #[cfg(target_os = "linux")]
    {
        linux_listeners()
    }
    #[cfg(target_os = "macos")]
    {
        macos_listeners()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// PID listening on this TCP port, if any.
pub fn listener_pid(port: u16) -> Option<u32> {
    listeners().into_iter().find(|(p, _)| *p == port).map(|(_, pid)| pid)
}

/// The (port, socket inode) of every listening socket in the lines of /proc/net/tcp or tcp6.
#[cfg(any(target_os = "linux", test))]
pub fn parse_proc_net_tcp(text: &str) -> Vec<(u16, u64)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            // columns: sl local_address rem_address st tx:rx tr:when retrnsmt uid timeout inode ...
            if c.len() < 10 || c[3] != "0A" {
                return None; // 0A = LISTEN
            }
            let port = u16::from_str_radix(c[1].rsplit(':').next()?, 16).ok()?;
            Some((port, c[9].parse().ok()?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_listeners() -> Vec<(u16, u32)> {
    use std::{collections::HashMap, fs};
    let mut sockets: Vec<(u16, u64)> = vec![];
    for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(t) = fs::read_to_string(f) {
            sockets.extend(parse_proc_net_tcp(&t));
        }
    }
    // which process owns which socket: the symlinks in /proc/<pid>/fd read "socket:[inode]" (only our own processes are readable)
    let mut owner: HashMap<u64, u32> = HashMap::new();
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        for fd in fs::read_dir(e.path().join("fd")).into_iter().flatten().flatten() {
            if let Some(inode) = fs::read_link(fd.path()).ok().and_then(|l| l.to_str().and_then(|s| s.strip_prefix("socket:[")?.strip_suffix(']')?.parse::<u64>().ok())) {
                owner.insert(inode, pid);
            }
        }
    }
    sockets.into_iter().map(|(port, inode)| (port, owner.get(&inode).copied().unwrap_or(0))).collect()
}

/// (port, pid) pairs from `lsof -F pn` output: a `p<pid>` line, then `n<address>:<port>` lines for that process.
#[cfg(any(target_os = "macos", test))]
pub fn parse_lsof(text: &str) -> Vec<(u16, u32)> {
    let (mut pid, mut out) = (0u32, vec![]);
    for l in text.lines() {
        match (l.chars().next(), l.get(1..)) {
            (Some('p'), Some(r)) => pid = r.parse().unwrap_or(0),
            (Some('n'), Some(r)) => {
                if let Some(port) = r.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
                    out.push((port, pid));
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn macos_listeners() -> Vec<(u16, u32)> {
    let Ok(o) = Command::new("lsof").args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"]).stdin(Stdio::null()).output() else { return Vec::new() };
    parse_lsof(&String::from_utf8_lossy(&o.stdout))
}

/// The program's full path (not available for other users' protected processes).
pub fn image_path(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let o = Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).stdin(Stdio::null()).output().ok()?;
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
        (o.status.success() && !t.is_empty()).then_some(t) // on macOS `comm` is the full path
    }
}

/// The program's name.
pub fn process_name(pid: u32) -> Option<String> {
    if let Some(p) = image_path(pid) {
        return p.rsplit('/').next().map(str::to_string).filter(|n| !n.is_empty());
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Is there a live process with this id? A finished child that nobody has collected yet (a zombie) does not count.
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists and may be signalled
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    let exists = rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    #[cfg(target_os = "linux")]
    if exists {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // "pid (name) S ...": the state follows the last ')'
            return stat.rsplit(')').next().and_then(|r| r.trim().chars().next()) != Some('Z');
        }
    }
    exists
}

/// Ask a process to end (SIGTERM: servers finish their work and exit). Err carries the errno.
pub fn terminate(pid: u32) -> Result<(), u32> {
    // SAFETY: plain signal delivery
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32) }
}

/// End a process and everything it started, now (SIGKILL to its process group, then to the process itself).
pub fn kill_tree(pid: u32) {
    // SAFETY: plain signal delivery; servers Runqora starts lead their own process group
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Nothing to do: a child Runqora starts does not inherit handles the way Windows children do.
pub fn keep_std_handles_private() {}

// Windows services do not exist here; the same calls simply answer "no".
pub fn service_exists(_name: &str) -> bool {
    false
}

pub fn service_image_path(_name: &str) -> Option<String> {
    None
}

pub fn run_elevated(_file: &str, _params: &str) -> Result<u32, String> {
    Err("administrator actions are a Windows feature; nothing here needs them".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_net_tcp_lines_become_listening_ports() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 100 0 0 10 0\n\
   1: 0100007F:C350 0100007F:0050 01 00000000:00000000 00:00000000 00000000  1000        0 99999 1 0000000000000000 100 0 0 10 0\n\
   2: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 777 1 0000000000000000 100 0 0 10 0\n";
        assert_eq!(parse_proc_net_tcp(text), [(8080, 12345), (80, 777)], "only LISTEN rows (state 0A), port read as hex");
        assert!(parse_proc_net_tcp("header only\n").is_empty());
    }

    #[test]
    fn lsof_output_pairs_each_port_with_its_process() {
        let text = "p123\nn*:80\nn127.0.0.1:8025\np456\nn[::1]:3306\nnnot-a-port\n";
        assert_eq!(parse_lsof(text), [(80, 123), (8025, 123), (3306, 456)]);
    }

    #[test]
    fn a_real_listener_is_found_and_a_dead_pid_is_not_alive() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(4_000_000), "no such process");
        // the process table lists it (owner unknown only for other users' processes)
        let found = listeners().into_iter().find(|(p, _)| *p == port);
        assert!(found.is_some(), "port {port} should be listed");
        if let Some((_, pid)) = found {
            assert!(pid == 0 || pid == std::process::id());
        }
    }
}
