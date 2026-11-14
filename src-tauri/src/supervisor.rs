//! 进程监管核心：独立子 shell 启动、输出流转发、健康检查、进程树终止、批量编排。
//!
//! 设计要点：
//! - 每个项目 / 模板一个独立子 shell（`zsh -lc` / `pwsh -NoProfile -Command`），
//!   env 由 Rust 侧构造后随 spawn 注入，从不调用 setx / 注册表 / shell profile 去改系统环境。
//! - 终止统一走进程树：Windows `taskkill /T /F`，macOS `kill(-pgid)`。
//! - 单条操作只影响自己的会话，批量操作按 dependsOn 拓扑序编排。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter, Runtime};

/// 事件出口：进程监管只依赖这个极小接口，不依赖 Tauri 运行时。
/// 这样测试可以用 NullSink，不必链接 wry / WebView2（Windows 上会 STATUS_ENTRYPOINT_NOT_FOUND）。
pub trait EventSink: Send + Sync + 'static {
    fn emit_event(&self, event: &str, payload: serde_json::Value);
}

/// 生产实现：转发到 Tauri 事件总线
pub struct TauriSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> EventSink for TauriSink<R> {
    fn emit_event(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }
}

/// 测试实现：吞掉事件
pub struct NullSink;

impl EventSink for NullSink {
    fn emit_event(&self, _event: &str, _payload: serde_json::Value) {}
}
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::model::*;
use crate::proc as procs;
use crate::state::{sanitize_id, AppState};
use crate::store;

pub const LOG_CAP_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct SpawnSpec {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category: String,
    pub command: String,
    pub workdir: String,
    pub env: BTreeMap<String, String>,
    pub ports: Vec<u16>,
    pub health: Option<HealthCheck>,
    pub policy: Policy,
    pub shell: Option<String>,
}

pub struct Session {
    pub id: String,
    pub kind: String,
    pub spec: Mutex<SpawnSpec>,
    pub inner: Mutex<Inner>,
}

pub struct Inner {
    pub status: Status,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
    pub started_instant: Option<Instant>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stopping: bool,
    pub restarts: u32,
    pub health_ok: bool,
    pub log: VecDeque<LogChunk>,
    pub log_bytes: usize,
    pub seq: u64,
    pub stdin: Option<mpsc::UnboundedSender<String>>,
    pub log_file: Option<std::fs::File>,
    /// 崩溃自动重启：由 tick 任务拉起，避免 start ⇄ finish 递归 await 造成非 Send
    pub restart_pending: bool,
}

impl Inner {
    fn new() -> Self {
        Self {
            status: Status::Stopped,
            pid: None,
            started_at: None,
            started_instant: None,
            exit_code: None,
            error: None,
            stopping: false,
            restarts: 0,
            health_ok: false,
            log: VecDeque::new(),
            log_bytes: 0,
            seq: 0,
            stdin: None,
            log_file: None,
            restart_pending: false,
        }
    }
}

impl Session {
    fn new(spec: SpawnSpec) -> Self {
        Self {
            id: spec.id.clone(),
            kind: spec.kind.clone(),
            spec: Mutex::new(spec),
            inner: Mutex::new(Inner::new()),
        }
    }

    pub fn view(&self) -> SessionView {
        let spec = self.spec.lock().unwrap().clone();
        let inner = self.inner.lock().unwrap();
        SessionView {
            id: self.id.clone(),
            name: spec.name,
            kind: self.kind.clone(),
            status: inner.status,
            pid: inner.pid,
            started_at: inner.started_at,
            exit_code: inner.exit_code,
            error: inner.error.clone(),
            restarts: inner.restarts,
            health_ok: inner.health_ok,
            command: spec.command,
            workdir: spec.workdir,
            ports: spec.ports,
            category: spec.category,
        }
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status
    }
}

fn emit_session(sink: &Arc<dyn EventSink>, s: &Session) {
    sink.emit_event(
        "ck:session",
        serde_json::to_value(s.view()).unwrap_or(serde_json::Value::Null),
    );
}

// ---------------------------------------------------------------- spec 构造

