<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import ComponentCard from '../components/ComponentCard.vue';
import { api } from '../api';
import { inTauri } from '../bridge';
import { pickOpenPath, pickSavePath } from '../dialog';
import { fmtCpu, fmtMem } from '../format';
import {
  apply,
  clearSelection,
  componentGroups,
  openEditor,
  runBatch,
  state,
  toast,
  visibleComponents,
} from '../store';
import type { Template } from '../types';

const tplOpen = ref(false);

// Esc 关闭模板弹层（和编辑弹窗保持一致的手感）
function onEsc(e: KeyboardEvent) {
  if (e.key === 'Escape') tplOpen.value = false;
}
watch(tplOpen, (open) => {
  if (open) window.addEventListener('keydown', onEsc);
  else window.removeEventListener('keydown', onEsc);
});
onBeforeUnmount(() => window.removeEventListener('keydown', onEsc));

function openTemplates() {
  if (!state.templates.length) {
    toast('warn', '还没有模板');
    return;
  }
  tplOpen.value = true;
}

/** 用模板新建组件（进编辑弹窗再确认） */
function useTemplate(t: Template) {
  tplOpen.value = false;
  const projectId =
    state.activeProjectId === 'all' ? state.projects[1]?.id ?? 'all' : state.activeProjectId;
  openEditor({
    id: '',
    name: t.name,
    projectId,
    stack: t.platform,
    workdir: t.workdir,
    stdinMode: 'pipe',
    termMode: 'pipe',
    envProfile: null,
    commands: t.commands
      .filter((c) => c.cmd.trim())
      .map((c) => ({ cmd: c.cmd, join: 'and' as const })),
    env: { ...t.env },
    ports: [],
    dependsOn: [],
    health: null,
    policy: { failFast: true, restartOnCrash: false, maxRestarts: 2 },
    autoStart: false,
    shell: t.shell,
    note: t.desc,
  });
}

/** 模板直接跑一次（会话归到「模板运行」分组，终端可见） */
async function runTemplateNow(t: Template) {
  tplOpen.value = false;
  try {
    await api.runTemplate(t.id);
    state.view = 'bulk';
    state.activeGroup = 'template';
    state.activeSession = `tpl:${t.id}`;
  } catch (e) {
    toast('error', String(e));
  }
}

const busyIO = ref(false);

async function exportConfig() {
  if (!state.selected.length) {
    toast('warn', '先勾选要导出的组件（可单选，也可多选）');
    return;
  }
  if (!inTauri) {
    toast('warn', '浏览器演示模式没有文件对话框');
    return;
  }
  const stamp = new Date().toISOString().slice(0, 10);
  const path = await pickSavePath(`commandkit-components-${stamp}.json`);
  if (!path) return;
  busyIO.value = true;
  try {
    const saved = await api.exportComponents([...state.selected], path);
    toast('ok', `已导出 ${state.selected.length} 个组件 → ${saved}`);
  } catch (e) {
    toast('error', String(e));
  } finally {
    busyIO.value = false;
  }
}

async function importConfig() {
  if (!inTauri) {
    toast('warn', '浏览器演示模式没有文件对话框');
    return;
  }
  const path = await pickOpenPath();
  if (!path) return;
  busyIO.value = true;
  try {
    const r = await api.importComponents(path);
    apply(r.snapshot);
    state.selected = [];
    if (!r.imported) {
      toast('warn', '没有可导入的组件（检查 JSON 结构）');
    } else {
      toast(
        'ok',
        `已导入 ${r.imported} 个组件${r.skipped.length ? `，跳过 ${r.skipped.length} 个` : ''}`
      );
    }
  } catch (e) {
    toast('error', String(e));
  } finally {
    busyIO.value = false;
  }
}

const groups = computed(() => componentGroups());

const counts = computed(() => {
  const list = visibleComponents.value;
  const out = { running: 0, starting: 0, stopped: 0, failed: 0 };
  for (const c of list) {
    const st = state.sessions[c.id]?.status ?? 'stopped';
    if (st === 'running') out.running++;
    else if (st === 'starting' || st === 'restarting' || st === 'stopping') out.starting++;
    else if (st === 'failed') out.failed++;
    else out.stopped++;
  }
  return out;
});

const totals = computed(() => {
  let cpu = 0;
  let mem = 0;
  for (const c of visibleComponents.value) {
    const m = state.metrics[c.id];
    if (!m) continue;
    cpu += m.cpu;
    mem += m.mem;
  }
  return { cpu, mem };
});

const bulk = computed(() => state.bulk);
const bulkPct = computed(() => {
  const b = bulk.value;
  if (!b || !b.total) return 0;
  const done = Object.values(b.status).filter((v) => v === 'done' || v === 'failed').length;
  return Math.round((done / b.total) * 100);
});
</script>

