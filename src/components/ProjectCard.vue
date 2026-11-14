<script setup lang="ts">
import { computed } from 'vue';
import { api } from '../api';
import { fmtCpu, fmtUptime, portText, statusText } from '../format';
import {
  openBulk,
  openEditor,
  restartProject,
  runProject,
  state,
  stopProject,
  toggleSelect,
} from '../store';
import type { Project, Status } from '../types';

const props = defineProps<{ project: Project }>();

const session = computed(() => state.sessions[props.project.id] ?? null);
const status = computed<Status>(() => (session.value?.status ?? 'stopped') as Status);
const metric = computed(() => state.metrics[props.project.id]);
const busy = computed(() => !!state.busy[props.project.id]);
const selected = computed(() => state.selected.includes(props.project.id));

const uptime = computed(() => fmtUptime(session.value?.startedAt, state.now));
const meta = computed(() => {
  const port = portText(props.project.ports);
  return port ? `${props.project.workdir} · ${port}` : props.project.workdir;
});

const isActive = computed(() =>
  ['running', 'starting', 'restarting', 'stopping'].includes(status.value)
);

async function toggle() {
  if (status.value === 'running' || status.value === 'starting' || status.value === 'restarting') {
    await stopProject(props.project.id);
  } else {
    await runProject(props.project.id);
  }
}
</script>

<template>
  <div class="proj" :class="{ selected }">
    <label class="pcheck" title="加入批量选择">
      <input type="checkbox" :checked="selected" @change="toggleSelect(project.id)" />
    </label>

    <div class="prow">
      <i class="dot" :class="status"></i>
      <span class="pname" :title="project.name">{{ project.name }}</span>
      <span class="tag">{{ project.stack || 'shell' }}</span>
    </div>

    <div class="pmeta" :title="meta" @click="api.openWorkdir(project.id).catch(() => undefined)">
      {{ meta }}
    </div>

    <div class="pstat">
      <template v-if="isActive || status === 'failed'">
        <span>PID <b>{{ session?.pid ?? '—' }}</b></span>
        <span v-if="status === 'running'">运行 <b>{{ uptime }}</b></span>
        <span v-else>状态 <b>{{ statusText(status) }}</b></span>
        <span>CPU <b>{{ fmtCpu(metric?.cpu ?? 0) }}</b></span>
      </template>
      <template v-else>
        <span>PID <b>—</b></span>
        <span>状态 <b>{{ statusText(status) }}</b></span>
        <span>CPU <b>—</b></span>
      </template>
    </div>

    <div v-if="session?.error" class="perr" :title="session.error">{{ session.error }}</div>

    <div class="pops">
      <template v-if="isActive">
        <span class="pop" :class="{ off: status === 'stopping' }" @click="restartProject(project.id)">重启</span>
        <span class="pop stop" :class="{ off: status === 'stopping' }" @click="stopProject(project.id)">停止</span>
        <span class="pop" @click="openBulk(project.id)">终端</span>
        <span class="pop" @click="openBulk(project.id)">日志</span>
      </template>
      <template v-else>
        <span class="pop go" @click="toggle">{{ status === 'failed' ? '重新运行' : '运行' }}</span>
        <span class="pop" @click="openEditor(project)">编辑</span>
        <span class="pop" @click="openBulk(project.id)">日志</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
.perr {
  color: #ff8484;
  font-size: 10.5px;
  font-family: var(--mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pmeta {
  cursor: pointer;
}
.pmeta:hover {
  color: var(--t2);
  text-decoration: underline;
  text-decoration-style: dotted;
}
</style>
