<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import {
  closeEditor,
  deleteProjectById,
  editor,
  saveProjectFromEditor,
  state,
} from '../store';
import { shellInvocation } from '../format';
import type { Project } from '../types';

const draft = reactive<Project>(JSON.parse(JSON.stringify(editor.project)) as Project);
const envRows = ref<{ k: string; v: string }[]>(
  Object.entries(draft.env ?? {}).map(([k, v]) => ({ k, v }))
);
const portsText = ref((draft.ports ?? []).join(', '));
const healthOn = ref(!!draft.health);
const healthKind = ref<'port' | 'url'>(draft.health?.url ? 'url' : 'port');

const isNew = computed(() => editor.isNew);
const valid = computed(
  () => draft.name.trim() !== '' && draft.command.trim() !== '' && draft.workdir.trim() !== ''
);

const CUSTOM_SHELL = '__custom__';
const shells = computed(() => state.info?.shells ?? []);

const defaultShell = computed(
  () => state.settings?.shell || (state.info?.platform === 'windows' ? 'pwsh.exe' : '/bin/zsh')
);

// '' = 跟随设置；已知值 = 指定 shell；CUSTOM_SHELL = 自定义路径
const shellSel = ref<string>(
  !draft.shell
    ? ''
    : shells.value.some((s) => s.value === draft.shell)
      ? draft.shell
      : CUSTOM_SHELL
);
const customShell = ref<string>(draft.shell && shellSel.value === CUSTOM_SHELL ? draft.shell : '');

const effectiveShell = computed(() => {
  if (shellSel.value === '') return defaultShell.value;
  if (shellSel.value === CUSTOM_SHELL) return customShell.value.trim() || defaultShell.value;
  return shellSel.value;
});

const shellHint = computed(() => shellInvocation(effectiveShell.value, draft.command));

const otherProjects = computed(() => state.projects.filter((p) => p.id !== draft.id));

watch(healthOn, (on) => {
  if (on && !draft.health) {
    draft.health = { url: null, port: draft.ports[0] ?? null, timeoutMs: 60000, retry: 30, intervalMs: 1000 };
  }
});

function addEnv() {
  envRows.value.push({ k: '', v: '' });
}

function removeEnv(i: number) {
  envRows.value.splice(i, 1);
}

function toggleDep(id: string) {
  const i = draft.dependsOn.indexOf(id);
  if (i >= 0) draft.dependsOn.splice(i, 1);
  else draft.dependsOn.push(id);
}

async function submit() {
  if (!valid.value) return;
  const env: Record<string, string> = {};
  for (const row of envRows.value) {
    if (row.k.trim()) env[row.k.trim()] = row.v;
  }
  draft.shell =
    shellSel.value === ''
      ? null
      : shellSel.value === CUSTOM_SHELL
        ? customShell.value.trim() || null
        : shellSel.value;
  draft.env = env;
  draft.ports = portsText.value
    .split(/[,\s]+/)
    .map((x) => parseInt(x.trim(), 10))
    .filter((n) => Number.isFinite(n) && n > 0);
  draft.health = healthOn.value
    ? {
        url: healthKind.value === 'url' ? (draft.health?.url ?? '') : null,
        port: healthKind.value === 'port' ? (draft.health?.port ?? null) : null,
        timeoutMs: draft.health?.timeoutMs ?? 60000,
        retry: draft.health?.retry ?? 30,
        intervalMs: draft.health?.intervalMs ?? 1000,
      }
    : null;
  if (draft.health && healthKind.value === 'url' && !draft.health.url?.trim()) {
    draft.health = null;
  }
  editor.project = draft;
  await saveProjectFromEditor();
}

function drop() {
  if (!draft.id) return;
  const id = draft.id;
  closeEditor();
  deleteProjectById(id);
}
</script>

