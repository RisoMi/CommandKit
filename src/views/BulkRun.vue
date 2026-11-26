<script setup lang="ts">
import { computed, onMounted } from 'vue';
import PtyTerminal from '../components/PtyTerminal.vue';
import Terminal from '../components/Terminal.vue';
import { api } from '../api';
import { copyText } from '../clipboard';
import { fmtMem, statusText } from '../format';
import {
  clearTerm,
  componentById,
  hydrateTerm,
  restartComponent,
  runComponent,
  state,
  stopComponent,
  termText,
  terminalGroups,
  toast,
} from '../store';
import type { SessionView } from '../types';

const groups = computed(() => terminalGroups());

/** 当前会话（项目）+ 该项目的组件页签 */
const activeGroup = computed(() => {
  const id = state.activeGroup;
  if (id && groups.value.some((g) => g.id === id)) return id;
  if (state.activeSession) {
    const pid =
      componentById(state.activeSession)?.projectId ?? state.sessions[state.activeSession]?.projectId;
    if (pid) return pid;
  }
  return groups.value[0]?.id ?? null;
});

const groupComponents = computed(
  () => groups.value.find((g) => g.id === activeGroup.value)?.components ?? []
);

interface TabItem {
  id: string;
  name: string;
  status: string;
  started: boolean;
}

const tabs = computed<TabItem[]>(() =>
  groupComponents.value.map((id) => {
    const s = state.sessions[id];
    const c = componentById(id);
    return {
      id,
      name: s?.name ?? c?.name ?? id,
      status: s?.status ?? 'stopped',
      started: !!s,
    };
  })
);

const active = computed(() => {
  const id = state.activeSession;
  if (id && groupComponents.value.includes(id)) return id;
  return groupComponents.value[0] ?? null;
});

const activeSession = computed<SessionView | null>(() =>
  active.value ? state.sessions[active.value] ?? null : null
);
const activeComponent = computed(() => (active.value ? componentById(active.value) : undefined));

const cmdText = computed(() => activeSession.value?.command ?? activeComponent.value?.commands?.map((s) => s.cmd).join(' 且 ') ?? '');
const cwdText = computed(() => activeSession.value?.workdir ?? activeComponent.value?.workdir ?? '');

onMounted(() => {
  if (active.value) hydrateTerm(active.value);
});

function pick(id: string) {
  state.activeSession = id;
  hydrateTerm(id);
}

const emptyHint = computed(() => {
  const id = active.value;
  if (!id) return '还没有会话。回到「项目工作台」点「一键运行」或单个「运行」。';
  const tab = tabs.value.find((t) => t.id === id);
  if (!tab) return undefined;
  if (!tab.started) {
    return '该组件还没启动 —— 等批量队列执行到它，或点下方「启动组件」。';
  }
  if (tab.status === 'stopped' || tab.status === 'exited' || tab.status === 'failed') {
    return `该会话${statusText(tab.status)}，点下方「启动组件」可重新运行（当前没有输出）。`;
  }
  return undefined;
});

const bulk = computed(() => state.bulk);
const bulkPct = computed(() => {
  const b = bulk.value;
  if (!b || !b.total) return 0;
  const done = Object.values(b.status).filter((v) => v === 'done' || v === 'failed').length;
  return Math.round((done / b.total) * 100);
});

const queue = computed(() => {
  const b = bulk.value;
  return groupComponents.value.map((id) => {
    const name = componentById(id)?.name ?? state.sessions[id]?.name ?? id;
    const bs = b?.status?.[id];
    let label = '';
    let tone: 'done' | 'running' | 'failed' | 'waiting' = 'waiting';
    if (bs === 'done') {
      label = '就绪';
      tone = 'done';
    } else if (bs === 'running') {
      label = '启动中';
      tone = 'running';
    } else if (bs === 'failed') {
      label = '失败';
      tone = 'failed';
    } else if (bs === 'skipped') {
      label = '已跳过';
    } else if (bs === 'waiting') {
      label = '等待';
    } else {
      const st = state.sessions[id]?.status ?? 'stopped';
      label = statusText(st);
      tone = st === 'running' ? 'done' : st === 'failed' ? 'failed' : st === 'starting' ? 'running' : 'waiting';
    }
    return { id, name, status: state.sessions[id]?.status ?? 'stopped', label, tone };
  });
});

/** 有选中就复制选中，否则复制整段输出（已去掉 ANSI 转义） */
async function copyOutput() {
  if (!active.value) return;
  const selected = (window.getSelection()?.toString() ?? '').trim();
  const text = selected || termText(active.value);
  if (!text.trim()) {
    toast('warn', '这个会话还没有输出');
    return;
  }
  const ok = await copyText(text);
  if (!ok) {
    toast('error', '复制失败，请手动选中后按 ⌘/Ctrl + C');
    return;
  }
  toast('ok', selected ? '已复制选中内容' : `已复制全部输出（${text.split('\n').length} 行）`);
}

async function exportLog() {
  if (!active.value) return;
  try {
    const path = await api.exportLog(active.value);
    toast('ok', `日志已导出：${path}`);
  } catch (e) {
    toast('error', String(e));
  }
}

async function openExternal() {
  if (!active.value) return;
  try {
    await api.openExternalTerminal(active.value);
  } catch (e) {
    toast('error', String(e));
  }
}
</script>

