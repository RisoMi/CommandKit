//! 进程监管核心：独立子 shell 启动、输出流转发、健康检查、进程树终止、批量编排。
//!
//! 设计要点：
//! - 每个项目 / 模板一个独立子 shell（`zsh -lc` / `pwsh -NoProfile -Command`），
//!   env 由 Rust 侧构造后随 spawn 注入，从不调用 setx / 注册表 / shell profile 去改系统环境。
//! - 终止统一走进程树：Windows `taskkill /T /F`，macOS `kill(-pgid)`。
//! - 单条操作只影响自己的会话，批量操作按 dependsOn 拓扑序编排。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use tokio::process::{Child, Command};
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
    /// 所属项目 id
    pub project_id: String,
    /// 标准输入策略
    pub stdin_mode: String,
    /// 终端模式："pipe"（管道，行渲染）| "pty"（伪终端，可跑 TUI）
    pub term_mode: String,
    /// 命令链（按顺序执行，join 决定衔接方式）
    pub steps: Vec<CommandStep>,
    /// 展示用：把命令链拼成一行
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
    pub ended_at: Option<u64>,
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
    /// PTY 会话的原始写入通道（每个按键都要转发，不能按行）
    pub pty_stdin: Option<std::sync::mpsc::Sender<String>>,
    /// 保留 master 句柄用于 resize
    pub pty_master: Option<Box<dyn MasterPty + Send>>,
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
            ended_at: None,
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
            pty_stdin: None,
            pty_master: None,
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
            ended_at: inner.ended_at,
            exit_code: inner.exit_code,
            error: inner.error.clone(),
            restarts: inner.restarts,
            health_ok: inner.health_ok,
            command: spec.command,
            workdir: spec.workdir,
            ports: spec.ports,
            project_id: spec.project_id,
            term_mode: spec.term_mode.clone(),
        }
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status
    }
}

/// 失败原因里能识别的常见坑，返回一句可操作的提示
pub fn failure_hint(tail: &str) -> Option<&'static str> {
    let t = tail.to_ascii_lowercase();
    if t.contains("no stdin data received")
        || t.contains("input must be provided")
        || t.contains("not a tty")
        || t.contains("raw mode is not supported")
        || t.contains("inappropriate ioctl")
    {
        return Some(
            "# 提示：这行命令在等交互式输入，而 CommandKit 的会话是管道（不是 TTY）。\n\
#   · 非交互用法：参数写全，例如 claude -p \"你的问题\"\n\
#   · 需要真终端 / TUI：点下方「打开外部终端」，或把组件的「标准输入」设为「关闭（EOF）」",
        );
    }
    if t.contains("--print") {
        return Some(
            "# 提示：--print 模式必须带提示词（如 claude -p \"你的问题\"），否则它会一直等 stdin",
        );
    }
    None
}

fn emit_session(sink: &Arc<dyn EventSink>, s: &Session) {
    sink.emit_event(
        "ck:session",
        serde_json::to_value(s.view()).unwrap_or(serde_json::Value::Null),
    );
}

// ---------------------------------------------------------------- spec 构造

fn join_label(join: Join) -> &'static str {
    match join {
        Join::And => "且",
        Join::Or => "或",
    }
}

