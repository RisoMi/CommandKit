<script setup lang="ts">
import { computed, onMounted } from 'vue';
import Terminal from '../components/Terminal.vue';
import { api } from '../api';
import { copyText } from '../clipboard';
import { fmtMem, statusText } from '../format';
import {
  clearTerm,
  hydrateTerm,
  projectById,
  restartProject,
  runProject,
  state,
  stopProject,
  termText,
  toast,
} from '../store';
import type { SessionView } from '../types';

const sessions = computed(() =>
  Object.values(state.sessions).filter((s) => s.kind === 'project' || s.kind === 'template')
);

const active = computed(() => {
  const id = state.activeSession;
  if (id && (state.sessions[id] || projectById(id))) return id;
  const running = sessions.value.find((s) => s.status === 'running');
  return running?.id ?? sessions.value[0]?.id ?? null;
});

const activeProject = computed(() => (active.value ? projectById(active.value) : undefined));
const cmdText = computed(() => activeSession.value?.command ?? activeProject.value?.command ?? '');
const cwdText = computed(() => activeSession.value?.workdir ?? activeProject.value?.workdir ?? '');

const activeSession = computed<SessionView | null>(() =>
  active.value ? state.sessions[active.value] ?? null : null
);

onMounted(() => {
  if (active.value) hydrateTerm(active.value);
});

function pick(id: string) {
  state.activeSession = id;
  hydrateTerm(id);
}

const bulk = computed(() => state.bulk);
const bulkPct = computed(() => {
  const b = bulk.value;
  if (!b || !b.total) return 0;
  const done = Object.values(b.status).filter((v) => v === 'done' || v === 'failed').length;
  return Math.round((done / b.total) * 100);
});

interface QueueRow {
  id: string;
  name: string;
  status: string;
  label: string;
  tone: 'done' | 'running' | 'failed' | 'waiting';
}

const queue = computed<QueueRow[]>(() => {
  const b = bulk.value;
  const ids = b?.order?.length
    ? b.order
    : state.projects.filter((p) => state.sessions[p.id]).map((p) => p.id);
  const rows: QueueRow[] = [];
  for (const id of ids) {
    const name = projectById(id)?.name ?? state.sessions[id]?.name ?? id;
    const bs = b?.status?.[id];
    let label = '';
    let tone: QueueRow['tone'] = 'waiting';
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
      tone = 'waiting';
    } else if (bs === 'waiting') {
      label = '等待';
      tone = 'waiting';
    } else {
      const st = state.sessions[id]?.status ?? 'stopped';
      label = statusText(st);
      tone = st === 'running' ? 'done' : st === 'failed' ? 'failed' : st === 'starting' ? 'running' : 'waiting';
    }
    rows.push({ id, name, status: bs ?? state.sessions[id]?.status ?? 'stopped', label, tone });
  }
  return rows;
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
        <h2>批量运行 · 多终端</h2>
        <p>按 dependsOn 拓扑排序启动，前序健康检查通过再启动后继；每个标签是独立会话。</p>
      </div>
      <div class="actions">
        <button class="btn" @click="api.openDataDir()">数据目录</button>
        <button class="btn" :disabled="!sessions.length" @click="exportLog">导出日志</button>
        <button
          v-if="bulk && !bulk.finished"
          class="btn danger"
          @click="api.cancelBulk()"
        >
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
      <span v-else>按依赖顺序：{{ bulk.order.map((id) => projectById(id)?.name ?? id).join(' → ') }}</span>
      <div class="progress"><i :style="{ width: `${bulkPct}%` }"></i></div>
    </div>

    <template v-if="sessions.length || active">
      <div class="tabs">
        <div
          v-for="s in sessions"
          :key="s.id"
          class="tab"
          :class="{ active: active === s.id }"
          @click="pick(s.id)"
        >
          <i class="dot" :class="s.status"></i>
          {{ s.name }}
        </div>
      </div>

      <div class="runbody">
        <div class="col">
          <div class="cmdbar">
            <span class="mono cmdtext" :title="cmdText">
              <span class="muted">$</span> {{ cmdText }}
            </span>
            <span class="mono muted">{{ cwdText }}</span>
            <span class="pill" :class="activeSession?.status">{{ statusText(activeSession?.status ?? 'stopped') }}</span>
            <span class="muted" style="font-size: 10px">可选中复制</span>
            <span v-if="state.metrics[active ?? '']" class="mono muted">
              CPU {{ (state.metrics[active ?? ''].cpu).toFixed(1) }}% ·
              {{ fmtMem(state.metrics[active ?? ''].mem) }}
            </span>
          </div>

          <Terminal :id="active ?? ''" />

          <div class="flex wrap" style="margin-top: 8px">
            <button class="btn sm" :disabled="!active" @click="active && restartProject(active)">重启当前</button>
            <button class="btn sm danger" :disabled="!active" @click="active && stopProject(active)">
              停止当前
            </button>
            <button class="btn sm" :disabled="!active" @click="openExternal">打开外部终端</button>
            <button class="btn sm" :disabled="!active" @click="copyOutput">复制输出</button>
            <button class="btn sm" :disabled="!active" @click="active && clearTerm(active)">清空输出</button>
            <button
              class="btn sm"
              :disabled="!active || activeSession?.status === 'running'"
              @click="active && runProject(active)"
            >
              启动会话
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
            还没有项目会话。
          </div>
          <div class="qhint">
            队列顺序由项目的依赖关系决定，可在「编辑项目 → 依赖」里调整。
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
.qname {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