pub fn spec_from_project(p: &Project) -> SpawnSpec {
    SpawnSpec {
        id: p.id.clone(),
        name: p.name.clone(),
        kind: "project".into(),
        category: p.category.clone(),
        command: p.command.clone(),
        workdir: p.workdir.clone(),
        env: p.env.clone(),
        ports: p.ports.clone(),
        health: p.health.clone(),
        policy: p.policy.clone(),
        shell: p.shell.clone(),
    }
}

pub fn spec_from_template(t: &Template, shell: String) -> SpawnSpec {
    let command = t
        .commands
        .iter()
        .filter(|c| !c.cmd.trim().is_empty())
        .map(|c| c.cmd.clone())
        .collect::<Vec<_>>()
        .join(" && ");
    SpawnSpec {
        id: format!("tpl:{}", t.id),
        name: format!("模板 · {}", t.name),
        kind: "template".into(),
        category: "template".into(),
        command,
        workdir: t.workdir.clone(),
        env: t.env.clone(),
        ports: vec![],
        health: None,
        policy: Policy::default(),
        shell: t.shell.clone().or(Some(shell)),
    }
}

// ---------------------------------------------------------------- 命令构造

fn default_shell() -> String {
    if cfg!(windows) {
        "pwsh.exe".into()
    } else {
        "/bin/zsh".into()
    }
}

fn normalize_key(key: &str) -> String {
    if cfg!(windows) && key.eq_ignore_ascii_case("path") {
        "Path".into()
    } else {
        key.to_string()
    }
}

/// 变量展开：先解析本项目内的交叉引用，再解析宿主环境变量（`%VAR%` / `$VAR` / `${VAR}`）。
pub fn expand(value: &str, local: &BTreeMap<String, String>) -> String {
    let mut out = value.to_string();
    for _ in 0..4 {
        let before = out.clone();
        for (k, v) in local {
            out = out
                .replace(&format!("%{k}%"), v)
                .replace(&format!("${{{k}}}"), v)
                .replace(&format!("${k}"), v);
        }
        out = expand_system(&out);
        if out == before {
            break;
        }
    }
    out
}