<template>
  <div class="view">
    <div v-if="state.info && !state.info.writable" class="banner">
      <b>数据目录不可写</b>
      <span class="muted">{{ state.info.dataRoot }}</span>
      <span>—— 便携版请放到有写权限的位置（不要在 Program Files / 只读 U 盘 / 网络盘运行）。</span>
    </div>

    <header class="head">
      <div>
        <h2>项目工作台</h2>
        <p>左侧按项目管理组件；每个组件占用独立终端会话，环境变量与工作目录互不影响。</p>
      </div>
      <div class="actions">
        <button class="btn primary" :disabled="!visibleComponents.length" @click="runBatch('run')">一键运行</button>
        <button class="btn" :disabled="!visibleComponents.length" @click="runBatch('restart')">一键重启</button>
        <button class="btn danger" :disabled="!visibleComponents.length" @click="runBatch('stop')">
          一键停止
        </button>
        <button class="btn" @click="openEditor()">+ 新建组件</button>
      </div>
    </header>

    <div class="summary">
      <div><span>运行中</span><strong>{{ counts.running }}</strong></div>
      <div><span>启动中</span><strong>{{ counts.starting }}</strong></div>
      <div><span>已停止</span><strong>{{ counts.stopped }}</strong></div>
      <div><span>异常</span><strong :class="{ bad: counts.failed > 0 }">{{ counts.failed }}</strong></div>
      <div>
        <span>CPU / 内存</span>
        <strong>{{ fmtCpu(totals.cpu) }} / {{ fmtMem(totals.mem) }}</strong>
      </div>
    </div>

    <div
      v-if="bulk && !bulk.finished"
      class="bulkbar"
      style="cursor: pointer"
      @click="state.view = 'bulk'"
    >
      <strong>批量{{ bulk.kind === 'stop' ? '停止' : bulk.kind === 'restart' ? '重启' : '启动' }}中 · {{ bulk.index + 1 }} / {{ bulk.total }}</strong>
      <span>按依赖顺序执行，点击查看终端</span>
      <div class="progress"><i :style="{ width: `${bulkPct}%` }"></i></div>
      <span class="mono">{{ bulkPct }}%</span>
    </div>

    <div class="toolbar">
      <button class="btn xs" :disabled="busyIO" @click="openTemplates">从模板新建</button>
      <button class="btn xs" :disabled="busyIO" @click="importConfig">导入配置</button>
      <button class="btn xs" :disabled="busyIO || !state.selected.length" @click="exportConfig">
        导出配置{{ state.selected.length ? ` (${state.selected.length})` : '' }}
      </button>
      <span class="muted" style="font-size: 10.5px">
        勾选组件后可导出为 JSON（单选 / 多选都行）；导入会追加到对应项目
      </span>
    </div>

    <div v-if="state.selected.length" class="flex" style="font-size: 11px; color: var(--t3)">
      已选 <b class="mono" style="color: var(--accent)">{{ state.selected.length }}</b> 个组件，批量按钮只作用于选中项
      <button class="btn xs ghost" @click="clearSelection">清除选择</button>
    </div>

    <div v-for="g in groups" :key="g.id">
      <div class="grouphead">{{ g.name }} <em>· {{ g.components.length }}</em></div>
      <div class="grid">
        <ComponentCard v-for="c in g.components" :key="c.id" :component="c" />
      </div>
    </div>

    <div v-if="!visibleComponents.length" class="empty">
      当前项目下还没有组件。<br />
      <span class="link" @click="openEditor()">新建一个组件</span>，或在左侧切换到「全部项目」。
    </div>

    <div v-if="tplOpen" class="mask" @click.self="tplOpen = false">
      <div class="modal sm">
        <div class="modal-h">
          <h3>从模板新建组件</h3>
          <span class="x" @click="tplOpen = false">✕</span>
        </div>
        <div class="modal-b">
          <div v-for="t in state.templates" :key="t.id" class="tplrow">
            <div class="tplmain">
              <div class="tplname">{{ t.name }}</div>
              <div class="tpldesc">{{ t.desc || '—' }}</div>
              <div class="tpldesc mono">{{ t.commands.map((c) => c.cmd).join('   ·   ') }}</div>
            </div>
            <div class="tplacts">
              <button class="btn xs" @click="useTemplate(t)">新建组件</button>
              <button class="btn xs" @click="runTemplateNow(t)">直接运行</button>
            </div>
          </div>
          <div v-if="!state.templates.length" class="muted" style="font-size: 11px">还没有模板。</div>
        </div>
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
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.tplrow {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 9px 0;
  border-bottom: 1px solid var(--stroke-soft);
}
.tplrow:last-child {
  border-bottom: none;
}
.tplmain {
  flex: 1;
  min-width: 0;
}
.tplname {
  font-size: 12px;
  color: var(--t1);
}
.tpldesc {
  font-size: 10.5px;
  color: var(--t4);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tplacts {
  display: flex;
  gap: 6px;
  flex: none;
}
</style>
