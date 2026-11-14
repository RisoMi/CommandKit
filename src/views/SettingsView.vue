<script setup lang="ts">
import { reactive, watch } from 'vue';
import { api } from '../api';
import { apply, state, toast } from '../store';
import type { Settings } from '../types';

const form = reactive<Settings>({
  shell: '',
  logLinesCap: 3000,
  confirmBulk: true,
  refreshMs: 2000,
  persistLogs: true,
  sidebarCollapsed: false,
});

watch(
  () => state.settings,
  (s) => {
    if (s) Object.assign(form, s);
  },
  { immediate: true, deep: true }
);

async function save() {
  try {
    const snap = await api.saveSettings({ ...form });
    apply(snap);
    toast('ok', '设置已保存');
  } catch (e) {
    toast('error', String(e));
  }
}

async function exportSnapshot() {
  try {
    toast('ok', `快照：${await api.exportSnapshot()}`);
  } catch (e) {
    toast('error', String(e));
  }
}
</script>

<template>
  <div class="view">
    <header class="head">
      <div>
        <h2>设置</h2>
        <p>数据目录、默认 shell 与日志策略。所有配置都在本地，不上传。</p>
      </div>
      <div class="actions">
        <button class="btn" @click="api.openDataDir()">打开数据目录</button>
        <button class="btn" @click="exportSnapshot">导出进程快照</button>
        <button class="btn primary" @click="save">保存设置</button>
      </div>
    </header>

    <div class="card">
      <div class="card-h"><h3>数据与便携性</h3></div>
      <div class="kv">
        <span class="k">数据目录</span>
        <span class="v selectable">{{ state.info?.dataRoot }}</span>
      </div>
      <div class="kv">
        <span class="k">模式</span>
        <span class="v plain">
          {{ state.info?.portable ? 'Windows 便携版（exe 同目录 data\\，整目录拷贝即迁移）' : 'macOS 常规版（~/Library/Application Support/CommandKit）' }}
        </span>
      </div>
      <div class="kv">
        <span class="k">可写</span>
        <span class="v plain" :style="{ color: state.info?.writable ? 'var(--accent)' : 'var(--err)' }">
          {{ state.info?.writable ? '是' : '否 —— 请移动到有写权限的目录' }}
        </span>
      </div>
      <div class="kv">
        <span class="k">文件</span>
        <span class="v">projects.json · templates.json · settings.json · backup\ · logs\</span>
      </div>
      <div class="kv">
        <span class="k">写入策略</span>
        <span class="v plain">原子写（tmp → fsync → rename）· 覆盖前备份最近 10 版 · 单实例 + 文件锁</span>
      </div>
    </div>

    <div class="card">
      <div class="card-h"><h3>运行</h3></div>
      <div class="body">
        <div class="field" style="flex: 1">
          <label>默认 shell</label>
          <input v-model="form.shell" placeholder="/bin/zsh 或 pwsh.exe" />
          <span class="hint">
            每个项目 / 模板在该 shell 下启动独立子进程：{{ state.info?.platform === 'windows' ? 'pwsh -NoProfile -Command' : 'zsh -lc' }}。
          </span>
        </div>
        <div class="field" style="flex: 0 0 160px">
          <label>日志保留行数</label>
          <input v-model.number="form.logLinesCap" type="number" min="300" max="20000" />
        </div>
        <div class="field" style="flex: 0 0 160px">
          <label>指标刷新（ms）</label>
          <input v-model.number="form.refreshMs" type="number" min="500" max="10000" step="500" />
        </div>
      </div>
      <div class="body" style="padding-top: 0">
        <label class="check"><input v-model="form.persistLogs" type="checkbox" /> 终端输出落盘到 data\logs\</label>
        <label class="check"><input v-model="form.confirmBulk" type="checkbox" /> 一键停止 / 重启二次确认</label>
      </div>
    </div>

    <div class="card">
      <div class="card-h"><h3>关于</h3></div>
      <div class="kv"><span class="k">版本</span><span class="v">CommandKit {{ state.info?.version }}</span></div>
      <div class="kv"><span class="k">运行壳</span><span class="v">Tauri 2 · Rust 进程监管 · {{ state.info?.webview }}</span></div>
      <div class="kv"><span class="k">平台</span><span class="v">{{ state.info?.platform }}</span></div>
      <div class="kv">
        <span class="k">进程终止</span>
        <span class="v plain">
          {{ state.info?.platform === 'windows' ? 'taskkill /T /F（递归整棵进程树）' : 'kill(-pgid) · TERM → 2s → KILL' }}
        </span>
      </div>
      <div class="kv">
        <span class="k">环境隔离</span>
        <span class="v plain">env 由 Rust 侧构造后随 spawn 注入，全程不修改系统环境变量与 shell profile。</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.body {
  display: flex;
  gap: 12px;
  padding: 12px 14px;
  flex-wrap: wrap;
  align-items: flex-start;
}
</style>
