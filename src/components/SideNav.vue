<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { api } from '../api';
import {
  apply,
  groupStatus,
  hydrateTerm,
  projectName,
  state,
  terminalGroups,
  toast,
} from '../store';
import type { Project, ViewKey } from '../types';

const NAV: { key: ViewKey; label: string }[] = [
  { key: 'workbench', label: '项目工作台' },
  { key: 'bulk', label: '终端' },
  { key: 'environments', label: '环境' },
  { key: 'monitor', label: '进程监管' },
  { key: 'settings', label: '设置' },
];

/** 每个项目下的组件数 */
const componentCount = computed(() => {
  const map: Record<string, number> = {};
  for (const c of state.components) map[c.projectId || 'all'] = (map[c.projectId || 'all'] ?? 0) + 1;
  return map;
});

/** 终端页的「会话」：按项目聚合 */
const groups = computed(() => terminalGroups());

const errorCount = computed(
  () => Object.values(state.sessions).filter((s) => s.status === 'failed').length
);

function go(view: ViewKey) {
  state.view = view;
}

function pickProject(id: string) {
  state.activeProjectId = id;
  state.view = 'workbench';
}

function pickGroup(id: string) {
  state.activeGroup = id;
  const g = groups.value.find((x) => x.id === id);
  if (g?.components.length) {
    state.activeSession = g.components[0];
    hydrateTerm(g.components[0]);
  }
}

// ---------------------------------------------------------------- 项目管理
const projectMgr = reactive({ open: false, name: '' });
const busy = ref(false);

async function addProject() {
  const name = projectMgr.name.trim();
  if (!name) return;
  busy.value = true;
  try {
    const snap = await api.saveProject({
      id: '',
      name,
      order: state.projects.length + 1,
      color: null,
    });
    apply(snap);
    projectMgr.name = '';
  } catch (e) {
    toast('error', String(e));
  } finally {
    busy.value = false;
  }
}

async function renameProject(p: Project) {
  try {
    apply(await api.saveProject(p));
    toast('ok', '项目已更新');
  } catch (e) {
    toast('error', String(e));
  }
}

async function removeProject(p: Project) {
  try {
    apply(await api.deleteProject(p.id));
    if (state.activeProjectId === p.id) state.activeProjectId = 'all';
    toast('ok', `已删除项目「${p.name}」，其中组件已归入未归属`);
  } catch (e) {
    toast('error', String(e));
  }
}

</script>

