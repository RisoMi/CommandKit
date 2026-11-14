//! 平台相关：进程树终止、端口占用探测、外部终端。

use std::time::Duration;

/// 终止整棵进程树。
/// - Windows：`taskkill /PID <pid> /T /F`（子进程在独立 console group 中，/T 递归整棵树）
/// - macOS / Linux：子进程以 `process_group(0)` 启动，pgid == pid，`kill(-pid)` 命中整组，
///   先 SIGTERM 给机会收尾，2s 后 SIGKILL 兜底。
pub async fn terminate_tree(pid: u32) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let out = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("调用 taskkill 失败：{e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // 进程已不存在视为成功
        if msg.contains("not found") || msg.contains("没有找到") || msg.contains("找不到") {
            Ok(())
        } else {
            Err(format!("taskkill 失败：{msg}"))
        }
    }
    #[cfg(unix)]
    {
        let group = -(pid as i32);
        let alive = unsafe { libc::kill(group, 0) } == 0 || unsafe { libc::kill(pid as i32, 0) } == 0;
        if !alive {
            return Ok(());
        }
        unsafe {
            libc::kill(group, libc::SIGTERM);
            libc::kill(pid as i32, libc::SIGTERM);
        }
        // 升级到 SIGKILL：不阻塞调用方
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2000)).await;
            unsafe {
                libc::kill(group, libc::SIGKILL);
                libc::kill(pid as i32, libc::SIGKILL);
            }
        });
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err("当前平台不支持进程树终止".into())
    }
}

/// 探测端口占用者，返回形如 `PID 18422 · node.exe` 的描述（尽力而为，失败返回 None）。
pub async fn port_owner(port: u16) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("lsof")
            .args([
                "-nP",
                &format!("-iTCP:{port}"),
                "-sTCP:LISTEN",
                "-Fpc",
            ])
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut pid = None;
        let mut cmd = None;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix('p') {
                pid = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix('c') {
                cmd = Some(v.trim().to_string());
            }
        }
        match (pid, cmd) {
            (Some(p), Some(c)) => Some(format!("PID {p} · {c}")),
            (Some(p), None) => Some(format!("PID {p}")),
            _ => None,
        }
    }
    #[cfg(windows)]
    {
        let out = tokio::process::Command::new("netstat").args(["-ano"]).output().await.ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let mut cols = line.split_whitespace();
            let local = cols.next().unwrap_or("");
            let _remote = cols.next();
            let _state = cols.next();
            let pid = cols.next().unwrap_or("");
            if local.ends_with(&format!(":{port}")) && !pid.is_empty() && pid != "0" {
                return Some(format!("PID {pid}"));
            }
        }
        None
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let out = tokio::process::Command::new("ss").args(["-ltnp"]).output().await.ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains(&format!(":{port}")) {
                if let Some(idx) = line.find("pid=") {
                    let rest = &line[idx + 4..];
                    let pid: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if !pid.is_empty() {
                        return Some(format!("PID {pid}"));
                    }
                }
            }
        }
        None
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = port;
        None
    }
}

/// 端口是否已被占用（用于启动前预检）
pub fn port_busy(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// 在系统终端里打开某目录（「打开外部终端」）
pub fn open_terminal(dir: &str) -> Result<(), String> {
    if dir.trim().is_empty() {
        return Err("工作目录为空".into());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-a", "Terminal", dir])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开终端失败：{e}"))
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "cmd", "/k", &format!("cd /d {dir}")])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开终端失败：{e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("x-terminal-emulator")
            .current_dir(dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开终端失败：{e}"))
    }
}

/// 在系统文件管理器中打开目录
pub fn open_path(path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}
