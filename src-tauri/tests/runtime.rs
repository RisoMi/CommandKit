//! 运行时端到端测试：真起子 shell、抓输出、健康检查、整树终止。
//!
//! supervisor 只依赖 `EventSink` 抽象，测试用 `NullSink` 即可，
//! 不需要 Tauri 运行时（否则 Windows 上会因链接 wry/WebView2 而 STATUS_ENTRYPOINT_NOT_FOUND）。
//!
//! 命令按平台分支：macOS 走 zsh 语法，Windows 走 PowerShell 语法（CI 两个平台都跑）。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use commandkit_lib::model::{CommandStep, HealthCheck, Join, Policy, Status};
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
        kind: "component".into(),
        project_id: "all".into(),
        stdin_mode: "pipe".into(),
        term_mode: "pipe".into(),
        steps: vec![CommandStep::new(command, Join::And)],
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

fn spec_steps(id: &str, steps: Vec<(&str, Join)>) -> SpawnSpec {
    let mut spec = spec(id, "", None);
    spec.steps = steps
        .into_iter()
        .map(|(cmd, join)| CommandStep::new(cmd, join))
        .collect();
    spec.command = spec
        .steps
        .iter()
        .map(|s| s.cmd.clone())
        .collect::<Vec<_>>()
        .join(" / ");
    spec
}

