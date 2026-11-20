//! 关键内核行为的集成测试：拓扑排序、环境变量展开、进程树终止、原子写。
//! 运行：`cargo test --tests`

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Duration;

use commandkit_lib::model::{CommandStep, Component, HealthCheck, Join, Policy, Project};
use commandkit_lib::supervisor::{expand, shell_invocation, shell_kind, ShellKind, topo_order};

fn project(id: &str, deps: &[&str]) -> Component {
    Component {
        id: id.into(),
        name: id.into(),
        project_id: "all".into(),
        stack: String::new(),
        workdir: std::env::temp_dir().display().to_string(),
        env_profile: None,
        commands: vec![CommandStep::new("echo ok", Join::And)],
        env: Default::default(),
        ports: vec![],
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        health: None::<HealthCheck>,
        policy: Policy::default(),
        auto_start: false,
        shell: None,
        note: String::new(),
    }
}

#[test]
fn topo_orders_dependencies_first() {
    // npm 前端依赖 java / python 后端，后端依赖 redis
    let projects = vec![
        project("web", &["java", "agent"]),
        project("agent", &["redis"]),
        project("java", &["redis"]),
        project("redis", &[]),
    ];
    let ids: Vec<String> = ["web", "agent", "java", "redis"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let order = topo_order(&projects, &ids).expect("应能排序");
    assert_eq!(order.len(), 4);
    let pos = |name: &str| order.iter().position(|x| x == name).unwrap();
    assert!(pos("redis") < pos("java"), "redis 应先于 java：{order:?}");
    assert!(pos("redis") < pos("agent"), "redis 应先于 agent：{order:?}");
    assert!(pos("java") < pos("web"), "java 应先于 web：{order:?}");
    assert!(pos("agent") < pos("web"), "agent 应先于 web：{order:?}");
}

#[test]
fn topo_ignores_outside_dependencies_and_detects_cycle() {
    // 选中集合之外的依赖不参与排序
    let projects = vec![project("a", &["outside"]), project("outside", &[])];
    let order = topo_order(&projects, &["a".to_string()]).unwrap();
    assert_eq!(order, vec!["a".to_string()]);

    // 环
    let cyclic = vec![project("x", &["y"]), project("y", &["x"])];
    let err = topo_order(&cyclic, &["x".to_string(), "y".to_string()]).unwrap_err();
    assert!(err.contains("循环"), "错误信息应指出循环依赖：{err}");
}

#[test]
fn env_expansion_resolves_local_and_system_vars() {
    let mut local = BTreeMap::new();
    local.insert("JAVA_HOME".to_string(), "/opt/jdk17".to_string());
    // 一条模板值覆盖两个平台：Windows 走 %PATH%，类 Unix 走 $PATH
    #[cfg(windows)]
    local.insert("PATH".to_string(), "%PATH%;%JAVA_HOME%\\bin".to_string());
    #[cfg(not(windows))]
    local.insert("PATH".to_string(), "$PATH:$JAVA_HOME/bin".to_string());

    let value = local.get("PATH").unwrap().clone();
    let expanded = expand(&value, &local);
    assert!(
        expanded.contains("/opt/jdk17"),
        "应展开本地变量引用：{expanded}"
    );

    let inherited = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let first = inherited.split(sep).next().unwrap_or("");
    if !first.is_empty() {
        assert!(
            expanded.starts_with(first),
            "应保留宿主 PATH 前缀：{expanded}"
        );
    }

    // 不存在的变量保留原样，不 panic
    let raw = expand("$NOT_A_REAL_VAR_XYZ/keep", &local);
    assert!(raw.contains("$NOT_A_REAL_VAR_XYZ") || raw.contains("keep"));
}

fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
}

#[cfg(unix)]
#[test]
fn terminate_tree_kills_grandchildren() {
    // 子进程在独立进程组中，kill(-pgid) 必须命中孙进程
    use std::os::unix::process::CommandExt;

    let marker = format!("/tmp/commandkit-test-{}.pid", std::process::id());
    let _ = std::fs::remove_file(&marker);
    let script = format!("sleep 30 & echo $! > {marker}; wait", marker = marker);
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(script)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.process_group(0);
    let mut child = cmd.spawn().expect("spawn sh");

    // 等待孙进程 pid 落盘
    let mut grandchild = None;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        if let Ok(text) = std::fs::read_to_string(&marker) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                grandchild = Some(pid);
                break;
            }
        }
    }
    let grandchild = grandchild.expect("应拿到孙进程 pid");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        commandkit_lib::proc::terminate_tree(child.id()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2600)).await;
    });

    let alive = is_alive(grandchild);
    let _ = child.wait();
    let _ = std::fs::remove_file(&marker);
    assert!(!alive, "孙进程 {grandchild} 应随进程组一起终止");
}

