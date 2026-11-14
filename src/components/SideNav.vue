<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { api } from '../api';
import {
  apply,
  hydrateTerm,
  state,
  toast,
} from '../store';
import type { Category, ViewKey } from '../types';

const NAV: { key: ViewKey; label: string }[] = [
  { key: 'workbench', label: '项目工作台' },
  { key: 'bulk', label: '批量运行' },
  { key: 'templates', label: '模板库' },
  { key: 'monitor', label: '进程监管' },
  { key: 'settings', label: '设置' },
];

const catCount = computed(() => {
  const map: Record<string, number> = {};
  for (const p of state.projects) map[p.category] = (map[p.category] ?? 0) + 1;
  return map;
});

const sessions = computed(() =>
  Object.values(state.sessions)
    .slice()
    .sort((a, b) => {
      const rank = (s: string) => (s === 'running' ? 0 : s === 'starting' ? 1 : s === 'failed' ? 2 : 3);
      const d = rank(a.status) - rank(b.status);
      return d !== 0 ? d : a.name.localeCompare(b.name);
    })
);

const errorCount = computed(() => Object.values(state.sessions).filter((s) => s.status === 'failed').length);

function go(view: ViewKey) {
  state.view = view;
}

function pickCategory(id: string) {
  state.activeCategory = id;
  state.view = 'workbench';
}

// ---------------------------------------------------------------- 分类管理
const catMgr = reactive({ open: false, name: '' });
const busy = ref(false);

async function addCategory() {
  const name = catMgr.name.trim();
  if (!name) return;
  busy.value = true;
  try {
    const snap = await api.saveCategory({
      id: '',
      name,
      order: state.categories.length + 1,
      color: null,
    });
    apply(snap);
    catMgr.name = '';
  } catch (e) {
    toast('error', String(e));
  } finally {
    busy.value = false;
  }
}

async function renameCategory(c: Category) {
  try {
    apply(await api.saveCategory(c));
    toast('ok', '分类已更新');
  } catch (e) {
    toast('error', String(e));
  }
}

async function removeCategory(c: Category) {
  try {
    apply(await api.deleteCategory(c.id));
    if (state.activeCategory === c.id) state.activeCategory = 'all';
    toast('ok', `已删除分类「${c.name}」，其中项目已归入未分类`);
  } catch (e) {
    toast('error', String(e));
  }
}

function pickTemplate(id: string) {
  state.activeTemplateId = id;
  state.view = 'templates';
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

    <!-- 项目工作台：分类 -->
    <template v-if="state.view === 'workbench'">
      <div class="sidehead">
        <span>分类</span><span @click="catMgr.open = true">管理</span>
      </div>
      <div class="cat" :class="{ active: state.activeCategory === 'all' }" @click="pickCategory('all')">
        <span class="cdot" style="background: #9eff00"></span>
        <span class="cname">全部项目</span>
        <em>{{ state.projects.length }}</em>
      </div>
      <div
        v-for="c in state.categories.filter((x) => x.id !== 'all')"
        :key="c.id"
        class="cat"
        :class="{ active: state.activeCategory === c.id }"
        @click="pickCategory(c.id)"
      >
        <span class="cdot" :style="{ background: c.color ?? '#62686d' }"></span>
        <span class="cname">{{ c.name }}</span>
        <em>{{ catCount[c.id] ?? 0 }}</em>
      </div>
      <div class="newbtn" @click="catMgr.open = true">+ 新建分类</div>
    </template>

    <!-- 批量运行：会话 -->
    <template v-else-if="state.view === 'bulk'">
      <div class="sidehead">
        <span>会话</span><span>{{ sessions.length }}</span>
      </div>
      <div
        v-for="s in sessions"
        :key="s.id"
        class="side-session"
        :class="{ active: state.activeSession === s.id }"
        @click="(state.activeSession = s.id), hydrateTerm(s.id)"
      >
        <i class="dot" :class="s.status"></i>
        <span class="nm">{{ s.name }}</span>
      </div>
      <div v-if="!sessions.length" class="sidefoot">还没有会话，从工作台启动项目。</div>
    </template>

    <!-- 模板库 -->
    <template v-else-if="state.view === 'templates'">
      <div class="sidehead">
        <span>全部模板</span><span>{{ state.templates.length }}</span>
      </div>
      <div
        v-for="t in state.templates"
        :key="t.id"
        class="side-session"
        :class="{ active: state.activeTemplateId === t.id }"
        @click="pickTemplate(t.id)"
      >
        <span class="nm">{{ t.name }}</span>
      </div>
    </template>

    <!-- 进程监管 -->
    <template v-else-if="state.view === 'monitor'">
      <div class="sidehead"><span>视图</span><span>刷新 2s</span></div>
      <div class="cat" :class="{ active: state.monitorFilter === 'all' }" @click="state.monitorFilter = 'all'">
        <span class="cname">全部进程</span><em>{{ sessions.length }}</em>
      </div>
      <div
        class="cat"
        :class="{ active: state.monitorFilter === 'running' }"
        @click="state.monitorFilter = 'running'"
      >
        <span class="cname">运行中</span>
        <em>{{ sessions.filter((s) => s.status === 'running' || s.status === 'starting').length }}</em>
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
        <em>{{ sessions.filter((s) => s.status === 'stopped' || s.status === 'exited').length }}</em>
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

    <!-- 分类管理弹层 -->
    <div v-if="catMgr.open" class="mask" @click.self="catMgr.open = false">
      <div class="modal sm">
        <div class="modal-h">
          <h3>分类管理</h3>
          <span class="x" @click="catMgr.open = false">✕</span>
        </div>
        <div class="modal-b">
          <div v-for="c in state.categories" :key="c.id" class="catrow">
            <input v-model="c.name" :disabled="c.id === 'all'" @change="renameCategory(c)" />
            <span class="cnt">{{ catCount[c.id] ?? 0 }} 个项目</span>
            <span v-if="c.id !== 'all'" class="pop stop" @click="removeCategory(c)">删除</span>
            <span v-else class="muted" style="font-size: 10.5px">内置</span>
          </div>
          <div class="row" style="margin-top: 4px">
            <input v-model="catMgr.name" placeholder="新分类名称，回车创建" @keyup.enter="addCategory" />
            <button class="btn sm" :disabled="busy || !catMgr.name.trim()" @click="addCategory">添加</button>
          </div>
          <div class="hint muted" style="font-size: 10.5px">
            删除分类不会删除项目，其中的项目会回到「未分类」。分类与项目同存 projects.json。
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
.pathline {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.catrow .cnt {
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
</style>
