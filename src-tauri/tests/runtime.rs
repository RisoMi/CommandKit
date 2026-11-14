//! 运行时端到端测试：真起子 shell、抓输出、健康检查、整树终止。
//! 用 tauri 的 mock 运行时构造 AppHandle，不需要开窗口。
//!
//! 命令按平台分支：macOS 走 zsh 语法，Windows 走 PowerShell 语法（CI 两个平台都跑）。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use commandkit_lib::model::{HealthCheck, Policy, Status};
use commandkit_lib::state::AppState;
use commandkit_lib::supervisor::{self, SpawnSpec};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("commandkit-rt-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec(id: &str, command: &str, health: Option<HealthCheck>) -> SpawnSpec {
    SpawnSpec {
        id: id.into(),
        name: id.into(),
        kind: "project".into(),
        category: "all".into(),
        command: command.into(),
        workdir: std::env::temp_dir().display().to_string(),
        env: BTreeMap::new(),
        ports: vec![],
        health,
        policy: Policy::default(),
        shell: None,
    }
}

fn app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
        .handle()
        .clone()
}

fn make_state(tag: &str) -> Arc<AppState> {
    Arc::new(commandkit_lib::state::load_state(temp_root(tag), None))
}

async fn wait_status(state: &Arc<AppState>, id: &str, want: Status, timeout_ms: u64) -> bool {
    let steps = timeout_ms / 100;
    for _ in 0..steps {
        if supervisor::get_session(state, id).map(|s| s.status()) == Some(want) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
}

fn long_running() -> &'static str {
    if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captures_stdout_and_exit_code() {
    let handle = app_handle();
    let state = make_state("out");
    let cmd = if cfg!(windows) {
        "Write-Output 'hello-from-test'; [Console]::Error.WriteLine('错误行'); Start-Sleep -Milliseconds 300"
    } else {
        "echo hello-from-test && echo 错误行 1>&2 && sleep 0.3"
    };
    let s = spec("t-out", cmd, None);
    supervisor::start(handle, state.clone(), s).await.unwrap();

    assert!(
        wait_status(&state, "t-out", Status::Stopped, 12000).await,
        "正常退出后应转 Stopped，实际：{:?}",
        supervisor::get_session(&state, "t-out").map(|s| s.view().status)
    );

    let text: String = supervisor::log_snapshot(&state, "t-out")
        .into_iter()
        .map(|c| c.data)
        .collect();
    assert!(text.contains("hello-from-test"), "应捕获 stdout：{text}");
    assert!(text.contains("错误行"), "应捕获 stderr：{text}");
    assert!(text.contains("会话结束"), "应记录退出摘要：{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_zero_exit_marks_failed() {
    let handle = app_handle();
    let state = make_state("fail");
    let s = spec("t-fail", "exit 3", None);
    supervisor::start(handle, state.clone(), s).await.unwrap();
    assert!(
        wait_status(&state, "t-fail", Status::Failed, 12000).await,
        "非零退出应标记 Failed"
    );
    let view = supervisor::get_session(&state, "t-fail").unwrap().view();
    assert_eq!(view.exit_code, Some(3));
    assert!(view.error.unwrap_or_default().contains("退出码"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_gates_running_then_stop_kills_tree() {
    // 先占一个端口充当“服务”，健康检查立即可过
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || listener.accept().map(|_| ()).ok());

    let handle = app_handle();
    let state = make_state("health");
    let health = HealthCheck {
        url: None,
        port: Some(port),
        timeout_ms: 5000,
        retry: 20,
        interval_ms: 150,
    };
    let s = spec("t-health", long_running(), Some(health));
    supervisor::start(handle, state.clone(), s).await.unwrap();

    assert!(
        wait_status(&state, "t-health", Status::Running, 12000).await,
        "健康检查通过后应转 Running"
    );
    let pid = supervisor::get_session(&state, "t-health")
        .unwrap()
        .view()
        .pid
        .unwrap();
    assert!(pid > 0);

    supervisor::stop(&app_handle(), &state, "t-health")
        .await
        .unwrap();
    assert!(
        wait_status(&state, "t-health", Status::Stopped, 12000).await,
        "停止后应转 Stopped"
    );

    // 进程树应被终止（留出 SIGTERM → SIGKILL / taskkill 的收敛时间）
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!is_alive(pid), "停止后 pid {pid} 不应存活");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_is_injected_without_touching_system() {
    let handle = app_handle();
    let state = make_state("env");
    let cmd = if cfg!(windows) {
        "Write-Output \"CK-$env:CK_MARK\""
    } else {
        "echo CK-$CK_MARK"
    };
    let mut s = spec("t-env", cmd, None);
    s.env.insert("CK_MARK".into(), "9527".into());
    supervisor::start(handle, state.clone(), s).await.unwrap();
    assert!(wait_status(&state, "t-env", Status::Stopped, 12000).await);

    let text: String = supervisor::log_snapshot(&state, "t-env")
        .into_iter()
        .map(|c| c.data)
        .collect();
    assert!(text.contains("CK-9527"), "子进程应收到注入的环境变量：{text}");
    assert!(
        std::env::var("CK_MARK").is_err(),
        "宿主进程环境不应被修改"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_workdir_reports_error_and_keeps_state_clean() {
    let handle = app_handle();
    let state = make_state("no-dir");
    let mut s = spec("t-dir", "echo x", None);
    s.workdir = "/definitely/not/here-commandkit".into();
    let err = supervisor::start(handle, state.clone(), s).await.unwrap_err();
    assert!(err.contains("工作目录不存在"), "应给出明确原因：{err}");
}
