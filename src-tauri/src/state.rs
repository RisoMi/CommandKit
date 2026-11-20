//! 全局状态：数据存储 + 会话表 + 批量任务 + 指标快照 + 单实例锁。

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::model::*;
use crate::store::{self, JsonFile};
use crate::supervisor::Session;

pub struct DataStore {
    pub projects_file: JsonFile,
    pub environments_file: JsonFile,
    pub templates_file: JsonFile,
    pub settings_file: JsonFile,
    pub snapshot_file: JsonFile,
    /// 项目（组件分组）
    pub projects: Vec<Project>,
    /// 组件（可启动单元）
    pub components: Vec<Component>,
    pub environments: Vec<Environment>,
    pub templates: Vec<Template>,
    pub settings: Settings,
}

impl DataStore {
    /// 项目与组件同存 projects.json：整目录迁移时只有一个真相源
    pub fn save_projects(&self) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct Bundle<'a> {
            version: u32,
            projects: &'a Vec<Project>,
            components: &'a Vec<Component>,
        }
        self.projects_file.write(&Bundle {
            version: 2,
            projects: &self.projects,
            components: &self.components,
        })
    }
    pub fn save_environments(&self) -> std::io::Result<()> {
        self.environments_file.write(&self.environments)
    }
    pub fn save_templates(&self) -> std::io::Result<()> {
        self.templates_file.write(&self.templates)
    }
    pub fn save_settings(&self) -> std::io::Result<()> {
        self.settings_file.write(&self.settings)
    }
}

pub struct AppState {
    pub root: PathBuf,
    pub writable: bool,
    pub data: Mutex<DataStore>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub bulk: Mutex<Option<BulkView>>,
    pub metrics: Mutex<HashMap<String, MetricRow>>,
    /// 持有到进程退出，释放文件锁
    pub _lock: Mutex<Option<File>>,
}

pub fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// v1 的组件结构（当时叫 project，字段叫 category / command）
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LegacyComponent {
    id: String,
    name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    stack: String,
    #[serde(default)]
    workdir: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    ports: Vec<u16>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    health: Option<HealthCheck>,
    #[serde(default)]
    policy: Policy,
    #[serde(default)]
    auto_start: bool,
    #[serde(default)]
    shell: Option<String>,
    #[serde(default)]
    note: String,
}

impl LegacyComponent {
    fn into_component(self) -> Component {
        Component {
            id: self.id,
            name: self.name,
            project_id: if self.category.is_empty() {
                "all".to_string()
            } else {
                self.category
            },
            stack: self.stack,
            workdir: self.workdir,
            env_profile: None,
            commands: if self.command.trim().is_empty() {
                Vec::new()
            } else {
                vec![CommandStep::new(&self.command, Join::And)]
            },
            env: self.env,
            ports: self.ports,
            depends_on: self.depends_on,
            health: self.health,
            policy: self.policy,
            auto_start: self.auto_start,
            shell: self.shell,
            note: self.note,
        }
    }
}

/// 读 projects.json：v2 直接解析，v1（categories + projects）就地迁移
fn parse_bundle(raw: &serde_json::Value) -> (Vec<Project>, Vec<Component>) {
    let version = raw.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
    let take = |key: &str| raw.get(key).cloned().unwrap_or(serde_json::Value::Array(vec![]));
    if version >= 2 {
        let projects: Vec<Project> = serde_json::from_value(take("projects")).unwrap_or_default();
        let components: Vec<Component> = serde_json::from_value(take("components")).unwrap_or_default();
        (projects, components)
    } else {
        let projects: Vec<Project> = serde_json::from_value(take("categories")).unwrap_or_default();
        let legacy: Vec<LegacyComponent> = serde_json::from_value(take("projects")).unwrap_or_default();
        (projects, legacy.into_iter().map(|l| l.into_component()).collect())
    }
}