fn expand_system(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && std::env::var(&name).is_ok()
                {
                    out.push_str(&std::env::var(&name).unwrap());
                    i = i + end + 2;
                    continue;
                }
            }
        }
        if chars[i] == '$' {
            let mut j = i + 1;
            if j < chars.len() && chars[j] == '{' {
                if let Some(end) = chars[j..].iter().position(|c| *c == '}') {
                    let name: String = chars[j + 1..j + end].iter().collect();
                    if let Ok(v) = std::env::var(&name) {
                        out.push_str(&v);
                        i = j + end + 1;
                        continue;
                    }
                }
            } else {
                let start = j;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j > start {
                    let name: String = chars[start..j].iter().collect();
                    if let Ok(v) = std::env::var(&name) {
                        out.push_str(&v);
                        i = j;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn resolve_shell(spec: &SpawnSpec, shell_setting: &str) -> String {
    spec.shell
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if shell_setting.trim().is_empty() {
                default_shell()
            } else {
                shell_setting.to_string()
            }
        })
}

fn build_command(spec: &SpawnSpec, shell: &str) -> Result<Command, String> {
    if spec.command.trim().is_empty() {
        return Err("命令为空，无法启动".into());
    }
    if spec.workdir.trim().is_empty() {
        return Err("工作目录为空".into());
    }
    let workdir = PathBuf::from(spec.workdir.trim());
    if !workdir.is_dir() {
        return Err(format!("工作目录不存在：{}", spec.workdir));
    }

    let mut cmd = Command::new(shell);
    if cfg!(windows) {
        cmd.arg("-NoProfile").arg("-Command").arg(&spec.command);
    } else {
        // -l：登录 shell，确保 Finder 启动的应用也能拿到用户 PATH
        cmd.arg("-lc").arg(&spec.command);
    }
    cmd.current_dir(&workdir);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in &spec.env {
        if k.trim().is_empty() {
            continue;
        }
        cmd.env(normalize_key(k.trim()), expand(v, &spec.env));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.as_std_mut()
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：pgid == pid，终止时 kill(-pid) 命中整棵树
        cmd.as_std_mut().process_group(0);
    }

    Ok(cmd)
}

// ---------------------------------------------------------------- 输出流转发

struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    fn new() -> Self {
        Self { pending: vec![] }
    }
    fn push(&mut self, bytes: &[u8]) -> String {
        let mut all = std::mem::take(&mut self.pending);
        all.extend_from_slice(bytes);
        match std::str::from_utf8(&all) {
            Ok(s) => s.to_string(),
            Err(err) => {
                let valid = err.valid_up_to();
                let text = String::from_utf8_lossy(&all[..valid]).to_string();
                let rest = &all[valid..];
                if rest.len() <= 3 {
                    self.pending = rest.to_vec();
                }
                text
            }
        }
    }
}

fn push_chunk(sink: &Arc<dyn EventSink>, session: &Session, data: &str, stream: &'static str) {
    if data.is_empty() {
        return;
    }
    let mut inner = session.inner.lock().unwrap();
    inner.seq += 1;
    let seq = inner.seq;
    inner.log_bytes += data.len();
    inner.log.push_back(LogChunk {
        seq,
        stream: stream.to_string(),
        data: data.to_string(),
    });
    while inner.log_bytes > LOG_CAP_BYTES {
        match inner.log.pop_front() {
            Some(front) => inner.log_bytes = inner.log_bytes.saturating_sub(front.data.len()),
            None => break,
        }
    }
    if let Some(file) = inner.log_file.as_mut() {
        let _ = file.write_all(data.as_bytes());
    }
    drop(inner);

    sink.emit_event(
        "ck:term",
        json!({ "id": session.id, "seq": seq, "stream": stream, "data": data }),
    );
}

async fn pump<S>(sink: Arc<dyn EventSink>, session: Arc<Session>, mut reader: S, stream: &'static str)
where
    S: AsyncRead + Unpin + Send + 'static,
{
    let mut decoder = Decoder::new();
    let mut buf = vec![0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let text = decoder.push(&buf[..n]);
                push_chunk(&sink, &session, &text, stream);
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------- 日志文件

fn open_log_file(root: &PathBuf, spec: &SpawnSpec, enabled: bool) -> Option<std::fs::File> {
    if !enabled {
        return None;
    }
    let dir = root.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.log", sanitize_id(&spec.id)));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let _ = writeln!(
        file,
        "\n===== {} 启动 · {} · cwd={} =====",
        store::fmt_local(store::now_ms()),
        spec.command,
        spec.workdir
    );
    Some(file)
}

// ---------------------------------------------------------------- 启动

pub async fn start(
    sink: Arc<dyn EventSink>,
    state: Arc<AppState>,
    spec: SpawnSpec,
) -> Result<(), String> {
    let persist = {
        let data = state.data.lock().unwrap();
        data.settings.persist_logs
    };
    let shell_setting = {
        let data = state.data.lock().unwrap();
        data.settings.shell.clone()
    };
    let shell = resolve_shell(&spec, &shell_setting);
    let mut cmd = build_command(&spec, &shell)?;

    let session = {
        let mut sessions = state.sessions.lock().unwrap();
        match sessions.get(&spec.id) {
            Some(existing) => {
                if existing.status().is_active() {
                    return Err(format!("{} 已在运行", spec.name));
                }
                *existing.spec.lock().unwrap() = spec.clone();
                existing.clone()
            }
            None => {
                let s = Arc::new(Session::new(spec.clone()));
                sessions.insert(spec.id.clone(), s.clone());
                s
            }
        }
    };

    {
        let mut inner = session.inner.lock().unwrap();
        inner.status = Status::Starting;
        inner.error = None;
        inner.exit_code = None;
        inner.stopping = false;
        inner.health_ok = false;
        inner.pid = None;
        inner.started_at = Some(store::now_ms());
        inner.started_instant = Some(Instant::now());
        inner.log_file = open_log_file(&state.root, &spec, persist);
    }
    emit_session(&sink, &session);

    push_chunk(
        &sink,
        &session,
        &format!("\u{1b}[90m# 独立会话 · cwd={} · env 由 Rust 注入\u{1b}[0m\n", spec.workdir),
        "out",
    );
    push_chunk(&sink, &session, &format!("\u{1b}[32m$\u{1b}[0m {}\n", spec.command), "out");

    // 端口预检：占用则给出明确提示（不阻止启动，交给用户判断）
    for port in &spec.ports {
        if procs::port_busy(*port) {
            let owner = procs::port_owner(*port)
                .await
                .unwrap_or_else(|| "占用进程未知".into());
            push_chunk(
                &sink,
                &session,
                &format!(
                    "\u{1b}[33m[preflight] 端口 {port} 已被占用（{owner}）\u{1b}[0m\n"
                ),
                "err",
            );
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            // Win10 常见：只有 Windows PowerShell，没有 pwsh 7 —— 自动回退一次
            let mut recovered = None;
            if cfg!(windows) && shell.to_ascii_lowercase().contains("pwsh") {
                if let Ok(mut alt) = build_command(&spec, "powershell.exe") {
                    if let Ok(c) = alt.spawn() {
                        push_chunk(
                            &sink,
                            &session,
                            "\u{1b}[33m# 未找到 pwsh.exe，已回退 windows powershell\u{1b}[0m\n",
                            "err",
                        );
                        recovered = Some(c);
                    }
                }
            }
            match recovered {
                Some(c) => c,
                None => {
                    let msg = format!("启动失败：{e}");
                    {
                        let mut inner = session.inner.lock().unwrap();
                        inner.status = Status::Failed;
                        inner.error = Some(msg.clone());
                        inner.log_file = None;
                    }
                    push_chunk(&sink, &session, &format!("\u{1b}[31m{msg}\u{1b}[0m\n"), "err");
                    emit_session(&sink, &session);
                    return Err(msg);
                }
            }
        }
    };

    let pid = child.id();
    {
        let mut inner = session.inner.lock().unwrap();
        inner.pid = pid;
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    if let Some(mut sin) = stdin {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        session.inner.lock().unwrap().stdin = Some(tx);
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if sin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                let _ = sin.flush().await;
            }
        });
    }

    if let Some(out) = stdout {
        tokio::spawn(pump(sink.clone(), session.clone(), out, "out"));
    }
    if let Some(err) = stderr {
        tokio::spawn(pump(sink.clone(), session.clone(), err, "err"));
    }

    {
        let sink2 = sink.clone();
        let state2 = state.clone();
        let s2 = session.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            finish(&sink2, &state2, &s2, status).await;
        });
    }

    match spec.health.clone() {
        Some(hc) => watch_health(sink.clone(), session.clone(), hc),
        None => {
            {
                let mut inner = session.inner.lock().unwrap();
                // 短命令可能在 waiter 里已经收敛为 Stopped，不能覆盖回 Running
                if matches!(inner.status, Status::Starting) {
                    inner.status = Status::Running;
                    inner.health_ok = true;
                }
            }
            emit_session(&sink, &session);
        }
    }

    Ok(())
}

