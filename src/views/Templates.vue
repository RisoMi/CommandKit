<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { api } from '../api';
import { apply, askConfirm, openBulk, state, toast } from '../store';
import type { Template, TemplateCommand } from '../types';

const editing = ref(false);
const draft = reactive<{ current: Template | null }>({ current: null });

const activeId = computed(() => state.activeTemplateId ?? state.templates[0]?.id ?? null);
const tpl = computed(() => state.templates.find((t) => t.id === activeId.value) ?? null);

watch(
  [activeId, () => state.templates.length],
  () => {
    if (!state.activeTemplateId && state.templates.length) {
      state.activeTemplateId = state.templates[0].id;
    }
  },
  { immediate: true }
);

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

function startEdit() {
  if (!tpl.value) return;
  draft.current = clone(tpl.value);
  editing.value = true;
}

function startNew() {
  draft.current = {
    id: '',
    name: '',
    desc: '',
    platform: state.info?.platform ?? 'macos',
    shell: null,
    workdir: '',
    env: {},
    commands: [{ cmd: '', tag: '' }],
    updatedAt: Date.now(),
  };
  editing.value = true;
}

function envRows(): { k: string; v: string }[] {
  if (!draft.current) return [];
  return Object.entries(draft.current.env).map(([k, v]) => ({ k, v }));
}

function setEnv(idx: number, key: 'k' | 'v', value: string) {
  const d = draft.current;
  if (!d) return;
  const rows = envRows();
  if (!rows[idx]) return;
  rows[idx][key] = value;
  const next: Record<string, string> = {};
  for (const r of rows) {
    if (r.k.trim()) next[r.k.trim()] = r.v;
  }
  d.env = next;
}

function addEnv() {
  const d = draft.current;
  if (!d) return;
  const n = Object.keys(d.env).length;
  d.env[`VAR_${n + 1}`] = '';
}

function removeEnv(key: string) {
  const d = draft.current;
  if (!d) return;
  delete d.env[key];
}

function addCommand() {
  draft.current?.commands.push({ cmd: '', tag: '' });
}

function removeCommand(i: number) {
  draft.current?.commands.splice(i, 1);
}

async function save() {
  const d = draft.current;
  if (!d) return;
  const t: Template = {
    ...d,
    env: { ...d.env },
    commands: d.commands.filter((c) => c.cmd.trim()).map((c) => ({ cmd: c.cmd.trim(), tag: c.tag.trim() })),
  };
  try {
    const snap = await api.saveTemplate(t);
    apply(snap);
    editing.value = false;
    draft.current = null;
    state.activeTemplateId = t.id || (snap.templates[snap.templates.length - 1]?.id ?? null);
    toast('ok', '模板已保存');
  } catch (e) {
    toast('error', String(e));
  }
}

async function remove() {
  if (!tpl.value) return;
  const ok = await askConfirm({
    title: '删除模板',
    message: `确认删除模板「${tpl.value.name}」？`,
    okText: '删除',
    danger: true,
  });
  if (!ok) return;
  try {
    const snap = await api.deleteTemplate(tpl.value.id);
    apply(snap);
    state.activeTemplateId = null;
    toast('ok', '模板已删除');
  } catch (e) {
    toast('error', String(e));
  }
}

async function runTemplate(id: string) {
  try {
    await api.runTemplate(id);
    openBulk(`tpl:${id}`);
  } catch (e) {
    toast('error', String(e));
  }
}

async function asComponent() {
  if (!tpl.value) return;
  const t = tpl.value;
  const component = {
    id: '',
    name: t.name,
    projectId: state.projects[1]?.id ?? 'all',
    stack: t.platform,
    workdir: t.workdir,
    commands: t.commands
      .filter((c: TemplateCommand) => c.cmd.trim())
      .map((c: TemplateCommand, i: number) => ({ cmd: c.cmd, join: i === 0 ? ('and' as const) : ('and' as const) })),
    env: { ...t.env },
    ports: [],
    dependsOn: [],
    health: null,
    policy: { failFast: true, restartOnCrash: false, maxRestarts: 2 },
    autoStart: false,
    shell: t.shell,
    note: t.desc,
  };
  try {
    const snap = await api.saveComponent(component);
    apply(snap);
    toast('ok', '已存为组件，可在工作台运行 / 编辑');
  } catch (e) {
    toast('error', String(e));
  }
}

