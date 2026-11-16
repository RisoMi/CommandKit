<script setup lang="ts">
import { computed } from 'vue';
import { api } from '../api';
import { fmtCpu, fmtUptime, portText, statusText } from '../format';
import {
  deleteComponentById,
  openBulk,
  openEditor,
  projectName,
  restartComponent,
  runComponent,
  state,
  stopComponent,
  toggleSelect,
} from '../store';
import type { Component, Status } from '../types';

const props = defineProps<{ component: Component; showProject?: boolean }>();

const session = computed(() => state.sessions[props.component.id] ?? null);
const status = computed<Status>(() => (session.value?.status ?? 'stopped') as Status);
const metric = computed(() => state.metrics[props.component.id]);
const selected = computed(() => state.selected.includes(props.component.id));

const uptime = computed(() => fmtUptime(session.value?.startedAt, state.now));
const meta = computed(() => {
  const port = portText(props.component.ports);
  return port ? `${props.component.workdir} · ${port}` : props.component.workdir;
});
const stepCount = computed(() => props.component.commands.filter((s) => s.cmd.trim()).length);

const isActive = computed(() =>
  ['running', 'starting', 'restarting', 'stopping'].includes(status.value)
);

async function toggle() {
  if (isActive.value) await stopComponent(props.component.id);
  else await runComponent(props.component.id);
}

async function openFolder() {
  await api.openWorkdir(props.component.id).catch(() => undefined);
}
</script>

<template>
  <div class="proj" :class="{ selected }">
    <div class="prow">
      <label class="pcheck" title="加入批量选择">
        <input type="checkbox" :checked="selected" @change="toggleSelect(component.id)" />
      </label>
      <i class="dot" :class="status"></i>
      <span class="pname" :title="component.name">{{ component.name }}</span>
      <span v-if="showProject" class="tag projtag">{{ projectName(component.projectId) }}</span>
      <span v-if="component.stack" class="tag">{{ component.stack }}</span>
      <span class="spacer"></span>
      <button class="iconbtn" title="打开工作目录" @click.stop="openFolder">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
          <path
            d="M1.5 4.2c0-.6.5-1.1 1.1-1.1h3l1.3 1.5h6.5c.6 0 1.1.5 1.1 1.1v6.6c0 .6-.5 1.1-1.1 1.1H2.6c-.6 0-1.1-.5-1.1-1.1z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
        </svg>
      </button>
      <button class="iconbtn danger" title="删除组件" @click.stop="deleteComponentById(component.id)">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
          <path
            d="M3 4.5h10M6.5 4.5V3.2c0-.4.3-.7.7-.7h1.6c.4 0 .7.3.7.7v1.3M4.3 4.5l.6 8.1c0 .5.4.9.9.9h4.4c.5 0 .9-.4.9-.9l.6-8.1"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </button>
    </div>

    <div class="pmeta" :title="meta" @click="openFolder">{{ meta }}</div>

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
        <span v-if="stepCount > 1">命令 <b>{{ stepCount }} 步</b></span>
      </template>
    </div>

    <div v-if="session?.error" class="perr" :title="session.error">{{ session.error }}</div>

    <div class="pops">
      <template v-if="isActive">
        <span class="pop" :class="{ off: status === 'stopping' }" @click="restartComponent(component.id)">重启</span>
        <span class="pop stop" :class="{ off: status === 'stopping' }" @click="stopComponent(component.id)">停止</span>
        <span class="pop" @click="openBulk(component.id)">终端</span>
      </template>
      <template v-else>
        <span class="pop go" @click="toggle">{{ status === 'failed' ? '重新运行' : '运行' }}</span>
        <span class="pop" @click="openEditor(component)">编辑</span>
        <span class="pop" @click="openBulk(component.id)">日志</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
.prow {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}
.pcheck {
  display: flex;
  align-items: center;
  flex: none;
  cursor: pointer;
}
.projtag {
  color: var(--t4);
}
.spacer {
  flex: 1;
}
.iconbtn {
  flex: none;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 4px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--t4);
  cursor: pointer;
  transition: color 0.12s, background 0.12s, border-color 0.12s;
}
.iconbtn:hover {
  color: var(--t1);
  background: #1e2124;
  border-color: var(--stroke);
}
.iconbtn.danger:hover {
  color: #ff8484;
  border-color: #4a2626;
  background: #2a1717;
}
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