fn watch_health(sink: Arc<dyn EventSink>, session: Arc<Session>, hc: HealthCheck) {
    tokio::spawn(async move {
        let deadline = Instant::now() + Duration::from_millis(hc.timeout_ms.max(1_000));
        let mut attempts = 0u32;
        loop {
            let status = session.status();
            if !matches!(status, Status::Starting | Status::Running) {
                return; // 进程已退出或已被停止
            }
            if check_health(&hc).await {
                {
                    let mut inner = session.inner.lock().unwrap();
                    if matches!(inner.status, Status::Starting) {
                        inner.status = Status::Running;
                    }
                    inner.health_ok = true;
                }
                emit_session(&sink, &session);
                sink.emit_event(
                    "ck:health",
                    json!({ "id": session.id, "ok": true, "attempts": attempts + 1 }),
                );
                return;
            }
            attempts += 1;
            let timed_out = Instant::now() >= deadline;
            let exhausted = attempts >= hc.retry.max(1);
            if timed_out || exhausted {
                let msg = format!("健康检查超时（{attempts} 次重试）");
                let pid = {
                    let mut inner = session.inner.lock().unwrap();
                    inner.status = Status::Failed;
                    inner.error = Some(msg.clone());
                    inner.pid
                };
                emit_session(&sink, &session);
                push_chunk(&sink, &session, &format!("\u{1b}[31m[health] {msg}\u{1b}[0m\n"), "err");
                if let Some(pid) = pid {
                    let _ = procs::terminate_tree(pid).await;
                }
                return;
            }
            sink.emit_event(
                "ck:health",
                json!({ "id": session.id, "ok": false, "attempts": attempts }),
            );
            tokio::time::sleep(Duration::from_millis(hc.interval_ms.max(200))).await;
        }
    });
}