<template>
  <div class="fill-col">
    <header class="head">
      <div>
        <h2>终端</h2>
        <p>按 dependsOn 拓扑排序启动；左侧「会话」是项目，顶部标签是该项目下的组件。</p>
      </div>
      <div class="actions">
        <button class="btn" @click="api.openDataDir()">数据目录</button>
        <button class="btn" :disabled="!active" @click="exportLog">导出日志</button>
        <button v-if="bulk && !bulk.finished" class="btn danger" @click="api.cancelBulk()">
          中断全部
        </button>
      </div>
    </header>

    <div v-if="bulk" class="bulkbar" :class="{ finished: bulk.finished }">
      <strong>
        批量{{ bulk.kind === 'stop' ? '停止' : bulk.kind === 'restart' ? '重启' : '启动' }}
        {{ bulk.finished ? '已完成' : '中' }} · {{ bulkPct }}%
      </strong>
      <span v-if="bulk.message">{{ bulk.message }}</span>
      <span v-else>
        共 {{ bulk.total }} 个组件，按依赖顺序执行
      </span>
      <div class="progress"><i :style="{ width: `${bulkPct}%` }"></i></div>
    </div>

    <template v-if="groups.length || active">
      <div class="tabs">
        <div
          v-for="t in tabs"
          :key="t.id"
          class="tab"
          :class="{ active: active === t.id, pending: !t.started }"
          :title="t.started ? t.name : `${t.name}（尚未启动）`"
          @click="pick(t.id)"
        >
          <i class="dot" :class="t.status"></i>
          <span class="tabname">{{ t.name }}</span>
        </div>
        <span v-if="!tabs.length" class="muted" style="font-size: 11px; padding: 7px 2px">
          该项目下没有组件
        </span>
      </div>

      <div class="runbody">
        <div class="col">
          <div class="cmdbar">
            <span class="mono cmdtext" :title="cmdText">
              <span class="muted">$</span> {{ cmdText }}
            </span>
            <span class="mono muted cmdcwd">{{ cwdText }}</span>
            <span class="pill" :class="activeSession?.status">{{ statusText(activeSession?.status ?? 'stopped') }}</span>
            <span v-if="activeSession?.termMode === 'pty'" class="muted" style="font-size: 10px">
              点击终端直接输入 · Ctrl+C 生效
            </span>
            <span v-else class="muted" style="font-size: 10px">可选中复制</span>
            <span v-if="state.metrics[active ?? '']" class="mono muted">
              CPU {{ (state.metrics[active ?? ''].cpu).toFixed(1) }}% ·
              {{ fmtMem(state.metrics[active ?? ''].mem) }}
            </span>
          </div>

          <PtyTerminal v-if="activeSession?.termMode === 'pty'" :id="active ?? ''" />
          <Terminal v-else :id="active ?? ''" :empty-hint="emptyHint" />

          <div class="flex wrap" style="margin-top: 8px">
            <button class="btn sm" :disabled="!active" @click="active && restartComponent(active)">重启当前</button>
            <button class="btn sm danger" :disabled="!active" @click="active && stopComponent(active)">
              停止当前
            </button>
            <button class="btn sm" :disabled="!active" @click="openExternal">打开外部终端</button>
            <button class="btn sm" :disabled="!active" @click="copyOutput">复制输出</button>
            <button class="btn sm" :disabled="!active" @click="active && clearTerm(active)">清空输出</button>
            <button
              class="btn sm"
              :disabled="!active || activeSession?.status === 'running'"
              @click="active && runComponent(active)"
            >
              启动组件
            </button>
          </div>
        </div>

        <div class="queue">
          <h3>启动队列</h3>
          <div v-for="row in queue" :key="row.id" class="qrow" :class="row.tone">
            <i
              class="dot"
              :class="row.tone === 'done' ? 'running' : row.tone === 'failed' ? 'failed' : row.tone === 'running' ? 'starting' : 'stopped'"
            ></i>
            <span class="qname" :title="row.name">{{ row.name }}</span>
            <b>{{ row.label }}</b>
          </div>
          <div v-if="!queue.length" class="muted" style="font-size: 10.5px">
            该项目下还没有组件。
          </div>
          <div class="qhint">
            队列顺序由组件的依赖关系决定，可在「编辑组件 → 依赖」里调整。
          </div>
        </div>
      </div>
    </template>

    <div v-else class="empty" style="flex: 1">
      还没有运行中的会话。<br />
      回到<span class="link" @click="state.view = 'workbench'">项目工作台</span>，点「一键运行」或单个
      「运行」，输出会在这里以多终端形式呈现。
    </div>
  </div>
</template>

<style scoped>
.fill-col {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
  min-height: 0;
}
.runbody {
  display: flex;
  gap: 10px;
  flex: 1;
  min-height: 0;
}
.col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.cmdbar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 6px 10px;
  background: #101214;
  border: 1px solid var(--stroke);
  border-bottom: none;
  border-radius: 6px 6px 0 0;
  font-size: 10.5px;
  overflow: hidden;
}
.cmdbar .cmdtext {
  color: var(--t2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
.cmdcwd {
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.qname {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tabname {
  max-width: 150px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tab.pending {
  opacity: 0.6;
}
.qhint {
  margin-top: auto;
  color: var(--t4);
  font-size: 10px;
  line-height: 1.55;
  border-top: 1px solid var(--stroke-soft);
  padding-top: 8px;
}
.bulkbar.finished {
  background: #181b1d;
  border-color: var(--stroke);
}
.bulkbar.finished strong {
  color: var(--t2);
}
</style>
