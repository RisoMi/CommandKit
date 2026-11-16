<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { api } from '../api';
import { copyText } from '../clipboard';
import { fmtCpu, fmtMem, fmtUptime, statusText } from '../format';
import { askConfirm, batchIds, componentById, openBulk, projectName, state, toast } from '../store';
import type { ProcessDetail, ProcNode, SessionView } from '../types';

const selected = ref<string | null>(null);
const detail = ref<ProcessDetail | null>(null);
let timer: number | undefined;

const rows = computed<SessionView[]>(() => {
  const list: SessionView[] = [];
  for (const c of state.components) {
    const s = state.sessions[c.id];
    list.push(
      s ?? {
        id: c.id,
        name: c.name,
        kind: 'component',
        status: 'stopped',
        pid: null,
        startedAt: null,
        exitCode: null,
        error: null,
        restarts: 0,
        healthOk: false,
        command: c.commands.map((x) => x.cmd).join(' 且 '),
        workdir: c.workdir,
        ports: c.ports,
        projectId: c.projectId,
      }
    );
  }
  for (const s of Object.values(state.sessions)) {
    if (s.kind === 'template') list.push(s);
  }
  const filter = state.monitorFilter;
  if (filter === 'running') return list.filter((s) => s.status === 'running' || s.status === 'starting');
  if (filter === 'error') return list.filter((s) => s.status === 'failed');
  if (filter === 'stopped') return list.filter((s) => s.status === 'stopped' || s.status === 'exited');
  return list;
});

const totals = computed(() => {
  let cpu = 0;
  let mem = 0;
  let running = 0;
  let failed = 0;
  for (const s of rows.value) {
    const m = state.metrics[s.id];
    if (m) {
      cpu += m.cpu;
      mem += m.mem;
    }
    if (s.status === 'running') running++;
    if (s.status === 'failed') failed++;
  }
  return { cpu, mem, running, failed };
});

function metric(id: string) {
  return state.metrics[id];
}

async function loadDetail() {
  if (!selected.value) {
    detail.value = null;
    return;
  }
  try {
    detail.value = await api.processDetail(selected.value);
  } catch {
    detail.value = null;
  }
}

watch(selected, loadDetail);

onMounted(() => {
  if (rows.value.length) selected.value = rows.value[0].id;
  timer = window.setInterval(() => {
    if (state.autoRefresh) loadDetail();
  }, 2000);
});

onBeforeUnmount(() => {
  if (timer) window.clearInterval(timer);
});

interface TreeRow {
  depth: number;
  node: ProcNode;
}

const treeRows = computed<TreeRow[]>(() => {
  const nodes = detail.value?.tree ?? [];
  const byPid = new Map<number, ProcNode>();
  for (const n of nodes) byPid.set(n.pid, n);
  const depthOf = (n: ProcNode): number => {
    let d = 0;
    let cur: ProcNode | undefined = n;
    const seen = new Set<number>();
    while (cur?.ppid !== null && cur?.ppid !== undefined) {
      if (seen.has(cur.pid)) break;
      seen.add(cur.pid);
      const parent = byPid.get(cur.ppid);
      if (!parent) break;
      d++;
      cur = parent;
    }
    return d;
  };
  return nodes.map((n) => ({ depth: depthOf(n), node: n }));
});

async function copyTail() {
  const text = (detail.value?.logTail ?? []).join('\n');
  if (!text.trim()) {
    toast('warn', '暂无日志可复制');
    return;
  }
  const ok = await copyText(text);
  toast(ok ? 'ok' : 'error', ok ? '已复制日志尾部' : '复制失败');
}

async function exportSnapshot() {
  try {
    const path = await api.exportSnapshot();
    toast('ok', `快照已写入 ${path}`);
  } catch (e) {
    toast('error', String(e));
  }
}

async function stopAll() {
  const ids = batchIds.value;
  const ok = await askConfirm({
    title: '停止全部',
    message: `将按依赖逆序终止 ${ids.length} 个项目的整棵进程树。`,
    okText: '确认停止',
    danger: true,
  });
  if (!ok) return;
  try {
    await api.stopMany(ids);
  } catch (e) {
    toast('error', String(e));
  }
}

function dotOf(s: SessionView): string {
  return s.status;
}
</script>