pub async fn check_health(hc: &HealthCheck) -> bool {
    if let Some(url) = hc.url.as_ref().filter(|u| !u.trim().is_empty()) {
        return http_ok(url.trim()).await;
    }
    if let Some(port) = hc.port {
        return tcp_ok(port).await;
    }
    true
}

async fn tcp_ok(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(900),
            tokio::net::TcpStream::connect(("127.0.0.1", port))
        )
        .await,
        Ok(Ok(_))
    )
}

/// 极简 HTTP GET：只用于本机健康检查端点，避免为一个探测引入 HTTP 客户端依赖。
async fn http_ok(url: &str) -> bool {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().unwrap_or(80)),
        None => (authority, 80),
    };
    let addr = format!("{host}:{port}");
    let Ok(Ok(mut stream)) = tokio::time::timeout(
        Duration::from_millis(1200),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    else {
        return false;
    };
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if tokio::time::timeout(Duration::from_millis(1200), stream.write_all(req.as_bytes()))
        .await
        .is_err()
    {
        return false;
    }
    let mut buf = vec![0u8; 64];
    let n = match tokio::time::timeout(Duration::from_millis(1200), stream.read(&mut buf)).await {
        Ok(Ok(n)) => n,
        _ => return false,
    };
    let head = String::from_utf8_lossy(&buf[..n]);
    let Some(line) = head.lines().next() else {
        return false;
    };
    let mut parts = line.split_whitespace();
    let _proto = parts.next();
    let code: u16 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
    (200..400).contains(&code)
}

// ---------------------------------------------------------------- 退出处理

async fn finish(
    sink: &Arc<dyn EventSink>,
    state: &Arc<AppState>,
    session: &Arc<Session>,
    status: std::io::Result<std::process::ExitStatus>,
) {
    let _ = &state;
    let mut restart = false;
    let spec;
    {
        spec = session.spec.lock().unwrap().clone();
        let mut inner = session.inner.lock().unwrap();
        let was_stopping = inner.stopping;
        // 健康检查失败等「先置错」的场景，保留第一条错误原因，不被退出码覆盖
        let prior_error = inner.error.clone();
        inner.pid = None;
        inner.stdin = None;
        inner.health_ok = false;
        inner.log_file = None;
        inner.started_instant = None;
        match status {
            Ok(st) => {
                let code = st.code();
                inner.exit_code = code;
                if was_stopping {
                    inner.status = Status::Stopped;
                    inner.error = None;
                } else if st.success() {
                    inner.status = Status::Stopped;
                    inner.error = None;
                } else {
                    inner.status = Status::Failed;
                    inner.error = Some(prior_error.unwrap_or_else(|| {
                        format!(
                            "进程异常退出（{}）",
                            code.map(|c| format!("退出码 {c}"))
                                .unwrap_or_else(|| "被信号终止".into())
                        )
                    }));
                    if spec.policy.restart_on_crash && inner.restarts < spec.policy.max_restarts {
                        inner.restarts += 1;
                        restart = true;
                    }
                }
            }
            Err(e) => {
                inner.status = Status::Failed;
                inner.error = Some(prior_error.unwrap_or_else(|| format!("等待进程失败：{e}")));
            }
        }
        let code = inner.exit_code;
        drop(inner);
        push_chunk(
            sink,
            session,
            &format!(
                "\u{1b}[90m# 会话结束（{}）\u{1b}[0m\n",
                code.map(|c| format!("退出码 {c}"))
                    .unwrap_or_else(|| "无退出码".into())
            ),
            "out",
        );
    }
    emit_session(sink, session);

    if restart {
        push_chunk(
            sink,
            session,
            "\u{1b}[33m# 自动重启策略触发（1s 后重新拉起）\u{1b}[0m\n",
            "out",
        );
        session.inner.lock().unwrap().restart_pending = true;
    }
}

