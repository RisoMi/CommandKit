mod commands;
mod metrics;
pub mod model;
pub mod proc;
pub mod state;
pub mod store;
pub mod supervisor;

use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        // 单实例：第二个实例直接唤起已有窗口（叠加 data 目录文件锁，避免并发写 JSON）
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .setup(|app| {
            let handle = app.handle().clone();
            let fallback = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("CommandKit"));
            let root = store::resolve_data_root(fallback);
            let writable = store::ensure_writable(&root).is_ok();
            if !writable {
                eprintln!(
                    "[commandkit] 数据目录不可写：{}（Program Files / 只读盘 / 网络盘？）",
                    root.display()
                );
            }
            let lock = store::acquire_lock(&root);
            if lock.is_none() {
                eprintln!("[commandkit] 数据目录已被其它实例锁定：{}", root.display());
            }
            let mut loaded = state::load_state(root, lock);
            loaded.writable = writable;
            let shared = Arc::new(loaded);
            app.manage(shared.clone());
            metrics::start(handle.clone(), shared.clone());

            // 标记了 autoStart 的项目，启动后按依赖顺序自动拉起一次
            let boot_app = handle.clone();
            let boot_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                let ids: Vec<String> = {
                    let data = boot_state.data.lock().unwrap();
                    data.projects
                        .iter()
                        .filter(|p| p.auto_start)
                        .map(|p| p.id.clone())
                        .collect()
                };
                if !ids.is_empty() {
                    let sink: Arc<dyn supervisor::EventSink> =
                        Arc::new(supervisor::TauriSink(boot_app));
                    let _ = supervisor::start_many(sink, boot_state, ids, "run").await;
                }
            });

            // 崩溃自动重启看护：独立于 start 的循环，被标记 restartPending 的会话在此重新拉起
            let tick_app = handle.clone();
            let tick_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                    let due: Vec<String> = {
                        let sessions = tick_state.sessions.lock().unwrap();
                        sessions
                            .values()
                            .filter(|s| s.inner.lock().unwrap().restart_pending)
                            .map(|s| s.id.clone())
                            .collect()
                    };
                    for id in due {
                        let target = { tick_state.sessions.lock().unwrap().get(&id).cloned() };
                        let Some(session) = target else { continue };
                        {
                            let mut inner = session.inner.lock().unwrap();
                            if inner.status.is_active() {
                                continue;
                            }
                            inner.restart_pending = false;
                        }
                        let spec = session.spec.lock().unwrap().clone();
                        let sink: Arc<dyn supervisor::EventSink> =
                            Arc::new(supervisor::TauriSink(tick_app.clone()));
                        let _ = supervisor::start(sink, tick_state.clone(), spec).await;
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_project,
            commands::delete_project,
            commands::save_category,
            commands::delete_category,
            commands::save_template,
            commands::delete_template,
            commands::save_settings,
            commands::run_project,
            commands::stop_project,
            commands::restart_project,
            commands::run_many,
            commands::stop_many_cmd,
            commands::restart_many,
            commands::cancel_bulk_cmd,
            commands::run_template,
            commands::send_input_cmd,
            commands::session_log,
            commands::clear_session_log,
            commands::process_detail,
            commands::export_log,
            commands::export_snapshot,
            commands::open_data_dir,
            commands::open_workdir,
            commands::open_external_terminal,
        ])
        .run(tauri::generate_context!())
        .expect("CommandKit 启动失败");
}