fn log_text(state: &Arc<AppState>, id: &str) -> String {
    supervisor::log_snapshot(state, id)
        .into_iter()
        .map(|c| c.data)
        .collect()
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

/// java -version 这类命令：只写 stderr、且瞬间退出（没有 sleep 兜底）
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captures_stderr_only_fast_command() {
    let sink = sink();
    let state = make_state("stderr-only");
    let cmd = if cfg!(windows) {
        "cmd.exe /D /C \"echo fast-err-only 1>&2\""
    } else {
        "echo fast-err-only 1>&2"
    };
    let s = spec("t-stderr", cmd, None);
    supervisor::start(sink, state.clone(), s).await.unwrap();
    assert!(wait_status(&state, "t-stderr", Status::Stopped, 12000).await);

    let text: String = supervisor::log_snapshot(&state, "t-stderr")
        .into_iter()
        .map(|c| c.data)
        .collect();
    assert!(
        text.contains("fast-err-only"),
        "只写 stderr 的快速命令也必须被捕获：{text:?}"
    );
}

/// 且：顺序执行，成功继续
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_and_runs_all_steps_in_order() {
    let sink = sink();
    let state = make_state("chain-and");
    let spec = spec_steps(
        "t-and",
        vec![("echo step-one", Join::And), ("echo step-two", Join::And)],
    );
    supervisor::start(sink, state.clone(), spec).await.unwrap();
    assert!(wait_status(&state, "t-and", Status::Stopped, 15000).await);

    let text = log_text(&state, "t-and");
    let h1 = text.find("步骤 1/2").expect("应有第 1 步标记");
    let h2 = text.find("步骤 2/2").expect("应有第 2 步标记");
    assert!(h1 < h2, "步骤必须按顺序执行：{text}");
    // 逐行比对：两步各自的 stdout 都要作为独立行出现
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    assert!(lines.contains(&"step-one"), "第一步应有输出：{text}");
    assert!(lines.contains(&"step-two"), "第二步应有输出：{text}");
}

/// 或：上一条失败时改跑备选
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_or_runs_fallback_after_failure() {
    let sink = sink();
    let state = make_state("chain-or-fail");
    let spec = spec_steps(
        "t-or-fail",
        vec![("exit 7", Join::And), ("echo fallback-ran", Join::Or)],
    );
    supervisor::start(sink, state.clone(), spec).await.unwrap();
    assert!(wait_status(&state, "t-or-fail", Status::Stopped, 15000).await);

    let text = log_text(&state, "t-or-fail");
    assert!(text.contains("步骤 2/2"), "备选命令应被执行：{text}");
    assert!(text.contains("fallback-ran"), "备选命令应被执行：{text}");
    let view = supervisor::get_session(&state, "t-or-fail").unwrap().view();
    assert_eq!(view.exit_code, Some(0), "整条链以成功收尾");
}

/// 或：上一条成功时跳过备选
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_or_skips_fallback_after_success() {
    let sink = sink();
    let state = make_state("chain-or-ok");
    let spec = spec_steps(
        "t-or-ok",
        vec![("echo primary-ok", Join::And), ("echo should-not-run", Join::Or)],
    );
    supervisor::start(sink, state.clone(), spec).await.unwrap();
    assert!(wait_status(&state, "t-or-ok", Status::Stopped, 15000).await);

    let text = log_text(&state, "t-or-ok");
    assert!(text.contains("primary-ok"), "主命令应执行：{text}");
    // banner 里也会出现命令文本，用「步骤 2/2」标记判断备选是否真的执行
    assert!(!text.contains("步骤 2/2"), "成功时不该跑备选：{text}");
}

/// 且：上一条失败即中止
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_and_stops_after_failure() {
    let sink = sink();
    let state = make_state("chain-and-fail");
    let spec = spec_steps(
        "t-and-fail",
        vec![("exit 5", Join::And), ("echo never-ran", Join::And)],
    );
    supervisor::start(sink, state.clone(), spec).await.unwrap();
    assert!(wait_status(&state, "t-and-fail", Status::Failed, 15000).await);

    let text = log_text(&state, "t-and-fail");
    assert!(!text.contains("步骤 2/2"), "失败后不该继续：{text}");
    let view = supervisor::get_session(&state, "t-and-fail").unwrap().view();
    assert_eq!(view.exit_code, Some(5));
}

/// 停止兜底：进程忽略 SIGTERM 也必须从 Stopping 收敛到 Stopped
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_never_stuck_in_stopping() {
    let sink = sink();
    let state = make_state("stop-watchdog");
    let cmd = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "trap '' TERM; sleep 30"
    };
    let s = spec("t-stuck", cmd, None);
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    supervisor::stop(&sink, &state, "t-stuck").await.unwrap();

    // 8 秒内必须离开 Starting/Running/Stopping
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let st = supervisor::get_session(&state, "t-stuck").unwrap().view().status;
        if !matches!(st, Status::Starting | Status::Running | Status::Restarting | Status::Stopping) {
            println!("ended with {st:?}");
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("停止后一直卡在 {st:?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// PTY 模式：必须真的分配了伪终端（能 resize 就是存在 pty master，管道会话做不到）
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_mode_allocates_real_tty() {
    let sink = sink();
    let state = make_state("pty");
    let cmd = if cfg!(windows) {
        "if ([Console]::IsInputRedirected) { 'redirected' } else { 'tty' }; Start-Sleep -Seconds 20"
    } else {
        "tty; sleep 20"
    };
    let mut s = spec("t-pty", cmd, None);
    s.term_mode = "pty".into();
    supervisor::start(sink.clone(), state.clone(), s).await.unwrap();

    // 醒来后窗口尺寸能被改写 → 说明 master 句柄存在，即真 pty
    let mut ok = false;
    for _ in 0..40 {
        if supervisor::resize_pty(&state, "t-pty", 100, 40).is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ok, "PTY 会话应当能 resize（说明分配了伪终端）");

    // 管道会话没有 pty，resize 必须报错
    let plain = spec("t-pipe", if cfg!(windows) { "Start-Sleep -Seconds 20" } else { "sleep 20" }, None);
    supervisor::start(sink.clone(), state.clone(), plain).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        supervisor::resize_pty(&state, "t-pipe", 100, 40).is_err(),
        "管道会话不该有伪终端"
    );

    let _ = supervisor::stop(&sink, &state, "t-pty").await;
    let _ = supervisor::stop(&sink, &state, "t-pipe").await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    // unix 上再直接确认子进程看到的是 tty 设备
    if !cfg!(windows) {
        let text = log_text(&state, "t-pty");
        assert!(text.contains("/dev/"), "应输出 pty 设备名：{text}");
    }
}

/// 标准输入关闭：读 stdin 的命令必须立刻拿到 EOF，而不是一直等
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn null_stdin_returns_eof_immediately() {
    let sink = sink();
    let state = make_state("stdin-null");
    let cmd = if cfg!(windows) {
        "$x = [Console]::In.ReadLine(); Write-Output \"got:$x\""
    } else {
        "read line; echo \"got:$line\""
    };
    let mut s = spec("t-eof", cmd, None);
    s.stdin_mode = "null".into();
    supervisor::start(sink, state.clone(), s).await.unwrap();

    assert!(
        wait_status(&state, "t-eof", Status::Stopped, 8000).await,
        "关闭 stdin 后命令应立即读到 EOF 并结束"
    );
    let text = log_text(&state, "t-eof");
    assert!(text.contains("got:"), "读到 EOF 后应继续执行：{text}");
}

/// 命中已知报错时，终端里要能看到那条可操作提示
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn known_cli_failure_gets_actionable_hint() {
    let sink = sink();
    let state = make_state("hint");
    let cmd = if cfg!(windows) {
        "Write-Error 'Error: Input must be provided either through stdin or as a prompt argument when using --print'; exit 1"
    } else {
        "echo 'Error: Input must be provided either through stdin or as a prompt argument when using --print' 1>&2; exit 1"
    };
    let s = spec("t-hint", cmd, None);
    supervisor::start(sink, state.clone(), s).await.unwrap();
    assert!(wait_status(&state, "t-hint", Status::Failed, 12000).await);

    let text = log_text(&state, "t-hint");
    assert!(text.contains("提示："), "应追加可操作提示：{text}");
    assert!(text.contains("打开外部终端"), "提示里要给出出路：{text}");
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
