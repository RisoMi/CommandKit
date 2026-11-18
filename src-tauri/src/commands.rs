//! 前端 IPC 命令：全部业务能力收敛在 Rust 侧，前端不直接 spawn 进程、不直接写文件。

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::model::*;
use crate::proc as procs;
use crate::store;
use crate::supervisor::{self, EventSink, SpawnSpec, TauriSink};

/// 把 AppHandle 包成事件出口，supervisor 本身不依赖 Tauri 运行时
fn sink_of(app: &AppHandle) -> Arc<dyn EventSink> {
    Arc::new(TauriSink(app.clone()))
}
use crate::state::AppState;

type St<'a> = State<'a, Arc<AppState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    info: AppInfo,
    /// 项目（组件分组）
    projects: Vec<Project>,
    /// 组件（可启动单元）
    components: Vec<Component>,
    templates: Vec<Template>,
    settings: Settings,
    sessions: Vec<SessionView>,
    metrics: HashMap<String, MetricRow>,
    bulk: Option<BulkView>,
}

fn snapshot_of(state: &Arc<AppState>) -> Snapshot {
    let (projects, components, templates, settings) = {
        let data = state.data.lock().unwrap();
        (
            data.projects.clone(),
            data.components.clone(),
            data.templates.clone(),
            data.settings.clone(),
        )
    };
    let sessions = supervisor::sessions_snapshot(state);
    let metrics = state.metrics.lock().unwrap().clone();
    let bulk = state.bulk.lock().unwrap().clone();
    let info = AppInfo {
        platform: std::env::consts::OS.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_root: state.root.display().to_string(),
        writable: state.writable,
        portable: cfg!(windows),
        shell: settings.shell.clone(),
        shells: shell_candidates(),
        webview: if cfg!(windows) {
            "WebView2".into()
        } else if cfg!(target_os = "macos") {
            "WKWebView".into()
        } else {
            "WebKitGTK".into()
        },
    };
    Snapshot {
        info,
        projects,
        components,
        templates,
        settings,
        sessions,
        metrics,
        bulk,
    }
}

/// 设置 / 项目编辑器里的 shell 候选（按平台给，附带是否已安装）
fn shell_candidates() -> Vec<ShellOption> {
    let mut out: Vec<ShellOption> = Vec::new();
    let push = |out: &mut Vec<ShellOption>, value: &str, label: &str, kind: &str| {
        out.push(ShellOption {
            value: value.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            available: store::command_exists(value),
        });
    };

    if cfg!(windows) {
        push(&mut out, "pwsh.exe", "PowerShell 7（pwsh）", "pwsh");
        push(&mut out, "powershell.exe", "Windows PowerShell（powershell）", "powershell");
        push(&mut out, "cmd.exe", "CMD（cmd）", "cmd");
    } else {
        push(&mut out, "/bin/zsh", "zsh（macOS 默认）", "posix");
        push(&mut out, "/bin/bash", "bash", "posix");
        push(&mut out, "/bin/sh", "sh", "posix");
        push(&mut out, "pwsh", "PowerShell 7（pwsh，若已安装）", "pwsh");
    }
    out
}

fn gen_id(prefix: &str) -> String {
    format!("{}{}", prefix, store::now_ms())
}

fn expand_home(path: &str) -> String {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = store::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    trimmed.to_string()
}

fn normalize_component(p: &mut Component, projects: &[Project]) -> Result<(), String> {
    p.name = p.name.trim().to_string();
    if p.name.is_empty() {
        return Err("组件名称不能为空".into());
    }
    for step in p.commands.iter_mut() {
        step.cmd = step.cmd.trim().to_string();
    }
    p.commands.retain(|s| !s.cmd.is_empty());
    if p.commands.is_empty() {
        return Err("启动命令不能为空".into());
    }
    // 第一条命令前不需要连接符
    p.commands[0].join = Join::And;
    p.workdir = expand_home(&p.workdir);
    if p.workdir.is_empty() {
        return Err("工作目录不能为空".into());
    }
    if p.id.trim().is_empty() {
        p.id = gen_id("c");
    }
    p.id = p.id.trim().to_string();
    if !projects.iter().any(|x| x.id == p.project_id) {
        p.project_id = "all".into();
    }
    p.ports.sort_unstable();
    p.ports.dedup();
    p.ports.retain(|x| *x > 0);
    p.depends_on.retain(|d| d != &p.id && !d.trim().is_empty());
    p.depends_on.sort();
    p.depends_on.dedup();
    p.env.retain(|k, _| !k.trim().is_empty());
    Ok(())
}