const shellLabel = computed(() => {
  const t = tpl.value;
  if (!t) return '—';
  return t.shell ?? state.settings?.shell ?? '—';
});
</script>

<template>
  <div class="view">
    <header class="head">
      <div>
        <h2>模板库</h2>
        <p>模板只描述「环境 + 命令序列」，运行时不改系统环境变量；可一键运行，也可另存为项目。</p>
      </div>
      <div class="actions">
        <button class="btn" @click="startNew">+ 新建模板</button>
        <template v-if="tpl && !editing">
          <button class="btn" @click="startEdit">编辑</button>
          <button class="btn danger" @click="remove">删除</button>
          <button class="btn primary" @click="runTemplate(tpl.id)">运行模板</button>
        </template>
      </div>
    </header>

    <div v-if="!state.templates.length" class="empty">还没有模板，点右上角「新建模板」开始。</div>

    <!-- 查看态 -->
    <template v-else-if="tpl && !editing">
      <div class="card">
        <div class="panel">
          <div class="crumb">模板库 / {{ tpl.name }}</div>
          <h3 class="tname">{{ tpl.name }}</h3>
          <p class="tdesc">{{ tpl.desc || '未填写说明' }}</p>
        </div>
        <div class="strip">
          <div><span>目标 shell</span><strong>{{ shellLabel }}</strong></div>
          <div><span>平台</span><strong>{{ tpl.platform }}</strong></div>
          <div><span>工作目录</span><strong class="path">{{ tpl.workdir || '—' }}</strong></div>
        </div>
      </div>

      <div class="card">
        <div class="card-h">
          <h3>环境变量注入</h3>
          <span class="spacer"></span>
          <span class="muted" style="font-size: 10.5px">经子进程 env 注入 · 不调用 setx / 不写注册表</span>
        </div>
        <div v-if="Object.keys(tpl.env).length" class="envbox">
          <div v-for="(v, k) in tpl.env" :key="k" class="envrow">
            <span class="ek">{{ k }}</span>
            <span class="ev">{{ v }}</span>
          </div>
        </div>
        <div v-else class="envbox muted">未配置</div>
      </div>

      <div class="card">
        <div class="card-h"><h3>预配置命令（按序执行）</h3></div>
        <div v-for="(c, i) in tpl.commands" :key="i" class="cmdrow">
          <span class="no">{{ String(i + 1).padStart(2, '0') }}</span>
          <code>{{ c.cmd }}</code>
          <span class="tag">{{ c.tag }}</span>
        </div>
        <div v-if="!tpl.commands.length" class="envbox muted">未配置命令</div>
      </div>

      <div class="isolation">
        <b>环境隔离</b>
        <span>
          变量随 spawn 注入到独立子 shell，模板之间互不影响；配置文件只写
          {{ state.info?.portable ? 'exe 同目录 data\\templates.json' : '~/Library/Application Support/CommandKit/templates.json' }}。
        </span>
      </div>
    </template>

    <!-- 编辑态 -->
    <div v-if="editing && draft.current" class="card editcard">
      <div class="editgrid">
        <div class="field">
          <label>模板名称</label>
          <input v-model="draft.current.name" placeholder="例如 WebGIS 服务启动" />
        </div>
        <div class="field">
          <label>工作目录</label>
          <input v-model="draft.current.workdir" placeholder="项目根目录" />
        </div>
      </div>
      <div class="field">
        <label>说明</label>
        <textarea v-model="draft.current.desc" rows="2" />
      </div>
      <div class="editgrid">
        <div class="field">
          <label>平台</label>
          <select v-model="draft.current.platform">
            <option value="macos">macos</option>
            <option value="windows">windows</option>
          </select>
        </div>
        <div class="field">
          <label>覆盖 shell（留空用设置里的默认）</label>
          <input v-model="draft.current.shell" placeholder="/bin/zsh 或 pwsh.exe" />
        </div>
      </div>

      <div class="section-t">环境变量</div>
      <div v-for="(row, i) in envRows()" :key="i" class="row" style="margin-bottom: 6px">
        <input style="flex: 0 0 210px" :value="row.k" placeholder="KEY" @change="setEnv(i, 'k', ($event.target as HTMLInputElement).value)" />
        <input :value="row.v" placeholder="VALUE（支持 %PATH% / $PATH 引用）" @change="setEnv(i, 'v', ($event.target as HTMLInputElement).value)" />
        <button class="btn xs" @click="removeEnv(row.k)">删除</button>
      </div>
      <div><button class="btn xs" @click="addEnv">+ 添加变量</button></div>

      <div class="section-t">命令序列</div>
      <div v-for="(c, i) in draft.current.commands" :key="i" class="row" style="margin-bottom: 6px">
        <input :value="c.cmd" placeholder="npm ci" @input="c.cmd = ($event.target as HTMLInputElement).value" />
        <input style="flex: 0 0 150px" :value="c.tag" placeholder="标签" @input="c.tag = ($event.target as HTMLInputElement).value" />
        <button class="btn xs" @click="removeCommand(i)">删除</button>
      </div>
      <div><button class="btn xs" @click="addCommand">+ 添加命令</button></div>

      <div class="modal-f" style="border: none; background: transparent; padding: 12px 0 0">
        <span class="spacer"></span>
        <button class="btn" @click="(editing = false), (draft.current = null)">取消</button>
        <button class="btn primary" @click="save">保存模板</button>
      </div>
    </div>

    <div v-if="tpl && !editing" class="flex">
      <button class="btn sm" @click="asComponent">另存为组件</button>
      <button class="btn sm" @click="runTemplate(tpl.id)">在终端运行</button>
      <span class="muted" style="font-size: 10.5px">
        运行模板会创建独立会话（会话 id 形如 tpl:{{ tpl.id }}），不进批量队列。
      </span>
    </div>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.panel {
  padding: 14px 15px 8px;
}
.tname {
  color: var(--t1);
  font-size: 15px;
  font-weight: 600;
  margin: 6px 0 4px;
}
.tdesc {
  color: var(--t3);
  font-size: 11.5px;
  margin: 0;
  line-height: 1.7;
  max-width: 820px;
}
.strip {
  display: flex;
  border-top: 1px solid var(--stroke-soft);
}
.strip > div {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 10px 15px;
  border-right: 1px solid var(--stroke-soft);
  min-width: 0;
}
.strip > div:last-child {
  border-right: none;
}
.strip span {
  color: var(--t4);
  font-size: 10.5px;
}
.strip strong {
  color: var(--t1);
  font-size: 11px;
  font-weight: 500;
  font-family: var(--mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.envbox {
  padding: 7px 0;
  font-family: var(--mono);
  font-size: 11px;
}
.envrow {
  display: flex;
  gap: 14px;
  padding: 4px 15px;
  border-bottom: 1px solid var(--stroke-soft);
}
.envrow:last-child {
  border-bottom: none;
}
.ek {
  color: var(--t2);
  width: 160px;
  flex: none;
}
.ev {
  color: var(--t3);
  word-break: break-all;
}
.cmdrow {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 15px;
  border-bottom: 1px solid var(--stroke-soft);
  font-size: 11px;
}
.cmdrow:last-child {
  border-bottom: none;
}
.cmdrow .no {
  color: var(--t4);
  font-family: var(--mono);
  width: 18px;
}
.cmdrow code {
  color: var(--t2);
  font-family: var(--mono);
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}
.cmdrow .tag {
  color: var(--t4);
  font-size: 10px;
  border: 1px solid var(--stroke);
  border-radius: 4px;
  padding: 1px 6px;
}
.editcard {
  padding: 14px 15px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.editgrid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}
</style>