<template>
  <div class="mask" @click.self="closeEditor">
    <div class="modal">
      <div class="modal-h">
        <h3>{{ isNew ? '新建项目' : `编辑项目 · ${draft.name}` }}</h3>
        <span class="x" @click="closeEditor">✕</span>
      </div>

      <div class="modal-b">
        <div class="row">
          <div class="field" style="flex: 2">
            <label>项目名称 *</label>
            <input v-model="draft.name" placeholder="fishery-agent" />
          </div>
          <div class="field" style="flex: 1">
            <label>分类</label>
            <select v-model="draft.category">
              <option v-for="c in state.categories" :key="c.id" :value="c.id">{{ c.name }}</option>
            </select>
          </div>
          <div class="field" style="flex: 1">
            <label>标签</label>
            <input v-model="draft.stack" placeholder="Python 3.11" />
          </div>
        </div>

        <div class="field">
          <label>工作目录 *</label>
          <input v-model="draft.workdir" placeholder="D:\work\fishery-agent 或 ~/work/fishery-agent" />
          <span class="hint">支持 ~ 展开；目录不存在时启动会直接报错，不会静默回退到 cwd。</span>
        </div>

        <div class="field">
          <label>启动命令 *</label>
          <textarea v-model="draft.command" rows="3" placeholder="py -3.11 -m uvicorn main:app --port 8000" />
          <span class="hint mono">将在独立子 shell 中执行：{{ shellHint }}</span>
        </div>

        <div class="row">
          <div class="field">
            <label>端口（逗号分隔）</label>
            <input v-model="portsText" placeholder="8000, 8001" />
          </div>
          <div class="field">
            <label>执行 shell</label>
            <select v-model="shellSel">
              <option value="">跟随设置（{{ defaultShell }}）</option>
              <option v-for="s in shells" :key="s.value" :value="s.value">
                {{ s.label }}{{ s.available ? '' : '（未安装）' }}
              </option>
              <option :value="CUSTOM_SHELL">自定义路径…</option>
            </select>
            <input
              v-if="shellSel === CUSTOM_SHELL"
              v-model="customShell"
              placeholder="cmd.exe 或 C:\Tools\msys64\usr\bin\bash.exe"
            />
          </div>
        </div>

        <div class="section-t">环境变量注入</div>
        <div v-for="(row, i) in envRows" :key="i" class="row" style="margin-bottom: 6px">
          <input v-model="row.k" style="flex: 0 0 200px" placeholder="KEY" />
          <input v-model="row.v" placeholder="VALUE（支持 %PATH% / $PATH 引用现有环境）" />
          <button class="btn xs" @click="removeEnv(i)">删除</button>
        </div>
        <div><button class="btn xs" @click="addEnv">+ 添加变量</button></div>

        <div class="section-t">依赖（决定一键运行顺序）</div>
        <div class="flex wrap" style="gap: 8px 14px">
          <label v-for="p in otherProjects" :key="p.id" class="check">
            <input type="checkbox" :checked="draft.dependsOn.includes(p.id)" @change="toggleDep(p.id)" />
            {{ p.name }}
          </label>
          <span v-if="!otherProjects.length" class="muted" style="font-size: 11px">暂无其它项目</span>
        </div>

        <div class="section-t">健康检查</div>
        <label class="check"><input v-model="healthOn" type="checkbox" /> 启用健康检查（就绪后才置为运行中）</label>
        <div v-if="healthOn" class="row">
          <div class="field" style="flex: 0 0 130px">
            <label>类型</label>
            <select v-model="healthKind">
              <option value="port">TCP 端口</option>
              <option value="url">HTTP 地址</option>
            </select>
          </div>
          <div v-if="healthKind === 'port'" class="field">
            <label>端口</label>
            <input
              :value="draft.health?.port ?? ''"
              type="number"
              placeholder="8000"
              @input="draft.health = { ...(draft.health ?? { url: null, port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 }), port: Number(($event.target as HTMLInputElement).value) || null }"
            />
          </div>
          <div v-else class="field">
            <label>URL</label>
            <input
              :value="draft.health?.url ?? ''"
              placeholder="http://127.0.0.1:8000/health"
              @input="draft.health = { ...(draft.health ?? { url: null, port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 }), url: ($event.target as HTMLInputElement).value }"
            />
          </div>
        </div>
        <div v-if="healthOn" class="row">
          <div class="field">
            <label>超时 (ms)</label>
            <input
              :value="draft.health?.timeoutMs ?? 60000"
              type="number"
              @input="draft.health = { ...(draft.health ?? { url: null, port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 }), timeoutMs: Number(($event.target as HTMLInputElement).value) || 60000 }"
            />
          </div>
          <div class="field">
            <label>重试次数</label>
            <input
              :value="draft.health?.retry ?? 30"
              type="number"
              @input="draft.health = { ...(draft.health ?? { url: null, port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 }), retry: Number(($event.target as HTMLInputElement).value) || 30 }"
            />
          </div>
          <div class="field">
            <label>间隔 (ms)</label>
            <input
              :value="draft.health?.intervalMs ?? 1000"
              type="number"
              @input="draft.health = { ...(draft.health ?? { url: null, port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 }), intervalMs: Number(($event.target as HTMLInputElement).value) || 1000 }"
            />
          </div>
        </div>

        <div class="section-t">策略</div>
        <div class="flex wrap" style="gap: 8px 16px">
          <label class="check"><input v-model="draft.policy.failFast" type="checkbox" /> 批量启动失败即中止</label>
          <label class="check"><input v-model="draft.policy.restartOnCrash" type="checkbox" /> 崩溃自动重启</label>
          <label class="check"><input v-model="draft.autoStart" type="checkbox" /> 应用启动时自动运行</label>
          <label class="check">
            最大重启
            <input v-model.number="draft.policy.maxRestarts" type="number" min="0" max="10" style="width: 56px" />
          </label>
        </div>

        <div class="field">
          <label>备注</label>
          <input v-model="draft.note" placeholder="可选，用于说明该项目" />
        </div>
      </div>

      <div class="modal-f">
        <button v-if="!isNew" class="btn danger" @click="drop">删除项目</button>
        <span class="spacer"></span>
        <button class="btn" @click="closeEditor">取消</button>
        <button class="btn primary" :disabled="!valid" @click="submit">保存</button>
      </div>
    </div>
  </div>
</template>