// ---------------------------------------------------------------- 读取

#[tauri::command]
pub fn bootstrap(state: St<'_>) -> Snapshot {
    snapshot_of(state.inner())
}

// ---------------------------------------------------------------- 项目 / 分类

#[tauri::command]
pub fn save_component(state: St<'_>, mut component: Component) -> Result<Snapshot, String> {
    {
        let mut data = state.data.lock().unwrap();
        normalize_component(&mut component, &data.projects)?;
        match data.components.iter_mut().find(|c| c.id == component.id) {
            Some(existing) => *existing = component.clone(),
            None => data.components.push(component.clone()),
        }
        data.save_projects()
            .map_err(|e| format!("保存 projects.json 失败：{e}"))?;
    }
    // 让终端里的名字 / 归属立即跟上（命令本身在下次启动生效）
    if let Some(session) = { state.sessions.lock().unwrap().get(&component.id).cloned() } {
        let mut spec = session.spec.lock().unwrap();
        spec.name = component.name.clone();
        spec.project_id = component.project_id.clone();
        spec.ports = component.ports.clone();
        spec.workdir = component.workdir.clone();
    }
    Ok(snapshot_of(state.inner()))
}

#[tauri::command]
pub fn delete_component(state: St<'_>, id: String) -> Result<Snapshot, String> {
    {
        let sessions = state.sessions.lock().unwrap();
        if let Some(s) = sessions.get(&id) {
            if s.status().is_active() {
                return Err("组件正在运行，请先停止再删除".into());
            }
        }
    }
    {
        let mut data = state.data.lock().unwrap();
        data.components.retain(|c| c.id != id);
        for c in data.components.iter_mut() {
            c.depends_on.retain(|d| d != &id);
        }
        data.save_projects()
            .map_err(|e| format!("保存 projects.json 失败：{e}"))?;
    }
    state.sessions.lock().unwrap().remove(&id);
    state.metrics.lock().unwrap().remove(&id);
    Ok(snapshot_of(state.inner()))
}

#[tauri::command]
pub fn save_project(state: St<'_>, mut project: Project) -> Result<Snapshot, String> {
    {
        let mut data = state.data.lock().unwrap();
        project.name = project.name.trim().to_string();
        if project.name.is_empty() {
            return Err("项目名称不能为空".into());
        }
        if project.id.trim().is_empty() || project.id == "new" {
            project.id = gen_id("pj");
        }
        if project.order == 0 && project.id != "all" {
            project.order = data.projects.iter().map(|p| p.order).max().unwrap_or(0) + 1;
        }
        match data.projects.iter_mut().find(|p| p.id == project.id) {
            Some(existing) => *existing = project,
            None => data.projects.push(project),
        }
        data.projects.sort_by_key(|p| (p.order, p.name.clone()));
        data.save_projects()
            .map_err(|e| format!("保存 projects.json 失败：{e}"))?;
    }
    Ok(snapshot_of(state.inner()))
}

#[tauri::command]
pub fn delete_project(state: St<'_>, id: String) -> Result<Snapshot, String> {
    if id == "all" {
        return Err("「全部项目」是内置视图，不能删除".into());
    }
    {
        let mut data = state.data.lock().unwrap();
        data.projects.retain(|p| p.id != id);
        for c in data.components.iter_mut() {
            if c.project_id == id {
                c.project_id = "all".into();
            }
        }
        data.save_projects()
            .map_err(|e| format!("保存 projects.json 失败：{e}"))?;
    }
    // 归属变了，同步到已有会话
    for s in state.sessions.lock().unwrap().values().cloned().collect::<Vec<_>>() {
        let mut spec = s.spec.lock().unwrap();
        if spec.project_id == id {
            spec.project_id = "all".into();
        }
    }
    Ok(snapshot_of(state.inner()))
}

// ---------------------------------------------------------------- 组件配置导入导出

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    imported: usize,
    skipped: Vec<String>,
    snapshot: Snapshot,
}

/// 把选中的组件导出成 JSON 文件（ids 为空则导出全部）
#[tauri::command]
pub fn export_components(state: St<'_>, ids: Vec<String>, path: String) -> Result<String, String> {
    let target = expand_home(&path);
    crate::transfer::export_to(state.inner(), &ids, &target)?;
    Ok(target)
}

