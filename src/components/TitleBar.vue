<script setup lang="ts">
import { computed } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { inTauri } from '../bridge';
import { state } from '../store';

// 浏览器演示模式下没有窗口元数据，退化为无操作的标题栏
const win = inTauri ? getCurrentWindow() : null;
// 首帧 info 还没回来时要先按 UA 判断，否则会闪一下 Windows 控件、标题也会压到红绿灯上
const uaIsMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent);
const isMac = computed(() =>
  state.info ? state.info.platform === 'macos' : uaIsMac
);

const TITLES: Record<string, string> = {
  workbench: '项目工作台',
  bulk: '终端',
  templates: '模板库',
  monitor: '进程监管',
  settings: '设置',
};

const title = computed(() => `CommandKit — ${TITLES[state.view] ?? '项目工作台'}`);

const shellLabel = computed(() => {
  const shell = state.settings?.shell ?? '';
  const base = shell.split(/[\\/]/).pop() ?? shell;
  if (/pwsh/i.test(base)) return 'PowerShell 7';
  if (/powershell/i.test(base)) return 'PowerShell';
  if (/cmd/i.test(base)) return 'cmd';
  return base;
});

const badge = computed(() => {
  const platform = state.info?.platform === 'windows' ? 'Windows' : 'macOS';
  return `${platform} · ${shellLabel.value}`;
});

const min = () => win?.minimize();
const max = () => win?.toggleMaximize();
const close = () => win?.close();
</script>

<template>
  <div class="titlebar" :class="{ mac: isMac }" data-tauri-drag-region>
    <span class="win-title" data-tauri-drag-region>{{ title }}</span>
    <div class="tb-right">
      <span class="badge">{{ badge }}</span>
      <template v-if="!isMac">
        <button class="wc" title="最小化" @click="min">—</button>
        <button class="wc" title="最大化" @click="max">▢</button>
        <button class="wc close" title="关闭" @click="close">✕</button>
      </template>
    </div>
  </div>
</template>
