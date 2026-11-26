<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import {
  closeEditor,
  componentById,
  deleteComponentById,
  editor,
  saveComponentFromEditor,
  state,
} from '../store';
import { shellInvocation } from '../format';
import type { CommandStep, Component, Join } from '../types';

const draft = reactive<Component>(JSON.parse(JSON.stringify(editor.project)) as Component);
const envRows = ref<{ k: string; v: string }[]>(
  Object.entries(draft.env ?? {}).map(([k, v]) => ({ k, v }))
);
const portsText = ref((draft.ports ?? []).join(', '));
const healthOn = ref(!!draft.health);
const healthKind = ref<'port' | 'url'>(draft.health?.url ? 'url' : 'port');

// 命令链：至少保留一行
const steps = ref<CommandStep[]>(
  draft.commands && draft.commands.length
    ? draft.commands.map((s) => ({ cmd: s.cmd, join: s.join ?? 'and' }))
    : [{ cmd: '', join: 'and' }]
);

const isNew = computed(() => editor.isNew);
const valid = computed(
  () =>
    draft.name.trim() !== '' &&
    draft.workdir.trim() !== '' &&
    steps.value.some((s) => s.cmd.trim() !== '')
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

const firstCmd = computed(() => steps.value.find((s) => s.cmd.trim())?.cmd ?? '');

/** stdin 只允许 pipe / null 两种取值 */
function stdoutFix(v: string): 'pipe' | 'null' {
  return v === 'null' ? 'null' : 'pipe';
}

// 环境引用：'' 表示不使用
const stdinSel = ref<string>(draft.stdinMode ?? 'pipe');
const envSel = ref<string>(draft.envProfile ?? '');
const envProfile = computed(() => state.environments.find((e) => e.id === envSel.value));
const envProfileVars = computed(() => Object.keys(envProfile.value?.env ?? {}));
const envHint = computed(() =>
  envProfile.value
    ? `环境提供 ${envProfileVars.value.length} 个变量（${envProfileVars.value.join(', ')}）；下面填同名变量会覆盖它。`
    : '可选：把常用变量组放到「环境」页统一维护，这里直接引用。'
);
const shellHint = computed(() => shellInvocation(effectiveShell.value, firstCmd.value));

/** 依赖候选：同项目的其它组件优先，其余按项目分组排序 */
const otherComponents = computed(() =>
  state.components
    .filter((c) => c.id !== draft.id)
    .slice()
    .sort((a, b) => {
      const sameA = a.projectId === draft.projectId ? 0 : 1;
      const sameB = b.projectId === draft.projectId ? 0 : 1;
      return sameA - sameB || a.name.localeCompare(b.name);
    })
);

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

function addStep(index?: number) {
  const at = index === undefined ? steps.value.length : index + 1;
  steps.value.splice(at, 0, { cmd: '', join: 'and' });
}

function removeStep(i: number) {
  steps.value.splice(i, 1);
  if (!steps.value.length) steps.value.push({ cmd: '', join: 'and' });
  if (i === 0) steps.value[0].join = 'and';
}

function moveStep(i: number, delta: number) {
  const j = i + delta;
  if (j < 0 || j >= steps.value.length) return;
  const [item] = steps.value.splice(i, 1);
  steps.value.splice(j, 0, item);
  steps.value[0].join = 'and';
}

async function submit() {
  if (!valid.value) return;
  const env: Record<string, string> = {};
  for (const row of envRows.value) {
    if (row.k.trim()) env[row.k.trim()] = row.v;
  }
  draft.env = env;
  draft.commands = steps.value
    .map((s, i) => ({ cmd: s.cmd.trim(), join: (i === 0 ? 'and' : s.join) as Join }))
    .filter((s) => s.cmd !== '');
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
  draft.stdinMode = stdoutFix(stdinSel.value);
  draft.envProfile = envSel.value || null;
  draft.shell =
    shellSel.value === ''
      ? null
      : shellSel.value === CUSTOM_SHELL
        ? customShell.value.trim() || null
        : shellSel.value;
  editor.project = draft;
  await saveComponentFromEditor();
}

function drop() {
  if (!draft.id) return;
  const id = draft.id;
  closeEditor();
  deleteComponentById(id);
}
</script>

<template>
  <div class="mask" @click.self="closeEditor">
    <div class="modal">
      <div class="modal-h">
        <h3>{{ isNew ? '新建组件' : `编辑组件 · ${draft.name}` }}</h3>
        <span class="x" @click="closeEditor">✕</span>
      </div>

      <div class="modal-b">
        <div class="row">
          <div class="field" style="flex: 2">
            <label>组件名称 *</label>
            <input v-model="draft.name" placeholder="例如 api-service" />
          </div>
          <div class="field" style="flex: 1.2">
            <label>所属项目</label>
            <select v-model="draft.projectId">
              <option v-for="p in state.projects" :key="p.id" :value="p.id">{{ p.name }}</option>
            </select>
          </div>
          <div class="field" style="flex: 1">
            <label>标签</label>
            <input v-model="draft.stack" placeholder="Python 3.11" />
          </div>
        </div>

        <div class="field">
          <label>工作目录 *</label>
          <input v-model="draft.workdir" placeholder="D:\work\api-service 或 ~/work/api-service" />
          <span class="hint">支持 ~ 展开；目录不存在时启动会直接报错，不会静默回退到 cwd。</span>
        </div>

        <div class="section-t">
          启动命令 *
          <span class="muted" style="font-weight: 400; font-size: 10.5px">按顺序执行，步骤之间可设「且 / 或」</span>
        </div>
        <div v-for="(step, i) in steps" :key="i" class="steprow">
          <div class="joinbox">
            <select v-if="i > 0" v-model="step.join" class="joinselect">
              <option value="and">且</option>
              <option value="or">或</option>
            </select>
            <span v-else class="joinfirst">①</span>
          </div>
          <input v-model="step.cmd" :placeholder="i === 0 ? 'npm ci' : 'npm run dev'" />
          <button class="iconbtn" title="在下方插入一条" @click="addStep(i)">
            <svg viewBox="0 0 16 16" width="14" height="14"><path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
          </button>
          <button class="iconbtn" title="上移" :disabled="i === 0" @click="moveStep(i, -1)">
            <svg viewBox="0 0 16 16" width="14" height="14"><path d="M8 12.5v-9M4.5 7L8 3.5 11.5 7" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </button>
          <button class="iconbtn" title="下移" :disabled="i === steps.length - 1" @click="moveStep(i, 1)">
            <svg viewBox="0 0 16 16" width="14" height="14"><path d="M8 3.5v9M4.5 9L8 12.5 11.5 9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </button>
          <button class="iconbtn danger" title="删除这一步" @click="removeStep(i)">
            <svg viewBox="0 0 16 16" width="14" height="14"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
          </button>
        </div>
        <div class="row" style="align-items: center">
          <button class="btn xs" @click="addStep()">+ 添加命令</button>
          <span class="hint" style="margin: 0">
            且：上一条成功才继续；或：上一条失败时改跑这一条（备选）
          </span>
        </div>
        <span class="hint mono" style="margin-top: 2px">首条实际执行：{{ shellHint }}</span>

        <div class="row">
          <div class="field">
            <label>端口（逗号分隔）</label>
            <input v-model="portsText" placeholder="8000, 8001" />
          </div>
          <div class="field">
            <label>标准输入</label>
            <select v-model="stdinSel">
              <option value="pipe">可输入（默认）</option>
              <option value="null">关闭（立即 EOF）</option>
            </select>
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

        <span class="hint" style="margin-top: -4px">
          有些 CLI 会等 stdin 才继续（例如 <code>claude --print</code> 等待提示词）；选「关闭」可让它立刻读到 EOF，
          也能避免某些工具在管道下端等 3s。
        </span>

        <div class="section-t">环境</div>
        <div class="row">
          <div class="field" style="flex: 1">
            <label>引用环境</label>
            <select v-model="envSel">
              <option value="">不使用环境</option>
              <option v-for="e in state.environments" :key="e.id" :value="e.id">
                {{ e.name }}（{{ Object.keys(e.env).length }} 个变量）
              </option>
            </select>
            <span class="hint">{{ envHint }}</span>
          </div>
        </div>

        <div class="section-t">
          变量覆盖<span v-if="envProfile" class="muted" style="font-weight: 400; font-size: 10.5px">
            （只填需要覆盖 {{ envProfile.name }} 的项）</span
          >
        </div>
        <div v-for="(row, i) in envRows" :key="i" class="row" style="margin-bottom: 6px">
          <input v-model="row.k" style="flex: 0 0 200px" placeholder="KEY" />
          <input v-model="row.v" placeholder="VALUE（支持 %PATH% / $PATH 引用现有环境）" />
          <button class="btn xs" @click="removeEnv(i)">删除</button>
        </div>
        <div><button class="btn xs" @click="addEnv">+ 添加变量</button></div>

        <div class="section-t">依赖（决定一键运行顺序）</div>
        <div class="flex wrap" style="gap: 8px 14px; max-height: 96px; overflow: auto; align-content: flex-start">
          <label v-for="c in otherComponents" :key="c.id" class="check">
            <input type="checkbox" :checked="draft.dependsOn.includes(c.id)" @change="toggleDep(c.id)" />
            {{ c.name }}
            <span class="muted" style="font-size: 10px">{{ componentById(c.id) ? '' : '' }}</span>
          </label>
          <span v-if="!otherComponents.length" class="muted" style="font-size: 11px">暂无其它组件</span>
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
          <input v-model="draft.note" placeholder="可选，用于说明该组件" />
        </div>
      </div>

      <div class="modal-f">
        <button v-if="!isNew" class="btn danger" @click="drop">删除组件</button>
        <span class="spacer"></span>
        <button class="btn" @click="closeEditor">取消</button>
        <button class="btn primary" :disabled="!valid" @click="submit">保存</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.steprow {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
/* 全局 input 是 width:100%，在 flex 行里会把右侧按钮挤出弹窗 */
.steprow input {
  width: auto;
  flex: 1;
  min-width: 0;
}
.joinbox {
  flex: 0 0 50px;
  display: flex;
  justify-content: center;
}
.joinselect {
  width: 48px;
  padding: 7px 4px 7px 8px;
  text-align: center;
  background-position: right 4px center;
}
.joinfirst {
  color: var(--t4);
  font-size: 11px;
}
.iconbtn {
  flex: none;
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  border: 1px solid var(--stroke);
  background: #17191b;
  color: var(--t2);
  cursor: pointer;
  transition: color 0.12s, background 0.12s, border-color 0.12s;
}
.iconbtn:hover:not(:disabled) {
  color: var(--t1);
  background: #22262a;
  border-color: #3a3e42;
}
.iconbtn:disabled {
  opacity: 0.35;
  cursor: default;
}
.iconbtn.danger:hover {
  color: #ff8484;
  border-color: #4a2626;
  background: #2a1717;
}
</style>