pub fn load_state(root: PathBuf, lock: Option<File>) -> AppState {
    let _ = std::fs::create_dir_all(root.join("logs"));
    let _ = std::fs::create_dir_all(root.join("backup"));

    let projects_file = JsonFile::new(&root, "projects.json");
    let environments_file = JsonFile::new(&root, "environments.json");
    let templates_file = JsonFile::new(&root, "templates.json");
    let settings_file = JsonFile::new(&root, "settings.json");
    let snapshot_file = JsonFile::new(&root, "process-snapshot.json");

    let raw: serde_json::Value = projects_file.read_or();
    let settings: Settings = settings_file.read_or();

    let (mut projects, mut components) = parse_bundle(&raw);
    if projects.is_empty() {
        projects = seed_projects();
    }
    let first_run = !projects_file.path().exists() && components.is_empty();
    if first_run {
        components = seed_components(&root);
    }
    let migrated = raw.get("version").and_then(|v| v.as_u64()).unwrap_or(1) < 2 && !components.is_empty();

    let environments: Vec<Environment> = environments_file.read_or();
    let environments = if environments.is_empty() {
        seed_environments()
    } else {
        environments
    };

    let templates: Vec<Template> = templates_file.read_or();
    let templates = if templates.is_empty() {
        seed_templates()
    } else {
        templates
    };

    let store = DataStore {
        projects_file,
        environments_file,
        templates_file,
        settings_file,
        snapshot_file,
        projects,
        components,
        environments,
        templates,
        settings,
    };
    if first_run || migrated {
        let _ = store.save_projects();
        let _ = store.save_environments();
        let _ = store.save_templates();
        let _ = store.save_settings();
    }

    AppState {
        root,
        writable: true,
        data: Mutex::new(store),
        sessions: Mutex::new(HashMap::new()),
        bulk: Mutex::new(None),
        metrics: Mutex::new(HashMap::new()),
        _lock: Mutex::new(lock),
    }
}

fn seed_projects() -> Vec<Project> {
    vec![
        Project {
            id: "all".into(),
            name: "全部项目".into(),
            order: 0,
            color: None,
        },
        Project {
            id: "frontend".into(),
            name: "前端".into(),
            order: 1,
            color: Some("#5B9DFF".into()),
        },
        Project {
            id: "backend".into(),
            name: "后端".into(),
            order: 2,
            color: Some("#9EFF00".into()),
        },
        Project {
            id: "middleware".into(),
            name: "中间件".into(),
            order: 3,
            color: Some("#E8A33D".into()),
        },
        Project {
            id: "data".into(),
            name: "数据服务".into(),
            order: 4,
            color: Some("#B27BFF".into()),
        },
    ]
}

fn seed_components(root: &std::path::Path) -> Vec<Component> {
    let home = store::home_dir().unwrap_or_else(|| root.to_path_buf());
    let home_str = home.display().to_string();
    let (tick_cmd, server_cmd) = if cfg!(target_os = "windows") {
        (
            "1..20 | ForEach-Object { \"tick $_ $(Get-Date -Format HH:mm:ss)\"; Start-Sleep -Milliseconds 900 }".to_string(),
            "py -m http.server 8899".to_string(),
        )
    } else {
        (
            "for i in $(seq 1 20); do echo \"tick $i $(date +%H:%M:%S)\"; sleep 0.9; done".to_string(),
            "python3 -m http.server 8899".to_string(),
        )
    };
    vec![
        Component {
            id: "demo-envcheck".into(),
            name: "环境自检".into(),
            project_id: "frontend".into(),
            stack: if cfg!(windows) { "PowerShell 7" } else { "zsh" }.into(),
            workdir: home_str.clone(),
            env_profile: None,
            commands: vec![CommandStep::new(
                if cfg!(windows) {
                    "node -v; npm -v; $PSVersionTable.PSVersion"
                } else {
                    "node -v; npm -v; sw_vers -productVersion"
                },
                Join::And,
            )],
            env: Default::default(),
            ports: vec![],
            depends_on: vec![],
            health: None,
            policy: Policy::default(),
            auto_start: false,
            shell: None,
            note: "示例项目：验证子 shell、环境注入与退出码回传。".into(),
        },
        Component {
            id: "demo-server".into(),
            name: "本地静态服务".into(),
            project_id: "backend".into(),
            stack: if cfg!(windows) { "Python" } else { "Python 3" }.into(),
            workdir: home_str.clone(),
            env_profile: None,
            commands: vec![CommandStep::new(&server_cmd, Join::And)],
            env: Default::default(),
            ports: vec![8899],
            depends_on: vec![],
            health: Some(HealthCheck {
                url: None,
                port: Some(8899),
                ..Default::default()
            }),
            policy: Policy::default(),
            auto_start: false,
            shell: None,
            note: "示例项目：端口健康检查通过后才置为运行中。".into(),
        },
        Component {
            id: "demo-worker".into(),
            name: "日志流演示".into(),
            project_id: "middleware".into(),
            stack: "shell".into(),
            workdir: home_str,
            env_profile: None,
            commands: vec![CommandStep::new(&tick_cmd, Join::And)],
            env: Default::default(),
            ports: vec![],
            depends_on: vec![],
            health: None,
            policy: Policy::default(),
            auto_start: false,
            shell: None,
            note: "示例项目：20 行输出后正常退出，用于观察状态机流转。".into(),
        },
    ]
}