<template>
  <div class="view">
    <header class="head">
      <div>
        <h2>进程监管</h2>
        <p>心跳 2s · 端口与进程树实时校验 · 数据来自 Rust 侧进程表（sysinfo）。</p>
      </div>
      <div class="actions">
        <button class="btn" @click="loadDetail">刷新</button>
        <button class="btn" @click="exportSnapshot">导出快照</button>
        <button class="btn danger" @click="stopAll">停止全部</button>
      </div>
    </header>

    <div class="metrics">
      <div class="metric"><span>运行中</span><strong>{{ totals.running }}</strong></div>
      <div class="metric">
        <span>异常 / 退出</span><strong :class="{ bad: totals.failed > 0 }">{{ totals.failed }}</strong>
      </div>
      <div class="metric"><span>总 CPU</span><strong>{{ fmtCpu(totals.cpu) }}</strong></div>
      <div class="metric"><span>总内存</span><strong>{{ fmtMem(totals.mem) }}</strong></div>
    </div>

    <table class="p">
      <thead>
        <tr>
          <th>项目</th>
          <th>分类</th>
          <th>PID</th>
          <th>端口</th>
          <th>CPU</th>
          <th>内存</th>
          <th>运行时长</th>
          <th>状态</th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="s in rows"
          :key="s.id"
          :class="{ sel: selected === s.id }"
          @click="selected = s.id"
        >
          <td>
            <span class="flex" style="gap: 7px">
              <i class="dot" :class="dotOf(s)"></i>{{ s.name }}
            </span>
          </td>
          <td class="muted">{{ s.projectId === 'template' ? '模板' : projectName(s.projectId) }}</td>
          <td class="mono">{{ s.pid ?? '—' }}</td>
          <td class="mono">{{ s.ports.length ? s.ports.map((p) => `:${p}`).join(' ') : '—' }}</td>
          <td class="mono">{{ metric(s.id) ? fmtCpu(metric(s.id)!.cpu) : '—' }}</td>
          <td class="mono">{{ metric(s.id) && metric(s.id)!.mem ? fmtMem(metric(s.id)!.mem) : '—' }}</td>
          <td class="mono">{{ fmtUptime(s.startedAt, state.now) }}</td>
          <td>
            <span class="pill" :class="s.status">{{ statusText(s.status) }}</span>
          </td>
          <td class="mono ops">
            <span class="link" @click.stop="openBulk(s.id)">日志</span>
            <span class="link" @click.stop="state.view = 'workbench'; selected = s.id">工作台</span>
          </td>
        </tr>
        <tr v-if="!rows.length">
          <td colspan="9" style="text-align: center; color: var(--t4); padding: 22px">
            该视图下没有进程
          </td>
        </tr>
      </tbody>
    </table>

    <div class="detail">
      <div class="card">
        <div class="card-h">
          <h3>进程树 · {{ detail?.id ?? '—' }}</h3>
          <span class="spacer"></span>
          <span v-if="detail?.tree.length" class="muted mono">{{ detail.tree.length }} 个进程</span>
        </div>
        <div class="tree">
          <template v-if="treeRows.length">
            <div v-for="row in treeRows" :key="row.node.pid" class="trow">
              <span class="muted">{{ row.depth ? '  '.repeat(row.depth - 1) + '└─ ' : '' }}</span>
              <span class="pid">{{ row.node.pid }}</span>
              <span class="tname">{{ row.node.name }}</span>
              <span class="muted tcmd">{{ row.node.cmd }}</span>
            </div>
          </template>
          <div v-else class="muted" style="padding: 8px 0">
            选中一个运行中的会话查看进程树。
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-h">
          <h3>实时日志尾部</h3>
          <span class="spacer"></span>
          <span class="link" @click="copyTail">复制</span>
          <span class="link" @click="selected && openBulk(selected)">打开多终端</span>
        </div>
        <div class="tree logtail">
          <div v-for="(l, i) in detail?.logTail ?? []" :key="i" class="trow">
            <span class="tcmd">{{ l }}</span>
          </div>
          <div v-if="!detail?.logTail?.length" class="muted">暂无输出</div>
        </div>
      </div>
    </div>

    <div class="detail">
      <div class="card">
        <div class="card-h"><h3>环境变量（模板注入 · 脱敏）</h3></div>
        <div class="envbox">
          <div v-for="kv in detail?.env ?? []" :key="kv[0]" class="envrow">
            <span class="ek">{{ kv[0] }}</span>
            <span class="ev">{{ kv[1] }}</span>
          </div>
          <div v-if="!detail?.env?.length" class="muted" style="padding: 8px 0">
            该项目未注入额外环境变量。
          </div>
        </div>
      </div>
      <div class="card">
        <div class="card-h"><h3>端口监听</h3></div>
        <div class="envbox">
          <div v-for="(l, i) in detail?.listen ?? []" :key="i" class="envrow">
            <span class="ev">{{ l }}</span>
          </div>
          <div v-if="!detail?.listen?.length" class="muted" style="padding: 8px 0">
            该会话未声明端口。
          </div>
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
.ops {
  display: flex;
  gap: 10px;
}
.detail {
  display: grid;
  grid-template-columns: 1.15fr 1fr;
  gap: 10px;
}
.tree {
  background: var(--term);
  border-radius: 0 0 8px 8px;
  padding: 9px 12px;
  font-family: var(--mono);
  font-size: 10.5px;
  line-height: 1.85;
  color: var(--t2);
  max-height: 240px;
  overflow: auto;
}
.trow {
  white-space: pre;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pid {
  color: var(--accent);
  margin-right: 8px;
}
.tname {
  color: var(--t1);
  margin-right: 8px;
}
.tcmd {
  color: var(--t4);
  white-space: pre;
  overflow: hidden;
  text-overflow: ellipsis;
}
.logtail {
  white-space: pre-wrap;
  word-break: break-all;
}
.envbox {
  padding: 6px 0;
  font-family: var(--mono);
  font-size: 10.5px;
  max-height: 150px;
  overflow: auto;
}
.envrow {
  display: flex;
  gap: 10px;
  padding: 3px 13px;
}
.ek {
  color: var(--t2);
  width: 150px;
  flex: none;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ev {
  color: var(--t4);
  word-break: break-all;
  flex: 1;
}
</style>
