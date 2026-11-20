//! 组件配置的导入 / 导出。
//!
//! 导出文件结构：`{ app, kind, version, exportedAt, components: [...] }`；
//! 导入兼容三种形态：上面的导出文件、组件数组、单个组件对象。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::model::{Component, Environment};
use crate::state::AppState;
use crate::store;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    app: &'static str,
    kind: &'static str,
    version: u32,
    exported_at: u64,
    components: Vec<&'a Component>,
    /// 只带被导出组件引用到的环境，保证换机器导入后变量不丢
    environments: Vec<&'a Environment>,
}

/// 写出 JSON 文件；ids 为空表示导出全部组件
pub fn export_to(state: &Arc<AppState>, ids: &[String], path: &str) -> Result<usize, String> {
    let (text, count) = {
        let data = state.data.lock().unwrap();
        let picked: Vec<&Component> = if ids.is_empty() {
            data.components.iter().collect()
        } else {
            data.components.iter().filter(|c| ids.contains(&c.id)).collect()
        };
        if picked.is_empty() {
            return Err("没有可导出的组件".into());
        }
        let n = picked.len();
        let used: std::collections::BTreeSet<String> = picked
            .iter()
            .filter_map(|c| c.env_profile.clone())
            .collect();
        let environments: Vec<&Environment> = data
            .environments
            .iter()
            .filter(|e| used.contains(&e.id))
            .collect();
        let payload = ExportFile {
            app: "CommandKit",
            kind: "components",
            version: 1,
            exported_at: store::now_ms(),
            components: picked,
            environments,
        };
        let text = serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化失败：{e}"))?;
        (text, n)
    };

    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
        }
    }
    std::fs::write(path, &text).map_err(|e| format!("写入失败：{e}"))?;
    Ok(count)
}

/// 从文件导入，返回 (成功数, 跳过的名称)
pub fn import_from(state: &Arc<AppState>, path: &str) -> Result<(usize, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取失败：{e}"))?;
    import_text(state, &text)
}

pub fn import_text(state: &Arc<AppState>, text: &str) -> Result<(usize, Vec<String>), String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("不是合法 JSON：{e}"))?;

    let raw_list: Vec<serde_json::Value> =
        if let Some(list) = value.get("components").and_then(|v| v.as_array()) {
            list.clone()
        } else if let Some(list) = value.as_array() {
            list.clone()
        } else if value.get("id").is_some() || value.get("name").is_some() {
            vec![value.clone()]
        } else {
            return Err("无法识别的配置：需要 {components:[…]} / 数组 / 单个组件对象".into());
        };

    let mut imported = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let env_list: Vec<serde_json::Value> = value
        .get("environments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    {
        let mut data = state.data.lock().unwrap();

        // 环境先落地：id 已存在就复用，否则新增并记录映射
        let mut env_map: HashMap<String, String> = HashMap::new();
        for item in env_list {
            let Ok(mut env) = serde_json::from_value::<Environment>(item) else {
                continue;
            };
            env.env.retain(|k, _| !k.trim().is_empty());
            if env.env.is_empty() {
                continue;
            }
            let old = env.id.clone();
            if let Some(existing) = data.environments.iter().find(|e| e.id == old) {
                env_map.insert(old, existing.id.clone());
                continue;
            }
            if old.trim().is_empty() || data.environments.iter().any(|e| e.id == old) {
                env.id = format!("env{}{}", store::now_ms(), data.environments.len());
            }
            env_map.insert(old, env.id.clone());
            data.environments.push(env);
        }

        // 先解析，拿到 oldId → 新 id 的映射，便于重写 dependsOn
        let mut parsed: Vec<(Option<String>, Component)> = Vec::new();
        for item in raw_list {
            let label = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("(未命名)")
                .to_string();
            match serde_json::from_value::<Component>(item) {
                Ok(c) => parsed.push((Some(c.id.clone()).filter(|x| !x.is_empty()), c)),
                Err(_) => skipped.push(label),
            }
        }

        let project_ids: Vec<String> = data.projects.iter().map(|p| p.id.clone()).collect();
        let mut id_map: HashMap<String, String> = HashMap::new();
        let base = store::now_ms();

        for (i, (old_id, mut comp)) in parsed.into_iter().enumerate() {
            let candidate = old_id.clone().unwrap_or_default();
            let conflict = candidate.is_empty()
                || data.components.iter().any(|c| c.id == candidate)
                || id_map.values().any(|v| v == &candidate);
            let new_id = if conflict {
                format!("{base}{i}")
            } else {
                candidate.clone()
            };
            if let Some(old) = old_id {
                id_map.insert(old, new_id.clone());
            }
            comp.id = new_id;

            if !project_ids.contains(&comp.project_id) {
                comp.project_id = "all".into();
            }
            // 环境引用：优先按导入映射改写；文件里没带的，如果本机已有同 id 就沿用
            comp.env_profile = match comp.env_profile.clone() {
                Some(p) => env_map
                    .get(&p)
                    .cloned()
                    .or_else(|| {
                        data.environments
                            .iter()
                            .any(|e| e.id == p)
                            .then_some(p.clone())
                    }),
                None => None,
            };
            comp.commands.retain(|s| !s.cmd.trim().is_empty());
            comp.name = comp.name.trim().to_string();
            if comp.name.is_empty() || comp.commands.is_empty() {
                skipped.push(if comp.name.is_empty() {
                    "(未命名)".to_string()
                } else {
                    comp.name.clone()
                });
                continue;
            }
            // 同项目下重名 → 加后缀，避免看起来是重复项
            if data
                .components
                .iter()
                .any(|c| c.name == comp.name && c.project_id == comp.project_id)
            {
                comp.name = format!("{} (导入)", comp.name);
            }
            comp.ports.sort_unstable();
            comp.ports.dedup();
            comp.ports.retain(|p| *p > 0);
            comp.env.retain(|k, _| !k.trim().is_empty());
            if comp.workdir.trim().is_empty() {
                comp.workdir = store::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .display()
                    .to_string();
            }
            // 跨文件依赖先清空，随后按 id_map 重写
            comp.depends_on = comp
                .depends_on
                .iter()
                .filter_map(|d| id_map.get(d).cloned())
                .collect();

            data.components.push(comp);
            imported += 1;
        }

        // 已存在组件对新导入组件的引用：把旧 id 换成新 id，指向不存在的丢掉
        let valid: Vec<String> = data.components.iter().map(|c| c.id.clone()).collect();
        for comp in data.components.iter_mut() {
            let mut deps: Vec<String> = comp
                .depends_on
                .iter()
                .map(|d| id_map.get(d).cloned().unwrap_or_else(|| d.clone()))
                .filter(|d| valid.contains(d) && d != &comp.id)
                .collect();
            deps.sort();
            deps.dedup();
            comp.depends_on = deps;
        }

        if !env_map.is_empty() {
            data.save_environments()
                .map_err(|e| format!("保存 environments.json 失败：{e}"))?;
        }
        if imported > 0 {
            data.save_projects()
                .map_err(|e| format!("保存 projects.json 失败：{e}"))?;
        }
    }

    Ok((imported, skipped))
}