// ---------------------------------------------------------------- 停止 / 等待

pub async fn stop(
    sink: &Arc<dyn EventSink>,
    state: &Arc<AppState>,
    id: &str,
) -> Result<(), String> {
    let session = {
        let sessions = state.sessions.lock().unwrap();
        sessions.get(id).cloned()
    }
    .ok_or_else(|| format!("会话不存在：{id}"))?;

    let pid = {
        let mut inner = session.inner.lock().unwrap();
        if !matches!(
            inner.status,
            Status::Starting | Status::Running | Status::Restarting | Status::Stopping
        ) {
            inner.status = Status::Stopped;
            inner.stopping = true;
            None
        } else {
            inner.stopping = true;
            inner.status = Status::Stopping;
            inner.pid
        }
    };
    emit_session(sink, &session);

    if let Some(pid) = pid {
        if let Err(e) = procs::terminate_tree(pid).await {
            let mut inner = session.inner.lock().unwrap();
            inner.error = Some(e.clone());
            inner.status = Status::Failed;
            drop(inner);
            emit_session(sink, &session);
            return Err(e);
        }
    }
    Ok(())
}

async fn wait_until(session: &Arc<Session>, target: &[Status], timeout: Duration) -> bool {
    let begin = Instant::now();
    loop {
        let st = session.status();
        if target.contains(&st) {
            return true;
        }
        if begin.elapsed() > timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

async fn wait_ready(session: &Arc<Session>, timeout: Duration) -> Result<(), String> {
    let begin = Instant::now();
    loop {
        let (st, err, pid) = {
            let inner = session.inner.lock().unwrap();
            (inner.status, inner.error.clone(), inner.pid)
        };
        match st {
            Status::Running => return Ok(()),
            Status::Failed => {
                return Err(err.unwrap_or_else(|| "启动失败".into()));
            }
            Status::Stopped | Status::Exited => {
                // 没有健康检查的短命命令会很快退出：视为已成功跑完
                if pid.is_none() {
                    return Ok(());
                }
                return Err("进程已退出".into());
            }
            _ => {}
        }
        if begin.elapsed() > timeout {
            return Err("启动超时".into());
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

pub async fn restart(
    sink: &Arc<dyn EventSink>,
    state: &Arc<AppState>,
    id: &str,
) -> Result<(), String> {
    let spec = {
        let data = state.data.lock().unwrap();
        data.projects
            .iter()
            .find(|p| p.id == id)
            .map(spec_from_project)
    };
    let spec = match spec {
        Some(s) => s,
        None => {
            let sessions = state.sessions.lock().unwrap();
            sessions
                .get(id)
                .map(|s| s.spec.lock().unwrap().clone())
                .ok_or_else(|| format!("未知项目：{id}"))?
        }
    };
    if let Err(e) = stop(sink, state, id).await {
        eprintln!("[commandkit] restart 前停止失败：{e}");
    }
    let existing = { state.sessions.lock().unwrap().get(id).cloned() };
    if let Some(session) = existing {
        wait_until(
            &session,
            &[Status::Stopped, Status::Failed, Status::Exited],
            Duration::from_secs(10),
        )
        .await;
        session.inner.lock().unwrap().restarts = 0;
    }
    start(sink.clone(), state.clone(), spec).await
}

pub fn send_input(state: &Arc<AppState>, id: &str, data: &str) -> Result<(), String> {
    let session = {
        let sessions = state.sessions.lock().unwrap();
        sessions.get(id).cloned()
    }
    .ok_or_else(|| format!("会话不存在：{id}"))?;
    let tx = session.inner.lock().unwrap().stdin.clone();
    match tx {
        Some(tx) => tx
            .send(data.to_string())
            .map_err(|_| "输入通道已关闭".to_string()),
        None => Err("该会话没有可写 stdin".into()),
    }
}

// ---------------------------------------------------------------- 拓扑排序

pub fn topo_order(projects: &[Project], selected: &[String]) -> Result<Vec<String>, String> {
    let set: HashSet<&str> = selected.iter().map(|s| s.as_str()).collect();
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in selected {
        indeg.entry(id.as_str()).or_insert(0);
    }
    for p in projects.iter().filter(|p| set.contains(p.id.as_str())) {
        for dep in &p.depends_on {
            if set.contains(dep.as_str()) && dep != &p.id {
                adj.entry(dep.as_str()).or_default().push(p.id.as_str());
                *indeg.entry(p.id.as_str()).or_insert(0) += 1;
            }
        }
    }

    // 稳定序：同层按 selected 传入顺序
    let mut ready: VecDeque<&str> = selected
        .iter()
        .filter(|id| indeg.get(id.as_str()).copied().unwrap_or(0) == 0)
        .map(|s| s.as_str())
        .collect();
    let mut out = Vec::with_capacity(selected.len());
    while let Some(id) = ready.pop_front() {
        out.push(id.to_string());
        if let Some(children) = adj.get(id) {
            for child in children {
                if let Some(d) = indeg.get_mut(child) {
                    *d = d.saturating_sub(1);
                    if *d == 0 {
                        ready.push_back(child);
                    }
                }
            }
        }
    }
    if out.len() < indeg.len() {
        let stuck: Vec<String> = indeg
            .iter()
            .filter(|(_, d)| **d > 0)
            .map(|(k, _)| k.to_string())
            .collect();
        return Err(format!("依赖存在循环：{}", stuck.join(" / ")));
    }
    Ok(out)
}

// ---------------------------------------------------------------- 批量编排

fn publish_bulk(state: &Arc<AppState>, sink: &Arc<dyn EventSink>, mut b: BulkView) {
    let mut guard = state.bulk.lock().unwrap();
    if let Some(prev) = guard.as_ref() {
        if prev.cancelled {
            b.cancelled = true;
        }
    }
    *guard = Some(b.clone());
    drop(guard);
    sink.emit_event("ck:bulk", serde_json::to_value(&b).unwrap_or(serde_json::Value::Null));
}

pub fn cancel_bulk(state: &Arc<AppState>, sink: &Arc<dyn EventSink>) {
    let mut guard = state.bulk.lock().unwrap();
    if let Some(b) = guard.as_mut() {
        b.cancelled = true;
        let snapshot = b.clone();
        drop(guard);
        sink.emit_event("ck:bulk", serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null));
    }
}

fn bulk_cancelled(state: &Arc<AppState>) -> bool {
    state
        .bulk
        .lock()
        .unwrap()
        .as_ref()
        .map(|b| b.cancelled)
        .unwrap_or(false)
}

pub async fn start_many(
    sink: Arc<dyn EventSink>,
    state: Arc<AppState>,
    ids: Vec<String>,
    kind: &str,
) -> Result<(), String> {
    let (order, specs) = {
        let data = state.data.lock().unwrap();
        let order = topo_order(&data.projects, &ids)?;
        let specs: HashMap<String, SpawnSpec> = data
            .projects
            .iter()
            .filter(|p| ids.iter().any(|i| i == &p.id))
            .map(|p| (p.id.clone(), spec_from_project(p)))
            .collect();
        (order, specs)
    };
    if order.is_empty() {
        return Err("没有可启动的项目".into());
    }

    let mut status: BTreeMap<String, String> = BTreeMap::new();
    for id in &order {
        status.insert(id.clone(), "waiting".into());
    }
    let mut bulk = BulkView {
        kind: kind.to_string(),
        total: order.len(),
        index: 0,
        order: order.clone(),
        status,
        cancelled: false,
        finished: false,
        message: None,
        started_at: store::now_ms(),
    };
    publish_bulk(&state, &sink, bulk.clone());

    for (i, id) in order.iter().enumerate() {
        if bulk_cancelled(&state) {
            break;
        }
        bulk.index = i;
        bulk.status.insert(id.clone(), "running".into());
        bulk.message = None;
        publish_bulk(&state, &sink, bulk.clone());

        let Some(spec) = specs.get(id).cloned() else {
            bulk.status.insert(id.clone(), "failed".into());
            continue;
        };
        let name = spec.name.clone();
        let fail_fast = spec.policy.fail_fast;
        let started = start(sink.clone(), state.clone(), spec.clone()).await;
        if let Err(e) = started {
            bulk.status.insert(id.clone(), "failed".into());
            bulk.message = Some(format!("{name}：{e}"));
            publish_bulk(&state, &sink, bulk.clone());
            if fail_fast {
                break;
            }
            continue;
        }
        let ready_timeout = spec
            .health
            .as_ref()
            .map(|h| Duration::from_millis(h.timeout_ms + 5_000))
            .unwrap_or(Duration::from_secs(12));
        let session = { state.sessions.lock().unwrap().get(id).cloned() };
        let Some(session) = session else { continue };
        match wait_ready(&session, ready_timeout).await {
            Ok(()) => {
                bulk.status.insert(id.clone(), "done".into());
            }
            Err(e) => {
                bulk.status.insert(id.clone(), "failed".into());
                bulk.message = Some(format!("{name}：{e}"));
                publish_bulk(&state, &sink, bulk.clone());
                if fail_fast {
                    break;
                }
            }
        }
        publish_bulk(&state, &sink, bulk.clone());
    }

    bulk.finished = true;
    if bulk.cancelled {
        bulk.message = Some("批量启动已中断".into());
        for (k, v) in bulk.status.clone() {
            if v == "waiting" {
                bulk.status.insert(k, "skipped".into());
            }
        }
    }
    publish_bulk(&state, &sink, bulk.clone());
    Ok(())
}

pub async fn stop_many(
    sink: Arc<dyn EventSink>,
    state: Arc<AppState>,
    ids: Vec<String>,
    kind: &str,
) -> Result<(), String> {
    let mut order = {
        let data = state.data.lock().unwrap();
        topo_order(&data.projects, &ids).unwrap_or_else(|_| ids.clone())
    };
    order.reverse(); // 停止按依赖逆序

    let mut status: BTreeMap<String, String> = BTreeMap::new();
    for id in &order {
        status.insert(id.clone(), "waiting".into());
    }
    let mut bulk = BulkView {
        kind: kind.to_string(),
        total: order.len(),
        index: 0,
        order: order.clone(),
        status,
        cancelled: false,
        finished: false,
        message: None,
        started_at: store::now_ms(),
    };
    publish_bulk(&state, &sink, bulk.clone());

    for (i, id) in order.iter().enumerate() {
        bulk.index = i;
        bulk.status.insert(id.clone(), "running".into());
        publish_bulk(&state, &sink, bulk.clone());
        let result = stop(&sink, &state, id).await;
        let stopped = { state.sessions.lock().unwrap().get(id).cloned() };
        if let Some(session) = stopped {
            wait_until(
                &session,
                &[Status::Stopped, Status::Failed, Status::Exited],
                Duration::from_secs(6),
            )
            .await;
        }
        bulk.status.insert(
            id.clone(),
            if result.is_ok() { "done" } else { "failed" }.to_string(),
        );
        if let Err(e) = result {
            bulk.message = Some(e);
        }
        publish_bulk(&state, &sink, bulk.clone());
    }
    bulk.finished = true;
    publish_bulk(&state, &sink, bulk.clone());
    Ok(())
}

// ---------------------------------------------------------------- 辅助

pub fn sessions_snapshot(state: &Arc<AppState>) -> Vec<SessionView> {
    let sessions = state.sessions.lock().unwrap();
    sessions.values().map(|s| s.view()).collect()
}

pub fn get_session(state: &Arc<AppState>, id: &str) -> Option<Arc<Session>> {
    state.sessions.lock().unwrap().get(id).cloned()
}

pub fn log_snapshot(state: &Arc<AppState>, id: &str) -> Vec<LogChunk> {
    match get_session(state, id) {
        Some(s) => s.inner.lock().unwrap().log.iter().cloned().collect(),
        None => vec![],
    }
}

pub fn clear_log(state: &Arc<AppState>, id: &str) {
    if let Some(s) = get_session(state, id) {
        let mut inner = s.inner.lock().unwrap();
        inner.log.clear();
        inner.log_bytes = 0;
    }
}
