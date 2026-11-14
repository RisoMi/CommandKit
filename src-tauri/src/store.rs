//! 数据目录解析与 JSON 持久化（原子写入 + 备份）。
//!
//! - Windows：便携模式，配置一律放在 exe 同目录 `data\`，用 `current_exe().parent()` 定位，
//!   不使用 `app_data_dir()`（那会落到 %APPDATA%，与「整目录拷贝即迁移」冲突）。
//! - macOS：`~/Library/Application Support/CommandKit/`。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

pub const APP_DIR: &str = "CommandKit";

pub fn home_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// 解析数据根目录。`fallback` 由 Tauri 的 app_data_dir 提供，仅作为极端情况的兜底。
pub fn resolve_data_root(fallback: PathBuf) -> PathBuf {
    if cfg!(target_os = "windows") {
        // 便携核心：exe 所在目录，不依赖 cwd、不写 AppData
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .map(|dir| dir.join("data"))
            .unwrap_or(fallback)
    } else if cfg!(target_os = "macos") {
        home_dir()
            .map(|h| h.join("Library").join("Application Support").join(APP_DIR))
            .unwrap_or(fallback)
    } else {
        home_dir()
            .map(|h| h.join(".config").join("CommandKit"))
            .unwrap_or(fallback)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 本地时间戳（日志文件抬头用）
pub fn fmt_local(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

/// 目录可写预检：无法写入时前端要明确提示（Program Files / 只读 U 盘 / 网络盘）。
pub fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let probe = dir.join(".write-probe");
    let mut f = File::create(&probe)?;
    f.write_all(b"ok")?;
    f.sync_all()?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// 原子 JSON 文件：写入 `xxx.tmp` → fsync → rename 覆盖；覆盖前把旧文件备份到 `backup/`。
pub struct JsonFile {
    path: PathBuf,
    backup_dir: PathBuf,
    keep: usize,
}

impl JsonFile {
    pub fn new(dir: &Path, name: &str) -> Self {
        Self {
            path: dir.join(name),
            backup_dir: dir.join("backup"),
            keep: 10,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read_or<T: DeserializeOwned + Default>(&self) -> T {
        match fs::read_to_string(&self.path) {
            Ok(text) if !text.trim().is_empty() => match serde_json::from_str::<T>(&text) {
                Ok(v) => v,
                Err(err) => {
                    // 损坏文件不阻塞启动：先另存一份，再回退默认值
                    eprintln!("[commandkit] 解析 {} 失败：{err}", self.path.display());
                    let _ = fs::rename(&self.path, self.path.with_extension("corrupt"));
                    T::default()
                }
            },
            _ => T::default(),
        }
    }

    pub fn write<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        // 备份上一版（保留最近 keep 份）
        if self.path.exists() {
            let _ = fs::create_dir_all(&self.backup_dir);
            let stem = self
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "data".into());
            let stamp = now_ms();
            let target = self.backup_dir.join(format!("{stem}.{stamp}.json"));
            let _ = fs::copy(&self.path, target);
            prune_backups(&self.backup_dir, &stem, self.keep);
        }
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn prune_backups(dir: &Path, stem: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{stem}.")) && name.ends_with(".json") {
                Some((name.clone(), e.path()))
            } else {
                None
            }
        })
        .collect();
    if items.len() <= keep {
        return;
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let drop_count = items.len() - keep;
    for (_, path) in items.into_iter().take(drop_count) {
        let _ = fs::remove_file(path);
    }
}

/// 单实例文件锁：进程退出自动释放；防止两个实例并发写 templates.json。
pub fn acquire_lock(dir: &Path) -> Option<File> {
    use fs2::FileExt;
    let path = dir.join(".commandkit.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .ok()?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = fs::write(dir.join(".commandkit.pid"), std::process::id().to_string());
            Some(file)
        }
        Err(_) => None,
    }
}
