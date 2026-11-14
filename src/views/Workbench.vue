<script setup lang="ts">
import { computed } from 'vue';
import ProjectCard from '../components/ProjectCard.vue';
import { fmtCpu, fmtMem } from '../format';
import {
  categoryGroups,
  clearSelection,
  openEditor,
  runBatch,
  state,
  visibleProjects,
} from '../store';

const groups = computed(() => categoryGroups());

const counts = computed(() => {
  const list = visibleProjects.value;
  const out = { running: 0, starting: 0, stopped: 0, failed: 0 };
  for (const p of list) {
    const st = state.sessions[p.id]?.status ?? 'stopped';
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
  for (const p of visibleProjects.value) {
    const m = state.metrics[p.id];
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
        <p>每个项目占用独立终端会话，环境变量与工作目录互不影响。</p>
      </div>
      <div class="actions">
        <button class="btn primary" :disabled="!visibleProjects.length" @click="runBatch('run')">一键运行</button>
        <button class="btn" :disabled="!visibleProjects.length" @click="runBatch('restart')">一键重启</button>
        <button class="btn danger" :disabled="!visibleProjects.length" @click="runBatch('stop')">
          一键停止
        </button>
        <button class="btn" @click="openEditor()">+ 新建项目</button>
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
      <span>按依赖顺序执行，点击查看多终端</span>
      <div class="progress"><i :style="{ width: `${bulkPct}%` }"></i></div>
      <span class="mono">{{ bulkPct }}%</span>
    </div>

    <div v-if="state.selected.length" class="flex" style="font-size: 11px; color: var(--t3)">
      已选 <b class="mono" style="color: var(--accent)">{{ state.selected.length }}</b> 个项目，批量按钮只作用于选中项
      <button class="btn xs ghost" @click="clearSelection">清除选择</button>
    </div>

    <div v-for="g in groups" :key="g.id">
      <div class="grouphead">{{ g.name }} <em>· {{ g.projects.length }}</em></div>
      <div class="grid">
        <ProjectCard v-for="p in g.projects" :key="p.id" :project="p" />
      </div>
    </div>

    <div v-if="!visibleProjects.length" class="empty">
      该分类下还没有项目。<br />
      <span class="link" @click="openEditor()">新建一个项目</span>，或在左侧切换到「全部项目」。
    </div>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
</style>