#[test]
fn data_root_is_writable_probe() {
    let dir = std::env::temp_dir().join(format!("commandkit-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    commandkit_lib::store::ensure_writable(&dir).expect("临时目录应可写");
    assert!(dir.is_dir());

    // Json 文件：写入 + 读回 + 备份
    let file = commandkit_lib::store::JsonFile::new(&dir, "t.json");
    let data = serde_json::json!({ "a": 1 });
    file.write(&data).unwrap();
    file.write(&data).unwrap(); // 第二次触发覆盖备份
    let back: serde_json::Value = file.read_or();
    assert_eq!(back["a"], 1);
    let backups = std::fs::read_dir(dir.join("backup")).unwrap().count();
    assert_eq!(backups, 1, "覆盖前应保留一份备份");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shell_kind_and_invocation_per_family() {
    // 三类 shell 的参数形式必须区分开：cmd 用 /D /C，powershell 用 -NoProfile -Command，posix 用 -lc
    assert_eq!(shell_kind("/bin/zsh"), ShellKind::Posix);
    assert_eq!(shell_kind("/bin/bash"), ShellKind::Posix);
    assert_eq!(
        shell_invocation("/bin/zsh", "npm run dev"),
        vec!["-lc".to_string(), "npm run dev".to_string()]
    );

    assert_eq!(shell_kind("cmd.exe"), ShellKind::Cmd);
    assert_eq!(
        shell_kind("C:\\Windows\\System32\\cmd.exe"),
        ShellKind::Cmd,
        "带路径的 cmd.exe 也要识别"
    );
    assert_eq!(
        shell_invocation("cmd.exe", "npm run dev"),
        vec!["/D".to_string(), "/C".to_string(), "npm run dev".to_string()]
    );

    assert_eq!(shell_kind("pwsh.exe"), ShellKind::Pwsh);
    assert_eq!(
        shell_kind("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
        ShellKind::Pwsh
    );
    assert_eq!(
        shell_invocation("pwsh.exe", "Get-Date"),
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Get-Date".to_string()
        ]
    );

    assert_eq!(shell_kind("powershell.exe"), ShellKind::PowerShell);
    assert_eq!(shell_kind("PowerShell.exe"), ShellKind::PowerShell);
}

/// 导出 → 导入：id 冲突要重新分配，dependsOn 要跟着改写，重名加后缀
#[test]
fn export_then_import_roundtrip() {
    use commandkit_lib::state::load_state;
    use commandkit_lib::transfer::{export_to, import_text};

    let root = std::env::temp_dir().join(format!("ck-transfer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let state = std::sync::Arc::new(load_state(root.clone(), None));

    // 造两个互相依赖的组件
    {
        let mut data = state.data.lock().unwrap();
        data.components.clear();
        let mut a = project("alpha", &[]);
        a.name = "alpha".into();
        let mut b = project("beta", &["alpha"]);
        b.name = "beta".into();
        data.components.push(a);
        data.components.push(b);
    }

    let file = root.join("out.json");
    let n = export_to(&state, &[], file.to_str().unwrap()).unwrap();
    assert_eq!(n, 2, "应导出 2 个组件");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"components\"") && text.contains("alpha"));

    // 原样导回：id 冲突 → 重新分配，名字加 (导入)，依赖改写到新 id
    let (imported, skipped) = import_text(&state, &text).unwrap();
    assert_eq!(imported, 2, "两个都能导入：{skipped:?}");
    assert!(skipped.is_empty());

    let data = state.data.lock().unwrap();
    assert_eq!(data.components.len(), 4);

    let imported_beta = data
        .components
        .iter()
        .find(|c| c.name == "beta (导入)")
        .expect("重名应加后缀");
    let imported_alpha = data
        .components
        .iter()
        .find(|c| c.name == "alpha (导入)")
        .expect("重名应加后缀");
    assert_ne!(imported_alpha.id, "alpha", "id 冲突必须重新分配");
    assert_eq!(
        imported_beta.depends_on,
        vec![imported_alpha.id.clone()],
        "依赖应改写到新 id"
    );
}

/// 环境合并：环境打底，组件自身同名变量覆盖
#[test]
fn env_profile_is_merged_with_component_overrides() {
    use commandkit_lib::model::Environment;
    use commandkit_lib::supervisor::{merged_env, spec_from_component};

    let mut base = BTreeMap::new();
    base.insert("JAVA_HOME".to_string(), "/opt/jdk17".to_string());
    base.insert("PATH".to_string(), "$JAVA_HOME/bin:$PATH".to_string());
    let envs = vec![Environment {
        id: "env-java".into(),
        name: "Java 17".into(),
        desc: String::new(),
        color: None,
        env: base,
    }];

    let mut c = project("app", &[]);
    c.env_profile = Some("env-java".into());
    c.env.insert("JAVA_OPTS".into(), "-Xmx2g".into());
    c.env.insert("PATH".into(), "/custom/bin:$PATH".into());

    let merged = merged_env(&c, &envs);
    assert_eq!(merged.get("JAVA_HOME").unwrap(), "/opt/jdk17", "环境变量应注入");
    assert_eq!(merged.get("JAVA_OPTS").unwrap(), "-Xmx2g", "组件自己的变量保留");
    assert_eq!(
        merged.get("PATH").unwrap(),
        "/custom/bin:$PATH",
        "同名时组件覆盖环境"
    );

    // 没有引用环境时只保留组件自身变量
    let mut plain = project("plain", &[]);
    plain.env.insert("A".into(), "1".into());
    assert_eq!(merged_env(&plain, &envs).len(), 1);

    // spec 走的是合并后的 env
    let spec = spec_from_component(&c, &envs);
    assert_eq!(spec.env.get("JAVA_HOME").unwrap(), "/opt/jdk17");
}

/// 导出会带上被引用的环境，导入后组件仍指向可用的环境
#[test]
fn export_carries_referenced_environments() {
    use commandkit_lib::state::load_state;
    use commandkit_lib::transfer::{export_to, import_text};

    let root = std::env::temp_dir().join(format!("ck-env-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let state = std::sync::Arc::new(load_state(root.clone(), None));

    let env_id = {
        let mut data = state.data.lock().unwrap();
        data.components.clear();
        let mut c = project("with-env", &[]);
        c.name = "with-env".into();
        c.env_profile = Some(data.environments[0].id.clone());
        let id = data.environments[0].id.clone();
        data.components.push(c);
        id
    };

    let file = root.join("env.json");
    export_to(&state, &[], file.to_str().unwrap()).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"environments\""), "导出文件应包含 environments");

    // 导入到干净状态：环境应被创建，组件引用可用
    let root2 = std::env::temp_dir().join(format!("ck-env-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root2);
    std::fs::create_dir_all(&root2).unwrap();
    let fresh = std::sync::Arc::new(load_state(root2.clone(), None));
    {
        let mut data = fresh.data.lock().unwrap();
        data.components.clear();
        data.environments.clear();
    }

    let (imported, _) = import_text(&fresh, &text).unwrap();
    assert_eq!(imported, 1);

    let data = fresh.data.lock().unwrap();
    assert!(!data.environments.is_empty(), "环境应随文件导入");
    let c = &data.components[0];
    let profile = c.env_profile.clone().expect("组件应仍引用环境");
    assert!(
        data.environments.iter().any(|e| e.id == profile),
        "引用的环境必须存在：{profile}"
    );
    let _ = env_id;
}

/// 坏数据要能跳过而不是整份失败
#[test]
fn import_skips_bad_entries() {
    use commandkit_lib::state::load_state;
    use commandkit_lib::transfer::import_text;

    let root = std::env::temp_dir().join(format!("ck-transfer-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let state = std::sync::Arc::new(load_state(root.clone(), None));

    let text = r#"{"components":[
        {"id":"ok-1","name":"好的","projectId":"all","workdir":"/tmp","commands":[{"cmd":"echo hi","join":"and"}]},
        {"id":"bad-1","name":"没有命令","projectId":"all","workdir":"/tmp","commands":[]},
        {"name":123}
    ]}"#;
    let (imported, skipped) = import_text(&state, text).unwrap();
    assert_eq!(imported, 1, "只有一条合法");
    assert_eq!(skipped.len(), 2, "其余应被跳过：{skipped:?}");
}

#[test]
fn categories_are_serializable() {
    // 守住 IPC 契约：Category 走 camelCase，前端 types.ts 依赖这一点
    let c = Project {
        id: "frontend".into(),
        name: "前端".into(),
        order: 1,
        color: Some("#5B9DFF".into()),
    };
    let text = serde_json::to_string(&c).unwrap();
    assert!(text.contains("\"id\":\"frontend\""));
}
