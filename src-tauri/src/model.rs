//! 数据模型：项目（组件分组）/ 组件（启动单元）/ 模板 / 设置 / 运行时状态。
//!
//! 所有结构体统一 `camelCase` 序列化，前端 TypeScript 类型与之一一对应。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_project_id() -> String {
    "all".to_string()
}

/// 命令步骤之间的连接方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    /// 且：上一条成功后继续；上一条失败即按失败策略处理
    And,
    /// 或：上一条失败时才跑这一条（备选），上一条成功则跳过
    Or,
}

impl Default for Join {
    fn default() -> Self {
        Join::And
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStep {
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub join: Join,
}

impl CommandStep {
    pub fn new(cmd: &str, join: Join) -> Self {
        Self {
            cmd: cmd.to_string(),
            join,
        }
    }
}

/// 组件：真正被启动的单元（旧版本里叫「项目」）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: String,
    pub name: String,
    /// 所属项目 id（旧版本字段名 category）
    #[serde(default = "default_project_id")]
    pub project_id: String,
    /// 卡片右侧标签，例如 "npm dev" / "Python 3.11"
    #[serde(default)]
    pub stack: String,
    #[serde(default)]
    pub workdir: String,
    /// 启动命令链：按顺序执行，步骤之间用「且 / 或」连接
    #[serde(default)]
    pub commands: Vec<CommandStep>,
    /// 仅注入到本组件的子进程，不写系统环境变量
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub ports: Vec<u16>,
    /// 依赖的其它组件 id（拓扑排序用）
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub health: Option<HealthCheck>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub auto_start: bool,
    /// 覆盖默认 shell
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    /// 形如 http://127.0.0.1:8000/health；与 port 二选一或同时给出
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default = "default_health_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_health_retry")]
    pub retry: u32,
    #[serde(default = "default_health_interval")]
    pub interval_ms: u64,
}

fn default_health_timeout() -> u64 {
    60_000
}
fn default_health_retry() -> u32 {
    30
}
fn default_health_interval() -> u64 {
    1_000
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            url: None,
            port: None,
            timeout_ms: default_health_timeout(),
            retry: default_health_retry(),
            interval_ms: default_health_interval(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    /// true：批量启动时前序失败即中止；false：跳过并继续
    #[serde(default = "default_true")]
    pub fail_fast: bool,
    #[serde(default)]
    pub restart_on_crash: bool,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
}

fn default_true() -> bool {
    true
}
fn default_max_restarts() -> u32 {
    2
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            fail_fast: true,
            restart_on_crash: false,
            max_restarts: default_max_restarts(),
        }
    }
}

/// 项目：组件的分组容器（旧版本里叫「分类」）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateCommand {
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub workdir: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub commands: Vec<TemplateCommand>,
    #[serde(default)]
    pub updated_at: u64,
}

fn default_platform() -> String {
    if cfg!(target_os = "windows") {
        "windows".into()
    } else {
        "macos".into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_shell")]
    pub shell: String,
    #[serde(default = "default_log_cap")]
    pub log_lines_cap: usize,
    #[serde(default = "default_true")]
    pub confirm_bulk: bool,
    #[serde(default = "default_refresh")]
    pub refresh_ms: u64,
    #[serde(default = "default_true")]
    pub persist_logs: bool,
    #[serde(default)]
    pub sidebar_collapsed: bool,
}

fn default_shell() -> String {
    if cfg!(target_os = "windows") {
        "pwsh.exe".into()
    } else {
        "/bin/zsh".into()
    }
}
fn default_log_cap() -> usize {
    3_000
}
fn default_refresh() -> u64 {
    2_000
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shell: default_shell(),
            log_lines_cap: default_log_cap(),
            confirm_bulk: true,
            refresh_ms: default_refresh(),
            persist_logs: true,
            sidebar_collapsed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Stopped,
    Starting,
    Running,
    Restarting,
    Stopping,
    Exited,
    Failed,
}

impl Status {
    pub fn is_active(self) -> bool {
        matches!(self, Status::Starting | Status::Running | Status::Restarting)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogChunk {
    pub seq: u64,
    /// "out" | "err"
    pub stream: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub restarts: u32,
    pub health_ok: bool,
    pub command: String,
    pub workdir: String,
    pub ports: Vec<u16>,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRow {
    pub id: String,
    pub pid: Option<u32>,
    pub cpu: f32,
    pub mem: u64,
    pub children: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcNode {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub cmd: String,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDetail {
    pub id: String,
    pub tree: Vec<ProcNode>,
    pub env: Vec<(String, String)>,
    pub listen: Vec<String>,
    pub log_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkView {
    pub kind: String,
    pub total: usize,
    pub index: usize,
    pub order: Vec<String>,
    pub status: BTreeMap<String, String>,
    pub cancelled: bool,
    pub finished: bool,
    pub message: Option<String>,
    pub started_at: u64,
}

/// 可选 shell（设置 / 项目编辑器共用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellOption {
    pub value: String,
    pub label: String,
    /// "pwsh" | "powershell" | "cmd" | "posix"
    pub kind: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub platform: String,
    pub version: String,
    pub data_root: String,
    pub writable: bool,
    pub portable: bool,
    pub shell: String,
    pub shells: Vec<ShellOption>,
    pub webview: String,
}
