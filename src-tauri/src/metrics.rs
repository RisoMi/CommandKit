//! 进程指标与详情：基于 sysinfo 的进程表，按会话根 PID 聚合成整棵进程树。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter};

use crate::model::*;
use crate::state::AppState;
use crate::supervisor;

pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut sys = System::new();
        loop {
            let wait = {
                let data = state.data.lock().unwrap();
                data.settings.refresh_ms.max(500)
            };
            let targets: Vec<(String, Option<u32>)> = {
                let sessions = state.sessions.lock().unwrap();
                sessions
                    .values()
                    .map(|s| (s.id.clone(), s.inner.lock().unwrap().pid))
                    .collect()
            };

            let mut rows: HashMap<String, MetricRow> = HashMap::new();
            if targets.is_empty() {
                sys.refresh_processes(ProcessesToUpdate::All, true);
            } else {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                let children = children_map(&sys);
                for (id, pid) in targets {
                    match pid {
                        Some(pid) => {
                            let (cpu, mem, count) = aggregate(&sys, &children, pid);
                            rows.insert(
                                id.clone(),
                                MetricRow {
                                    id,
                                    pid: Some(pid),
                                    cpu,
                                    mem,
                                    children: count,
                                },
                            );
                        }
                        None => {
                            rows.insert(
                                id.clone(),
                                MetricRow {
                                    id,
                                    pid: None,
                                    cpu: 0.0,
                                    mem: 0,
                                    children: 0,
                                },
                            );
                        }
                    }
                }
            }

            *state.metrics.lock().unwrap() = rows.clone();
            let _ = app.emit("ck:metrics", &rows);
            tokio::time::sleep(Duration::from_millis(wait)).await;
        }
    });
}

fn children_map(sys: &System) -> HashMap<Pid, Vec<Pid>> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, process) in sys.processes() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    children
}

fn aggregate(sys: &System, children: &HashMap<Pid, Vec<Pid>>, root: u32) -> (f32, u64, u32) {
    let root_pid = Pid::from_u32(root);
    let mut cpu = 0.0f32;
    let mut mem = 0u64;
    let mut extra = 0u32;
    let mut seen: HashSet<Pid> = HashSet::new();
    let mut queue: VecDeque<Pid> = VecDeque::new();
    queue.push_back(root_pid);
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(process) = sys.process(pid) {
            cpu += process.cpu_usage();
            mem += process.memory();
            if pid != root_pid {
                extra += 1;
            }
        }
        if let Some(kids) = children.get(&pid) {
            for kid in kids {
                queue.push_back(*kid);
            }
        }
    }
    (cpu, mem, extra)
}

fn mask_env(key: &str, value: &str) -> String {
    let upper = key.to_uppercase();
    let sensitive = ["SECRET", "TOKEN", "PASSWORD", "PASSWD", "PRIVATE_KEY", "ACCESS_KEY"]
        .iter()
        .any(|k| upper.contains(k));
    if sensitive {
        "••••••••（已脱敏）".into()
    } else {
        value.to_string()
    }
}

pub fn detail(state: &Arc<AppState>, id: &str) -> ProcessDetail {
    let session = supervisor::get_session(state, id);
    let spec = session
        .as_ref()
        .map(|s| s.spec.lock().unwrap().clone());
    let pid = session
        .as_ref()
        .and_then(|s| s.inner.lock().unwrap().pid);

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut tree: Vec<ProcNode> = Vec::new();
    if let Some(root) = pid {
        let children = children_map(&sys);
        let mut seen: HashSet<Pid> = HashSet::new();
        let mut queue: VecDeque<Pid> = VecDeque::new();
        queue.push_back(Pid::from_u32(root));
        while let Some(p) = queue.pop_front() {
            if !seen.insert(p) {
                continue;
            }
            if let Some(process) = sys.process(p) {
                tree.push(ProcNode {
                    pid: p.as_u32(),
                    ppid: process.parent().map(|x| x.as_u32()),
                    name: process.name().to_string_lossy().to_string(),
                    cmd: process
                        .cmd()
                        .iter()
                        .map(|c| c.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(" "),
                    cpu: process.cpu_usage(),
                    mem: process.memory(),
                });
            }
            if let Some(kids) = children.get(&p) {
                for kid in kids {
                    queue.push_back(*kid);
                }
            }
        }
    }

    let env = spec
        .as_ref()
        .map(|s| {
            s.env
                .iter()
                .map(|(k, v)| (k.clone(), mask_env(k, v)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let listen = spec
        .as_ref()
        .map(|s| {
            s.ports
                .iter()
                .map(|port| {
                    if crate::proc::port_busy(*port) {
                        format!("127.0.0.1:{port}  LISTEN  {}", pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()))
                    } else {
                        format!("127.0.0.1:{port}  —  未监听")
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let log_tail = supervisor::log_snapshot(state, id)
        .into_iter()
        .flat_map(|c| c.data.lines().map(|l| l.to_string()).collect::<Vec<_>>())
        .rev()
        .take(14)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    ProcessDetail {
        id: id.to_string(),
        tree,
        env,
        listen,
        log_tail,
    }
}
