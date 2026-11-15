//! 运行时端到端测试：真起子 shell、抓输出、健康检查、整树终止。
//!
//! supervisor 只依赖 `EventSink` 抽象，测试用 `NullSink` 即可，
//! 不需要 Tauri 运行时（否则 Windows 上会因链接 wry/WebView2 而 STATUS_ENTRYPOINT_NOT_FOUND）。
//!
//! 命令按平台分支：macOS 走 zsh 语法，Windows 走 PowerShell 语法（CI 两个平台都跑）。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use commandkit_lib::model::{HealthCheck, Policy, Status};
use commandkit_lib::state::AppState;
use commandkit_lib::supervisor::{self, EventSink, NullSink, SpawnSpec};

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

/// 事件出口：测试里吞掉所有事件
fn sink() -> Arc<dyn EventSink> {
    Arc::new(NullSink)
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
    let sink = sink();
    let state = make_state("out");
    let cmd = if cfg!(windows) {
        "Write-Output 'hello-from-test'; [Console]::Error.WriteLine('错误行'); Start-Sleep -Milliseconds 300"
    } else {
        "echo hello-from-test && echo 错误行 1>&2 && sleep 0.3"
    };
    let s = spec("t-out", cmd, None);
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();

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
    let sink = sink();
    let state = make_state("fail");
    let s = spec("t-fail", "exit 3", None);
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();
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

    let sink = sink();
    let state = make_state("health");
    let health = HealthCheck {
        url: None,
        port: Some(port),
        timeout_ms: 5000,
        retry: 20,
        interval_ms: 150,
    };
    let s = spec("t-health", long_running(), Some(health));
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();

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

    supervisor::stop(&sink, &state, "t-health").await.unwrap();
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
    let sink = sink();
    let state = make_state("env");
    let cmd = if cfg!(windows) {
        "Write-Output \"CK-$env:CK_MARK\""
    } else {
        "echo CK-$CK_MARK"
    };
    let mut s = spec("t-env", cmd, None);
    s.env.insert("CK_MARK".into(), "9527".into());
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();
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

/// CMD 与 POSIX shell 的调用形式不同（/D /C vs -lc），这里真跑一条命令验证选中的 shell 生效
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_shell_override_is_used() {
    let sink = sink();
    let state = make_state("shell");
    let id = "t-shell";

    let (shell, command, expect) = if cfg!(windows) {
        ("cmd.exe", "echo ck-cmd-ok", "ck-cmd-ok")
    } else {
        ("/bin/bash", "echo ck-bash-ok", "ck-bash-ok")
    };

    let mut s = spec(id, command, None);
    s.shell = Some(shell.to_string());
    supervisor::start(sink, state.clone(), s).await.unwrap();
    assert!(
        wait_status(&state, id, Status::Stopped, 15000).await,
        "指定 shell 后应正常跑完，实际：{:?}",
        supervisor::get_session(&state, id).map(|s| s.view().status)
    );

    let text: String = supervisor::log_snapshot(&state, id)
        .into_iter()
        .map(|c| c.data)
        .collect();
    assert!(text.contains(expect), "{shell} 应执行命令：{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_workdir_reports_error_and_keeps_state_clean() {
    let sink = sink();
    let state = make_state("no-dir");
    let mut s = spec("t-dir", "echo x", None);
    s.workdir = "/definitely/not/here-commandkit".into();
    let err = supervisor::start(sink.clone(), state.clone(), s).await.unwrap_err();
    assert!(err.contains("工作目录不存在"), "应给出明确原因：{err}");
}
