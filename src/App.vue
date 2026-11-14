<script setup lang="ts">
import { onBeforeUnmount, onMounted } from 'vue';
import ConfirmDialog from './components/ConfirmDialog.vue';
import ProjectDialog from './components/ProjectDialog.vue';
import SideNav from './components/SideNav.vue';
import TitleBar from './components/TitleBar.vue';
import BulkRun from './views/BulkRun.vue';
import Monitor from './views/Monitor.vue';
import SettingsView from './views/SettingsView.vue';
import Templates from './views/Templates.vue';
import Workbench from './views/Workbench.vue';
import { bootstrapApp, closeEditor, confirmState, editor, resolveConfirm, state } from './store';
import type { ViewKey } from './types';

const ORDER: ViewKey[] = ['workbench', 'bulk', 'templates', 'monitor', 'settings'];

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (confirmState.open) {
      resolveConfirm(false);
      e.preventDefault();
      return;
    }
    if (editor.open) {
      closeEditor();
      e.preventDefault();
    }
    return;
  }
  if ((e.metaKey || e.ctrlKey) && /^[1-5]$/.test(e.key)) {
    const idx = Number(e.key) - 1;
    if (ORDER[idx]) {
      state.view = ORDER[idx];
      e.preventDefault();
    }
  }
}

onMounted(() => {
  bootstrapApp();
  window.addEventListener('keydown', onKey);
});

onBeforeUnmount(() => window.removeEventListener('keydown', onKey));
</script>

<template>
  <div class="app">
    <TitleBar />
    <div class="shell">
      <SideNav />
      <main class="main" :class="{ fixed: state.view === 'bulk' }">
        <div v-if="state.booting" class="boot">
          <div class="boot-logo">CommandKit</div>
          <div class="boot-sub mono">正在加载 data\\projects.json …</div>
        </div>
        <template v-else>
          <Workbench v-if="state.view === 'workbench'" />
          <BulkRun v-else-if="state.view === 'bulk'" />
          <Templates v-else-if="state.view === 'templates'" />
          <Monitor v-else-if="state.view === 'monitor'" />
          <SettingsView v-else />
        </template>
      </main>
    </div>

    <ProjectDialog v-if="editor.open" />
    <ConfirmDialog v-if="confirmState.open" />

    <div class="toasts">
      <div v-for="t in state.toasts" :key="t.id" class="toast" :class="t.level">{{ t.text }}</div>
    </div>
  </div>
</template>

<style scoped>
.main.fixed {
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.main.fixed > * {
  flex: 1;
  min-height: 0;
}
.boot {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
}
.boot-logo {
  color: var(--t1);
  font-size: 18px;
  font-weight: 600;
  letter-spacing: 0.5px;
}
.boot-sub {
  color: var(--t4);
  font-size: 11px;
}
</style>