/// 命令链的可读形式：A 且 B 或 C
pub fn display_chain(steps: &[CommandStep]) -> String {
    steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if i == 0 {
                s.cmd.clone()
            } else {
                format!(" {} {}", join_label(s.join), s.cmd)
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// 合并环境：环境提供基础变量，组件自身的 env 覆盖同名项
pub fn merged_env(c: &Component, envs: &[Environment]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(id) = c.env_profile.as_deref() {
        if let Some(profile) = envs.iter().find(|e| e.id == id) {
            for (k, v) in &profile.env {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    for (k, v) in &c.env {
        out.insert(k.clone(), v.clone());
    }
    out
}

pub fn spec_from_component(c: &Component, envs: &[Environment]) -> SpawnSpec {
    SpawnSpec {
        id: c.id.clone(),
        name: c.name.clone(),
        kind: "component".into(),
        project_id: c.project_id.clone(),
        stdin_mode: if c.stdin_mode.is_empty() {
            "pipe".into()
        } else {
            c.stdin_mode.clone()
        },
        term_mode: if c.term_mode.is_empty() {
            "pipe".into()
        } else {
            c.term_mode.clone()
        },
        command: display_chain(&c.commands),
        steps: c.commands.clone(),
        workdir: c.workdir.clone(),
        env: merged_env(c, envs),
        ports: c.ports.clone(),
        health: c.health.clone(),
        policy: c.policy.clone(),
        shell: c.shell.clone(),
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
    let steps: Vec<CommandStep> = t
        .commands
        .iter()
        .filter(|c| !c.cmd.trim().is_empty())
        .map(|c| CommandStep::new(&c.cmd, Join::And))
        .collect();
    SpawnSpec {
        id: format!("tpl:{}", t.id),
        name: format!("模板 · {}", t.name),
        kind: "template".into(),
        project_id: "template".into(),
        stdin_mode: "pipe".into(),
        term_mode: "pipe".into(),
        steps,
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

/// shell 家族：决定命令行参数，而不是靠平台硬编码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Pwsh,
    PowerShell,
    Cmd,
    Posix,
}

impl ShellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "pwsh",
            ShellKind::PowerShell => "powershell",
            ShellKind::Cmd => "cmd",
            ShellKind::Posix => "posix",
        }
    }
}

pub fn shell_kind(shell: &str) -> ShellKind {
    let base = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let base = base.trim_end_matches(".exe");
    if base.contains("pwsh") {
        ShellKind::Pwsh
    } else if base.contains("powershell") {
        ShellKind::PowerShell
    } else if base == "cmd" {
        ShellKind::Cmd
    } else {
        ShellKind::Posix
    }
}

/// 按 shell 家族给出调用参数（抽出来便于单测，也便于 UI 展示「实际执行形式」）
pub fn shell_invocation(shell: &str, command: &str) -> Vec<String> {
    match shell_kind(shell) {
        // /D 关闭 cmd 的 AutoRun，避免注册表里的启动脚本污染环境
        ShellKind::Cmd => vec!["/D".into(), "/C".into(), command.into()],
        ShellKind::Pwsh | ShellKind::PowerShell => {
            vec!["-NoProfile".into(), "-Command".into(), command.into()]
        }
        // -l：登录 shell，确保 Finder 启动的应用也能拿到用户 PATH
        ShellKind::Posix => vec!["-lc".into(), command.into()],
    }
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

fn build_command(spec: &SpawnSpec, shell: &str, command: &str) -> Result<Command, String> {
    if command.trim().is_empty() {
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
    for arg in shell_invocation(shell, command) {
        cmd.arg(arg);
    }
    cmd.current_dir(&workdir);
    // null：立刻 EOF（CLI 不会等 3s 数据）；pipe：可在终端里手动输入
    let stdin = if spec.stdin_mode == "null" {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    cmd.stdin(stdin)
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
        inner.ended_at = None;
        inner.started_instant = Some(Instant::now());
        inner.log_file = open_log_file(&state.root, &spec, persist);
    }
    emit_session(&sink, &session);

    push_chunk(
        &sink,
        &session,
        &format!(
            "\u{1b}[90m# 独立会话 · shell={} · cwd={} · env 由 Rust 注入\u{1b}[0m\n",
            shell, spec.workdir
        ),
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

    // 先做一次同步校验：工作目录 / 命令为空这类错误直接回给调用方
    build_command(&spec, &shell, &spec.steps[0].cmd)?;

    let runner_sink = sink.clone();
    let runner_state = state.clone();
    let runner_session = session.clone();
    let runner_spec = spec.clone();
    tokio::spawn(async move {
        run_chain(runner_sink, runner_state, runner_session, runner_spec, shell).await;
    });

    Ok(())
}

// ---------------------------------------------------------------- 命令链

/// 成功后下一步：跳过紧随其后的「或」备选
fn next_after_success(steps: &[CommandStep], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < steps.len() && steps[j].join == Join::Or {
        j += 1;
    }
    if j < steps.len() {
        Some(j)
    } else {
        None
    }
}

/// 失败后下一步：只有「或」备选才继续
fn next_after_failure(steps: &[CommandStep], i: usize) -> Option<usize> {
    match steps.get(i + 1) {
        Some(s) if s.join == Join::Or => Some(i + 1),
        _ => None,
    }
}

/// 最后一条「主线」命令（后面没有「且」步骤）：健康检查在这条上挂
fn is_terminal_step(steps: &[CommandStep], i: usize) -> bool {
    next_after_success(steps, i).is_none()
}

/// 一步的实际子进程：管道 or 伪终端
enum StepWaiter {
    Plain(Child),
    Pty(Box<dyn portable_pty::Child + Send + Sync>),
}

/// 一步的退出结果（管道 / PTY 统一形状）
#[derive(Debug, Clone)]
pub struct Outcome {
    pub code: Option<i32>,
    pub success: bool,
    pub error: Option<String>,
}

impl StepWaiter {
    fn pid(&self) -> Option<u32> {
        match self {
            StepWaiter::Plain(c) => c.id(),
            StepWaiter::Pty(c) => c.process_id(),
        }
    }

    async fn wait(self) -> Outcome {
        match self {
            StepWaiter::Plain(mut c) => match c.wait().await {
                Ok(st) => Outcome {
                    code: st.code(),
                    success: st.success(),
                    error: None,
                },
                Err(e) => Outcome {
                    code: None,
                    success: false,
                    error: Some(e.to_string()),
                },
            },
            StepWaiter::Pty(mut c) => {
                // 不用 spawn_blocking：万一进程不退出，阻塞线程会一直挂着
                // （测试运行时会等在阻塞任务上永久卡死）。这里异步轮询。
                loop {
                    match c.try_wait() {
                        Ok(Some(st)) => {
                            return Outcome {
                                code: Some(st.exit_code() as i32),
                                success: st.success(),
                                error: None,
                            }
                        }
                        Ok(None) => tokio::time::sleep(Duration::from_millis(80)).await,
                        Err(e) => {
                            return Outcome {
                                code: None,
                                success: false,
                                error: Some(e.to_string()),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn pty_builder(spec: &SpawnSpec, shell: &str, command: &str) -> Result<CommandBuilder, String> {
    // 复用管道模式的前置校验（命令为空 / 工作目录不存在）
    build_command(spec, shell, command)?;
    let mut cmd = CommandBuilder::new(shell);
    for arg in shell_invocation(shell, command) {
        cmd.arg(arg);
    }
    cmd.cwd(spec.workdir.trim());
    for (k, v) in &spec.env {
        if k.trim().is_empty() {
            continue;
        }
        cmd.env(normalize_key(k.trim()), expand(v, &spec.env));
    }
    Ok(cmd)
}

/// 打开伪终端并启动 shell；失败时按需回退 powershell.exe
fn spawn_pty(
    sink: &Arc<dyn EventSink>,
    session: &Arc<Session>,
    spec: &SpawnSpec,
    shell: &str,
    command: &str,
) -> Result<(StepWaiter, Option<Box<dyn MasterPty + Send>>), String> {
    let attempt = |sh: &str| -> Result<(StepWaiter, Box<dyn MasterPty + Send>), String> {
        let pty_cmd = pty_builder(spec, sh, command)?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("创建伪终端失败：{e}"))?;
        let child = pair
            .slave
            .spawn_command(pty_cmd)
            .map_err(|e| format!("启动失败：{e}"))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("读取伪终端失败：{e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("写入伪终端失败：{e}"))?;
        let master = pair.master;

        // 读线程：PTY 把 stdout/stderr 合并成一路，全部按 out 处理
        {
            let sink2 = sink.clone();
            let session2 = session.clone();
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut decoder = Decoder::new();
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let text = decoder.push(&buf[..n]);
                            if !text.is_empty() {
                                push_chunk(&sink2, &session2, &text, "out");
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // 写线程：终端里每敲一个键都直接写进 pty（TUI 需要逐字节）
        {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            session.inner.lock().unwrap().pty_stdin = Some(tx);
            std::thread::spawn(move || {
                let mut writer = writer;
                while let Ok(data) = rx.recv() {
                    if writer.write_all(data.as_bytes()).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        }

        Ok((StepWaiter::Pty(child), master))
    };

    match attempt(shell) {
        Ok((waiter, master)) => Ok((waiter, Some(master))),
        Err(e) => {
            if cfg!(windows) && matches!(shell_kind(shell), ShellKind::Pwsh) {
                if let Ok((waiter, master)) = attempt("powershell.exe") {
                    push_chunk(
                        sink,
                        session,
                        "\u{1b}[33m# 未找到 pwsh.exe，已回退 windows powershell\u{1b}[0m\n",
                        "err",
                    );
                    return Ok((waiter, Some(master)));
                }
            }
            Err(e)
        }
    }
}

fn spawn_step(
    sink: &Arc<dyn EventSink>,
    session: &Arc<Session>,
    spec: &SpawnSpec,
    shell: &str,
    command: &str,
) -> Result<(StepWaiter, Option<Box<dyn MasterPty + Send>>), String> {
    if spec.term_mode == "pty" {
        return spawn_pty(sink, session, spec, shell, command);
    }

    let mut cmd = build_command(spec, shell, command)?;
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            // Win10 常见：只有 Windows PowerShell，没有 pwsh 7 —— 自动回退一次
            if cfg!(windows) && matches!(shell_kind(shell), ShellKind::Pwsh) {
                if let Ok(mut alt) = build_command(spec, "powershell.exe", command) {
                    if let Ok(c) = alt.spawn() {
                        push_chunk(
                            sink,
                            session,
                            "\u{1b}[33m# 未找到 pwsh.exe，已回退 windows powershell\u{1b}[0m\n",
                            "err",
                        );
                        return Ok((StepWaiter::Plain(c), None));
                    }
                }
            }
            return Err(format!("启动失败：{e}"));
        }
    };

    if let Some(mut sin) = child.stdin.take() {
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
    if let Some(out) = child.stdout.take() {
        tokio::spawn(pump(sink.clone(), session.clone(), out, "out"));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(pump(sink.clone(), session.clone(), err, "err"));
    }

    Ok((StepWaiter::Plain(child), None))
}

/// 逐条执行命令链，全部结束后统一收敛会话状态
async fn run_chain(
    sink: Arc<dyn EventSink>,
    state: Arc<AppState>,
    session: Arc<Session>,
    spec: SpawnSpec,
    shell: String,
) {
    let steps = spec.steps.clone();
    let mut idx = 0usize;
    let mut outcome = Outcome {
        code: None,
        success: false,
        error: Some("命令链没有执行".into()),
    };

    loop {
        if session.inner.lock().unwrap().stopping {
            break;
        }
        let step = steps[idx].clone();

        if steps.len() > 1 {
            push_chunk(
                &sink,
                &session,
                &format!(
                    "\u{1b}[90m# 步骤 {}/{} · {}\u{1b}[0m\n",
                    idx + 1,
                    steps.len(),
                    step.cmd
                ),
                "out",
            );
        } else if steps.len() > 1 {
            push_chunk(
                &sink,
                &session,
                &format!("\u{1b}[32m$\u{1b}[0m {}\n", step.cmd),
                "out",
            );
        }
        // 单步命令的 "$ cmd" 已由 start() 的横幅打印，避免重复

        let (waiter, master) = match spawn_step(&sink, &session, &spec, &shell, &step.cmd) {
            Ok(v) => v,
            Err(e) => {
                {
                    let mut inner = session.inner.lock().unwrap();
                    inner.status = Status::Failed;
                    inner.error = Some(e.clone());
                    inner.pid = None;
                    inner.log_file = None;
                }
                push_chunk(&sink, &session, &format!("\u{1b}[31m{e}\u{1b}[0m\n"), "err");
                emit_session(&sink, &session);
                return;
            }
        };

        let pid = waiter.pid();
        let spawn_ms = {
            let mut inner = session.inner.lock().unwrap();
            inner.pid = pid;
            inner.pty_master = master;
            inner
                .started_instant
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(0)
        };
        if spawn_ms >= 500 {
            push_chunk(
                &sink,
                &session,
                &format!(
                    "\u{1b}[33m# 启动较慢：距点击已 {}ms（shell 冷启动 / 端口预检 / 目录在慢速盘上）\u{1b}[0m\n",
                    spawn_ms
                ),
                "err",
            );
        }

        // 15s 一条输出都没有 → 给出可操作的提示，避免用户以为「卡死没反应」
        {
            let seq_at_spawn = session.inner.lock().unwrap().seq;
            let hint_sink = sink.clone();
            let hint_session = session.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let (seq_now, status, stopping) = {
                    let i = hint_session.inner.lock().unwrap();
                    (i.seq, i.status, i.stopping)
                };
                if stopping || seq_now != seq_at_spawn {
                    return;
                }
                if !matches!(status, Status::Starting | Status::Running) {
                    return;
                }
                push_chunk(
                    &hint_sink,
                    &hint_session,
                    "\u{1b}[33m# 已等待 15s 仍无任何输出：命令可能在等输入、访问网络盘，或 shell 启动很慢。\
可点「打开外部终端」用同一命令对照，或换个 shell 试试\u{1b}[0m\n",
                    "err",
                );
            });
        }

        // 只有最后一条主线命令才挂健康检查 / 置为运行中
        if is_terminal_step(&steps, idx) {
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
        }

        outcome = waiter.wait().await;
        let ok = outcome.success;

        {
            let mut inner = session.inner.lock().unwrap();
            inner.pid = None;
            inner.stdin = None;
            inner.pty_stdin = None;
            inner.pty_master = None;
        }

        if steps.len() > 1 {
            push_chunk(
                &sink,
                &session,
                &format!(
                    "\u{1b}[90m# 步骤 {}/{} 结束（{}）\u{1b}[0m\n",
                    idx + 1,
                    steps.len(),
                    outcome
                        .code
                        .map(|c| format!("退出码 {c}"))
                        .unwrap_or_else(|| "无退出码".into())
                ),
                "out",
            );
        }

        if session.inner.lock().unwrap().stopping {
            break;
        }
        match if ok {
            next_after_success(&steps, idx)
        } else {
            next_after_failure(&steps, idx)
        } {
            Some(n) => idx = n,
            None => break,
        }
    }

    finish(&sink, &state, &session, outcome).await;
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
    outcome: Outcome,
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
        inner.pty_stdin = None;
        inner.pty_master = None;
        inner.health_ok = false;
        inner.log_file = None;
        inner.started_instant = None;
        // 冻结运行时长：结束后不再随时钟增长
        inner.ended_at = Some(store::now_ms());

        inner.exit_code = outcome.code;
        if let Some(e) = outcome.error.clone() {
            inner.status = Status::Failed;
            inner.error = Some(prior_error.unwrap_or_else(|| format!("等待进程失败：{e}")));
        } else if was_stopping || outcome.success {
            inner.status = Status::Stopped;
            inner.error = None;
        } else {
            inner.status = Status::Failed;
            inner.error = Some(prior_error.unwrap_or_else(|| {
                format!(
                    "进程异常退出（{}）",
                    outcome
                        .code
                        .map(|c| format!("退出码 {c}"))
                        .unwrap_or_else(|| "被信号终止".into())
                )
            }));
            if spec.policy.restart_on_crash && inner.restarts < spec.policy.max_restarts {
                inner.restarts += 1;
                restart = true;
            }
        }
        let code = inner.exit_code;
        let bad = matches!(inner.status, Status::Failed)
            || code.map(|c| c != 0).unwrap_or(false);
        // 取日志尾部用于识别「需要 TTY / 缺提示词」这类失败
        let tail = if bad {
            let mut acc = String::new();
            for chunk in inner.log.iter().rev() {
                acc.insert_str(0, &chunk.data);
                if acc.len() > 2000 {
                    break;
                }
            }
            acc
        } else {
            String::new()
        };
        drop(inner);
        if let Some(hint) = failure_hint(&tail) {
            push_chunk(sink, session, &format!("\u{1b}[33m{hint}\u{1b}[0m\n"), "err");
        }
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
        // taskkill / kill(-pgid) 自己也可能卡住（进程卡在内核态、句柄不可中断），必须加超时
        match tokio::time::timeout(Duration::from_secs(4), procs::terminate_tree(pid)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let mut inner = session.inner.lock().unwrap();
                inner.error = Some(e.clone());
                inner.status = Status::Failed;
                drop(inner);
                emit_session(sink, &session);
                return Err(e);
            }
            Err(_) => {
                push_chunk(
                    sink,
                    &session,
                    "\u{1b}[33m# 终止命令超时（4s），继续等待进程退出…\u{1b}[0m\n",
                    "err",
                );
            }
        }
    }

    // 看门狗：任何情况下都不能让界面永远停在「停止中」
    {
        let sink2 = sink.clone();
        let session2 = session.clone();
        tokio::spawn(async move {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if session2.status() != Status::Stopping {
                    return;
                }
                if Instant::now() < deadline {
                    continue;
                }
                // 最后一枪，然后强制收敛状态
                let pid = session2.inner.lock().unwrap().pid;
                if let Some(pid) = pid {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        procs::terminate_tree(pid),
                    )
                    .await;
                }
                {
                    let mut inner = session2.inner.lock().unwrap();
                    if inner.status != Status::Stopping {
                        return;
                    }
                    inner.status = Status::Stopped;
                    inner.stopping = true;
                    inner.pid = None;
                    inner.stdin = None;
                    inner.health_ok = false;
                    inner.ended_at = Some(store::now_ms());
                    inner.error = Some("进程未响应终止信号，已强制标记为已停止（可能仍在后台存活）".into());
                }
                emit_session(&sink2, &session2);
                push_chunk(
                    &sink2,
                    &session2,
                    "\u{1b}[33m# 进程未响应终止，已强制标记为已停止；如仍占用端口请手动结束\u{1b}[0m\n",
                    "err",
                );
                return;
            }
        });
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
        data.components
            .iter()
            .find(|c| c.id == id)
            .map(|c| spec_from_component(c, &data.environments))
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
    let (pty_tx, line_tx, active) = {
        let inner = session.inner.lock().unwrap();
        (
            inner.pty_stdin.clone(),
            inner.stdin.clone(),
            matches!(
                inner.status,
                Status::Starting | Status::Running | Status::Restarting
            ),
        )
    };
    // 已停止 / 已结束的会话：xterm 仍可能为 ESC[6n 之类自动应答回写，直接丢掉，
    // 不要报错（否则界面会被「没有可写 stdin」刷屏）
    if !active {
        return Ok(());
    }
    // PTY 会话必须逐字节转发（TUI 的按键、方向键、Ctrl 组合）
    if let Some(tx) = pty_tx {
        return tx
            .send(data.to_string())
            .map_err(|_| "伪终端输入通道已关闭".to_string());
    }
    match line_tx {
        Some(tx) => tx
            .send(data.to_string())
            .map_err(|_| "输入通道已关闭".to_string()),
        None => Err("该会话没有可写 stdin".into()),
    }
}

/// 调整伪终端尺寸（TUI 需要正确的行列数）
pub fn resize_pty(state: &Arc<AppState>, id: &str, cols: u16, rows: u16) -> Result<(), String> {
    let session = {
        let sessions = state.sessions.lock().unwrap();
        sessions.get(id).cloned()
    }
    .ok_or_else(|| format!("会话不存在：{id}"))?;
    let size = PtySize {
        rows: rows.max(5),
        cols: cols.max(20),
        pixel_width: 0,
        pixel_height: 0,
    };
    let inner = session.inner.lock().unwrap();
    match inner.pty_master.as_ref() {
        Some(m) => m.resize(size).map_err(|e| format!("调整窗口失败：{e}")),
        None => Err("该会话不是伪终端".into()),
    }
}

// ---------------------------------------------------------------- 拓扑排序

pub fn topo_order(components: &[Component], selected: &[String]) -> Result<Vec<String>, String> {
    let set: HashSet<&str> = selected.iter().map(|s| s.as_str()).collect();
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in selected {
        indeg.entry(id.as_str()).or_insert(0);
    }
    for p in components.iter().filter(|p| set.contains(p.id.as_str())) {
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
        let order = topo_order(&data.components, &ids)?;
        let specs: HashMap<String, SpawnSpec> = data
            .components
            .iter()
            .filter(|c| ids.iter().any(|i| i == &c.id))
            .map(|c| (c.id.clone(), spec_from_component(c, &data.environments)))
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
        {
            // 模板会话（tpl:*）不在组件表里，拓扑排序会丢掉它们 —— 这里补回原顺序
            let mut order = topo_order(&data.components, &ids).unwrap_or_else(|_| ids.clone());
            for id in &ids {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
            order
        }
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