/// 从 JSON 文件导入组件
#[tauri::command]
pub fn import_components(state: St<'_>, path: String) -> Result<ImportResult, String> {
    let (imported, skipped) = crate::transfer::import_from(state.inner(), &expand_home(&path))?;
    Ok(ImportResult {
        imported,
        skipped,
        snapshot: snapshot_of(state.inner()),
    })
}

// ---------------------------------------------------------------- 模板 / 设置

#[tauri::command]
pub fn save_template(state: St<'_>, mut template: Template) -> Result<Snapshot, String> {
    {
        let mut data = state.data.lock().unwrap();
        template.name = template.name.trim().to_string();
        if template.name.is_empty() {
            return Err("模板名称不能为空".into());
        }
        if template.id.trim().is_empty() {
            template.id = gen_id("tpl");
        }
        template.updated_at = store::now_ms();
        template.env.retain(|k, _| !k.trim().is_empty());
        match data.templates.iter_mut().find(|t| t.id == template.id) {
            Some(existing) => *existing = template,
            None => data.templates.push(template),
        }
        data.save_templates()
            .map_err(|e| format!("保存 templates.json 失败：{e}"))?;
    }
    Ok(snapshot_of(state.inner()))
}

#[tauri::command]
pub fn delete_template(state: St<'_>, id: String) -> Result<Snapshot, String> {
    {
        let mut data = state.data.lock().unwrap();
        data.templates.retain(|t| t.id != id);
        data.save_templates()
            .map_err(|e| format!("保存 templates.json 失败：{e}"))?;
    }
    Ok(snapshot_of(state.inner()))
}

#[tauri::command]
pub fn save_settings(state: St<'_>, settings: Settings) -> Result<Snapshot, String> {
    {
        let mut data = state.data.lock().unwrap();
        let mut next = settings;
        if next.shell.trim().is_empty() {
            next.shell = if cfg!(windows) { "pwsh.exe".into() } else { "/bin/zsh".into() };
        }
        next.log_lines_cap = next.log_lines_cap.clamp(300, 20_000);
        next.refresh_ms = next.refresh_ms.clamp(500, 10_000);
        data.settings = next;
        data.save_settings()
            .map_err(|e| format!("保存 settings.json 失败：{e}"))?;
    }
    Ok(snapshot_of(state.inner()))
}

// ---------------------------------------------------------------- 运行控制

fn emit_toast(app: &AppHandle, level: &str, text: impl Into<String>) {
    let _ = app.emit(
        "ck:toast",
        serde_json::json!({ "level": level, "text": text.into() }),
    );
}

fn spec_for(state: &Arc<AppState>, id: &str) -> Result<SpawnSpec, String> {
    let data = state.data.lock().unwrap();
    data.components
        .iter()
        .find(|c| c.id == id)
        .map(supervisor::spec_from_component)
        .ok_or_else(|| format!("未知组件：{id}"))
}

#[tauri::command]
pub async fn run_component(
    app: AppHandle,
    state: St<'_>,
    id: String,
) -> Result<(), String> {
    let shared = state.inner().clone();
    let spec = spec_for(&shared, &id)?;
    supervisor::start(sink_of(&app), shared, spec).await
}

#[tauri::command]
pub async fn stop_component(app: AppHandle, state: St<'_>, id: String) -> Result<(), String> {
    let shared = state.inner().clone();
    supervisor::stop(&sink_of(&app), &shared, &id).await
}

