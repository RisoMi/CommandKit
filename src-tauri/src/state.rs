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
    pub templates_file: JsonFile,
    pub settings_file: JsonFile,
    pub snapshot_file: JsonFile,
    pub projects: Vec<Project>,
    pub categories: Vec<Category>,
    pub templates: Vec<Template>,
    pub settings: Settings,
}

impl DataStore {
    /// 分类与项目同存 projects.json：整目录迁移时只有一个真相源
    pub fn save_projects(&self) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct Bundle<'a> {
            version: u32,
            categories: &'a Vec<Category>,
            projects: &'a Vec<Project>,
        }
        self.projects_file.write(&Bundle {
            version: 1,
            categories: &self.categories,
            projects: &self.projects,
        })
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

#[derive(serde::Deserialize, Default)]
struct ProjectBundle {
    #[serde(default)]
    projects: Vec<Project>,
    #[serde(default)]
    categories: Vec<Category>,
}

pub fn load_state(root: PathBuf, lock: Option<File>) -> AppState {
    let _ = std::fs::create_dir_all(root.join("logs"));
    let _ = std::fs::create_dir_all(root.join("backup"));

    let projects_file = JsonFile::new(&root, "projects.json");
    let templates_file = JsonFile::new(&root, "templates.json");
    let settings_file = JsonFile::new(&root, "settings.json");
    let snapshot_file = JsonFile::new(&root, "process-snapshot.json");

    let bundle: ProjectBundle = projects_file.read_or();
    let settings: Settings = settings_file.read_or();

    let mut projects = bundle.projects;
    let mut categories = bundle.categories;
    if categories.is_empty() {
        categories = seed_categories();
    }
    let first_run = !projects_file.path().exists() && projects.is_empty();
    if first_run {
        projects = seed_projects(&root);
    }

    let templates: Vec<Template> = templates_file.read_or();
    let templates = if templates.is_empty() {
        seed_templates()
    } else {
        templates
    };

    let store = DataStore {
        projects_file,
        templates_file,
        settings_file,
        snapshot_file,
        projects,
        categories,
        templates,
        settings,
    };
    if first_run {
        let _ = store.save_projects();
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

fn seed_categories() -> Vec<Category> {
    vec![
        Category {
            id: "all".into(),
            name: "全部项目".into(),
            order: 0,
            color: None,
        },
        Category {
            id: "frontend".into(),
            name: "前端".into(),
            order: 1,
            color: Some("#5B9DFF".into()),
        },
        Category {
            id: "backend".into(),
            name: "后端".into(),
            order: 2,
            color: Some("#9EFF00".into()),
        },
        Category {
            id: "middleware".into(),
            name: "中间件".into(),
            order: 3,
            color: Some("#E8A33D".into()),
        },
        Category {
            id: "data".into(),
            name: "数据服务".into(),
            order: 4,
            color: Some("#B27BFF".into()),
        },
    ]
}

fn seed_projects(root: &std::path::Path) -> Vec<Project> {
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
        Project {
            id: "demo-envcheck".into(),
            name: "环境自检".into(),
            category: "frontend".into(),
            stack: if cfg!(windows) { "PowerShell 7" } else { "zsh" }.into(),
            workdir: home_str.clone(),
            command: if cfg!(windows) {
                "node -v; npm -v; $PSVersionTable.PSVersion".into()
            } else {
                "node -v; npm -v; sw_vers -productVersion".into()
            },
            env: Default::default(),
            ports: vec![],
            depends_on: vec![],
            health: None,
            policy: Policy::default(),
            auto_start: false,
            shell: None,
            note: "示例项目：验证子 shell、环境注入与退出码回传。".into(),
        },
        Project {
            id: "demo-server".into(),
            name: "本地静态服务".into(),
            category: "backend".into(),
            stack: if cfg!(windows) { "Python" } else { "Python 3" }.into(),
            workdir: home_str.clone(),
            command: server_cmd,
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
        Project {
            id: "demo-worker".into(),
            name: "日志流演示".into(),
            category: "middleware".into(),
            stack: "shell".into(),
            workdir: home_str,
            command: tick_cmd,
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