/// 内置几个常见环境，用户可以直接拿来用或改
fn seed_environments() -> Vec<Environment> {
    let mut java = std::collections::BTreeMap::new();
    let mut py = std::collections::BTreeMap::new();
    let mut node = std::collections::BTreeMap::new();
    if cfg!(windows) {
        java.insert("JAVA_HOME".to_string(), "C:\\Program Files\\Java\\jdk-17".to_string());
        java.insert("PATH".to_string(), "%JAVA_HOME%\\bin;%PATH%".to_string());
        py.insert("PYTHONUTF8".to_string(), "1".to_string());
        py.insert("PYTHONIOENCODING".to_string(), "utf-8".to_string());
        node.insert("NODE_OPTIONS".to_string(), "--max-old-space-size=4096".to_string());
    } else {
        java.insert("JAVA_HOME".to_string(), "/Library/Java/JavaVirtualMachines/jdk-17.jdk/Contents/Home".to_string());
        java.insert("PATH".to_string(), "$JAVA_HOME/bin:$PATH".to_string());
        py.insert("PYTHONUTF8".to_string(), "1".to_string());
        py.insert("PYTHONIOENCODING".to_string(), "utf-8".to_string());
        node.insert("NODE_OPTIONS".to_string(), "--max-old-space-size=4096".to_string());
    }

    vec![
        Environment {
            id: "env-java17".into(),
            name: "Java 17".into(),
            desc: "JDK 17 + JAVA_HOME 注入".into(),
            color: Some("#E8A33D".into()),
            env: java,
        },
        Environment {
            id: "env-python".into(),
            name: "Python UTF-8".into(),
            desc: "强制 UTF-8 输出，避免中文乱码".into(),
            color: Some("#5B9DFF".into()),
            env: py,
        },
        Environment {
            id: "env-node".into(),
            name: "Node 大内存".into(),
            desc: "前端构建时放宽堆上限".into(),
            color: Some("#9EFF00".into()),
            env: node,
        },
    ]
}

fn seed_templates() -> Vec<Template> {
    let now = store::now_ms();
    let win = cfg!(target_os = "windows");
    let mut env = std::collections::BTreeMap::new();
    let mut commands = vec![];
    if win {
        env.insert("JAVA_HOME".to_string(), "C:\\Program Files\\Java\\jdk-17".to_string());
        env.insert("PYTHON_HOME".to_string(), "C:\\Python311".to_string());
        env.insert("MAVEN_HOME".to_string(), "C:\\Tools\\apache-maven-3.9.6".to_string());
        env.insert(
            "PATH".to_string(),
            "%PATH%;%JAVA_HOME%\\bin;%MAVEN_HOME%\\bin".to_string(),
        );
        commands = vec![
            TemplateCommand { cmd: "java -version".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: "py -3.11 --version".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: "mvn -v".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: ".\\gradlew.bat clean build".into(), tag: "构建".into() },
        ];
    } else {
        env.insert("JAVA_HOME".to_string(), "/Library/Java/JavaVirtualMachines/jdk-17.jdk/Contents/Home".to_string());
        env.insert("PYENV_ROOT".to_string(), "$HOME/.pyenv".to_string());
        env.insert("MAVEN_HOME".to_string(), "/opt/homebrew/opt/maven".to_string());
        env.insert(
            "PATH".to_string(),
            "$PATH:$JAVA_HOME/bin:$MAVEN_HOME/bin".to_string(),
        );
        commands = vec![
            TemplateCommand { cmd: "java -version".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: "python3 --version".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: "mvn -v".into(), tag: "前置校验".into() },
            TemplateCommand { cmd: "./gradlew clean build".into(), tag: "构建".into() },
        ];
    }
    vec![
        Template {
            id: "tpl-webgis".into(),
            name: "WebGIS 服务启动".into(),
            desc: "为 fishery-agent 准备 JDK 17、Python 与 Maven 构建环境，切目录后按序执行版本校验与构建。"
                .into(),
            platform: if win { "windows".into() } else { "macos".into() },
            shell: None,
            workdir: if win { "D:\\work\\fishery-agent".into() } else { "$HOME/work/fishery-agent".into() },
            env,
            commands,
            updated_at: now,
        },
        Template {
            id: "tpl-node".into(),
            name: "Node 前端构建".into(),
            desc: "锁定 Node 版本、清理缓存后构建，产物输出到 dist。".into(),
            platform: if win { "windows".into() } else { "macos".into() },
            shell: None,
            workdir: if win { "D:\\work\\ocean-fishery-code\\web".into() } else { "$HOME/work/ocean-fishery-code/web".into() },
            env: Default::default(),
            commands: vec![
                TemplateCommand { cmd: "node -v".into(), tag: "校验".into() },
                TemplateCommand { cmd: "npm ci".into(), tag: "依赖".into() },
                TemplateCommand { cmd: "npm run build".into(), tag: "构建".into() },
            ],
            updated_at: now,
        },
    ]
}