<template>
  <aside class="side">
    <div class="brand">
      <strong>CommandKit</strong>
      <em>v{{ state.info?.version ?? '1.1.0' }}</em>
    </div>

    <div
      v-for="nav in NAV"
      :key="nav.key"
      class="navitem"
      :class="{ active: state.view === nav.key }"
      @click="go(nav.key)"
    >
      {{ nav.label }}
      <span v-if="nav.key === 'monitor' && errorCount" class="kbadge">{{ errorCount }}</span>
    </div>

    <!-- 项目工作台：项目列表 -->
    <template v-if="state.view === 'workbench'">
      <div class="sidehead">
        <span>项目</span><span class="mgmt" @click="projectMgr.open = true">管理</span>
      </div>
      <div class="cat" :class="{ active: state.activeProjectId === 'all' }" @click="pickProject('all')">
        <span class="cdot" style="background: #9eff00"></span>
        <span class="cname">全部项目</span>
        <em>{{ state.components.length }}</em>
      </div>
      <div
        v-for="p in state.projects.filter((x) => x.id !== 'all')"
        :key="p.id"
        class="cat"
        :class="{ active: state.activeProjectId === p.id }"
        @click="pickProject(p.id)"
      >
        <span class="cdot" :style="{ background: p.color ?? '#62686d' }"></span>
        <span class="cname">{{ p.name }}</span>
        <em>{{ componentCount[p.id] ?? 0 }}</em>
      </div>
      <div class="newbtn" @click="projectMgr.open = true">+ 新建项目</div>
    </template>

    <!-- 终端：会话（按项目聚合） -->
    <template v-else-if="state.view === 'bulk'">
      <div class="sidehead">
        <span>会话</span><span>{{ groups.length }}</span>
      </div>
      <div
        v-for="g in groups"
        :key="g.id"
        class="side-session"
        :class="{ active: state.activeGroup === g.id }"
        @click="pickGroup(g.id)"
      >
        <i class="dot" :class="groupStatus(g)"></i>
        <span class="nm">{{ g.name }}</span>
        <em class="cnt">{{ g.components.length }}</em>
      </div>
      <div v-if="!groups.length" class="sidefoot">还没有会话，从工作台点「一键运行」或单个「运行」。</div>
    </template>

    <!-- 环境 -->
    <template v-else-if="state.view === 'environments'">
      <div class="sidehead">
        <span>环境</span><span>{{ state.environments.length }}</span>
      </div>
      <div v-for="e in state.environments" :key="e.id" class="side-session">
        <i class="cdot" :style="{ background: e.color ?? '#62686d' }"></i>
        <span class="nm">{{ e.name }}</span>
      </div>
      <div class="sidefoot">在「编辑组件 → 环境」里引用</div>
    </template>

    <!-- 进程监管 -->
    <template v-else-if="state.view === 'monitor'">
      <div class="sidehead"><span>视图</span><span>刷新 2s</span></div>
      <div class="cat" :class="{ active: state.monitorFilter === 'all' }" @click="state.monitorFilter = 'all'">
        <span class="cname">全部组件</span><em>{{ Object.keys(state.sessions).length }}</em>
      </div>
      <div
        class="cat"
        :class="{ active: state.monitorFilter === 'running' }"
        @click="state.monitorFilter = 'running'"
      >
        <span class="cname">运行中</span>
        <em>{{ Object.values(state.sessions).filter((s) => s.status === 'running' || s.status === 'starting').length }}</em>
      </div>
      <div
        class="cat"
        :class="{ active: state.monitorFilter === 'error' }"
        @click="state.monitorFilter = 'error'"
      >
        <span class="cname">异常</span><em>{{ errorCount }}</em>
      </div>
      <div
        class="cat"
        :class="{ active: state.monitorFilter === 'stopped' }"
        @click="state.monitorFilter = 'stopped'"
      >
        <span class="cname">已停止</span>
        <em>{{ Object.values(state.sessions).filter((s) => s.status === 'stopped' || s.status === 'exited').length }}</em>
      </div>
      <div class="newbtn" @click="state.autoRefresh = !state.autoRefresh">
        自动刷新 · {{ state.autoRefresh ? '开' : '关' }}
      </div>
    </template>

    <template v-else>
      <div class="sidehead"><span>数据</span><span></span></div>
      <div class="cat" @click="api.openDataDir()">
        <span class="cname">打开数据目录</span>
      </div>
    </template>

    <div class="sidefoot">
      <div>{{ state.info?.portable ? '便携模式 · exe 同目录 data\\' : '常规模式 · Application Support' }}</div>
      <div class="pathline" :title="state.info?.dataRoot">{{ state.info?.dataRoot }}</div>
    </div>

    <!-- 项目管理弹层 -->
    <div v-if="projectMgr.open" class="mask" @click.self="projectMgr.open = false">
      <div class="modal sm">
        <div class="modal-h">
          <h3>项目管理</h3>
          <span class="x" @click="projectMgr.open = false">✕</span>
        </div>
        <div class="modal-b">
          <div v-for="p in state.projects" :key="p.id" class="catrow">
            <input v-model="p.name" :disabled="p.id === 'all'" @change="renameProject(p)" />
            <span class="cnt">{{ componentCount[p.id] ?? 0 }} 个组件</span>
            <span v-if="p.id !== 'all'" class="pop stop" @click="removeProject(p)">删除</span>
            <span v-else class="muted" style="font-size: 10.5px">内置</span>
          </div>
          <div class="row" style="margin-top: 4px">
            <input v-model="projectMgr.name" placeholder="新项目名称，回车创建" @keyup.enter="addProject" />
            <button class="btn sm" :disabled="busy || !projectMgr.name.trim()" @click="addProject">添加</button>
          </div>
          <div class="hint muted" style="font-size: 10.5px">
            删除项目不会删除组件，其中的组件会回到「未归属」。项目与组件同存 projects.json。
          </div>
        </div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.kbadge {
  margin-left: auto;
  background: #3a1a1a;
  color: #ff8484;
  border-radius: 8px;
  padding: 0 6px;
  font-size: 10px;
  font-family: var(--mono);
}
.catrow {
  display: flex;
  align-items: center;
  gap: 8px;
}
.catrow input {
  flex: 1;
  min-width: 0;
}
.catrow .cdot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
}
.cnt {
  color: var(--t4);
  font-size: 10.5px;
  white-space: nowrap;
  width: 62px;
  text-align: right;
  flex: none;
}
.catrow .pop {
  flex: none;
}
.mgmt {
  cursor: pointer;
}
.mgmt:hover {
  color: var(--t2);
}
.cdot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
}
.cnt {
  color: var(--t4);
  font-size: 10px;
  font-family: var(--mono);
  flex: none;
}
.pathline {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