#[tauri::command]
pub async fn restart_component(app: AppHandle, state: St<'_>, id: String) -> Result<(), String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = supervisor::restart(&sink_of(&app), &shared, &id).await {
            emit_toast(&app, "error", e);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn run_many(
    app: AppHandle,
    state: St<'_>,
    ids: Vec<String>,
) -> Result<(), String> {
    let shared = state.inner().clone();
    // 先同步校验依赖图，循环依赖立刻回给调用方
    {
        let data = shared.data.lock().unwrap();
        supervisor::topo_order(&data.components, &ids)?;
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = supervisor::start_many(sink_of(&app2), shared, ids, "run").await {
            emit_toast(&app2, "error", e);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn stop_many_cmd(
    app: AppHandle,
    state: St<'_>,
    ids: Vec<String>,
) -> Result<(), String> {
    let shared = state.inner().clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = supervisor::stop_many(sink_of(&app2), shared, ids, "stop").await {
            emit_toast(&app2, "error", e);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn restart_many(
    app: AppHandle,
    state: St<'_>,
    ids: Vec<String>,
) -> Result<(), String> {
    let shared = state.inner().clone();
    {
        let data = shared.data.lock().unwrap();
        supervisor::topo_order(&data.components, &ids)?;
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let ids2 = ids.clone();
        if let Err(e) = supervisor::stop_many(sink_of(&app2), shared.clone(), ids, "restart").await {
            emit_toast(&app2, "error", e);
            return;
        }
        if let Err(e) = supervisor::start_many(sink_of(&app2), shared, ids2, "restart").await {
            emit_toast(&app2, "error", e);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_bulk_cmd(app: AppHandle, state: St<'_>) -> Result<(), String> {
    let shared = state.inner().clone();
    supervisor::cancel_bulk(&shared, &sink_of(&app));
    let order = shared
        .bulk
        .lock()
        .unwrap()
        .as_ref()
        .map(|b| b.order.clone())
        .unwrap_or_default();
    if !order.is_empty() {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = supervisor::stop_many(sink_of(&app2), shared, order, "stop").await;
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn run_template(app: AppHandle, state: St<'_>, id: String) -> Result<(), String> {
    let shared = state.inner().clone();
    let spec = {
        let data = shared.data.lock().unwrap();
        let shell = data.settings.shell.clone();
        data.templates
            .iter()
            .find(|t| t.id == id)
            .map(|t| supervisor::spec_from_template(t, shell))
            .ok_or_else(|| format!("未知模板：{id}"))?
    };
    supervisor::start(sink_of(&app), shared, spec).await
}

// ---------------------------------------------------------------- 终端

#[tauri::command]
pub fn send_input_cmd(state: St<'_>, id: String, data: String) -> Result<(), String> {
    supervisor::send_input(state.inner(), &id, &data)
}

#[tauri::command]
pub fn session_log(state: St<'_>, id: String) -> Vec<LogChunk> {
    supervisor::log_snapshot(state.inner(), &id)
}

#[tauri::command]
pub fn clear_session_log(state: St<'_>, id: String) {
    supervisor::clear_log(state.inner(), &id);
}

#[tauri::command]
pub fn process_detail(state: St<'_>, id: String) -> ProcessDetail {
    crate::metrics::detail(state.inner(), &id)
}

// ---------------------------------------------------------------- 导出 / 打开

#[tauri::command]
pub fn export_log(state: St<'_>, id: String) -> Result<String, String> {
    let chunks = supervisor::log_snapshot(state.inner(), &id);
    if chunks.is_empty() {
        return Err("该会话没有日志".into());
    }
    let dir = state.root.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{}.log", crate::state::sanitize_id(&id), store::now_ms()));
    let mut text = String::new();
    for c in chunks {
        text.push_str(&c.data);
        if !c.data.ends_with('\n') {
            text.push('\n');
        }
    }
    std::fs::write(&path, text).map_err(|e| format!("写入失败：{e}"))?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn export_snapshot(state: St<'_>) -> Result<String, String> {
    let snap = snapshot_of(state.inner());
    let payload = serde_json::json!({
        "exportedAt": store::now_ms(),
        "platform": snap.info.platform,
        "dataRoot": snap.info.data_root,
        "sessions": snap.sessions,
        "metrics": snap.metrics,
    });
    let data = state.data.lock().unwrap();
    data.snapshot_file
        .write(&payload)
        .map_err(|e| format!("写入快照失败：{e}"))?;
    Ok(data.snapshot_file.path().display().to_string())
}

#[tauri::command]
pub fn open_data_dir(state: St<'_>) -> Result<(), String> {
    procs::open_path(&state.root.display().to_string())
}

#[tauri::command]
pub fn open_workdir(state: St<'_>, id: String) -> Result<(), String> {
    let path = {
        let data = state.data.lock().unwrap();
        data.components
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.workdir.clone())
    }
    .or_else(|| {
        state
            .sessions
            .lock()
            .unwrap()
            .get(&id)
            .map(|s| s.spec.lock().unwrap().workdir.clone())
    })
    .ok_or_else(|| format!("未知组件：{id}"))?;
    procs::open_path(&path)
}

#[tauri::command]
pub fn open_external_terminal(state: St<'_>, id: String) -> Result<(), String> {
    let path = {
        let data = state.data.lock().unwrap();
        data.components
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.workdir.clone())
    }
    .ok_or_else(|| format!("未知组件：{id}"))?;
    procs::open_terminal(&path)
}
