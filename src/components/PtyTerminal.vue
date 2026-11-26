<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { api } from '../api';
import { state, term, toast } from '../store';

/**
 * 伪终端渲染器：xterm.js 解释 ANSI / 光标控制，按键逐字转发给 PTY。
 * 只有组件把「终端模式」设为 pty 时才会用到这里。
 */
const props = defineProps<{ id: string }>();

const host = ref<HTMLDivElement | null>(null);
let xterm: XTerm | null = null;
let fit: FitAddon | null = null;
let ro: ResizeObserver | null = null;
let written = 0;
let disposed = false;

const THEME = {
  background: '#0a0b0c',
  foreground: '#d8dadc',
  cursor: '#9eff00',
  cursorAccent: '#0a0b0c',
  selectionBackground: '#2c3a1e',
  black: '#1c1e20',
  red: '#ff8484',
  green: '#9eff00',
  yellow: '#e8a33d',
  blue: '#5b9dff',
  magenta: '#b27bff',
  cyan: '#3ad6a8',
  white: '#d8dadc',
  brightBlack: '#62686d',
  brightRed: '#ff9a9a',
  brightGreen: '#b6ff5c',
  brightYellow: '#f0bd6a',
  brightBlue: '#8bbcff',
  brightMagenta: '#c9a4ff',
  brightCyan: '#6fe3c0',
  brightWhite: '#f2f3f4',
};

function flushNew() {
  const t = term[props.id];
  if (!xterm || !t) return;
  while (written < t.raw.length) {
    xterm.write(t.raw[written]);
    written += 1;
  }
}

function create() {
  if (!host.value || xterm) return;
  xterm = new XTerm({
    fontFamily:
      '"SF Mono", "JetBrains Mono", Menlo, Consolas, "Liberation Mono", monospace',
    fontSize: 12,
    lineHeight: 1.25,
    cursorBlink: true,
    scrollback: 5000,
    convertEol: false,
    theme: THEME,
    allowProposedApi: true,
  });
  fit = new FitAddon();
  xterm.loadAddon(fit);
  xterm.open(host.value);

  // 尺寸同步给 PTY，TUI 才能正确布局
  xterm.onResize(({ cols, rows }) => {
    api.resizePty(props.id, cols, rows).catch(() => undefined);
  });
  // 按键 / 组合键 / 方向键：原样转发
  xterm.onData((data) => {
    api.sendInput(props.id, data).catch((e) => toast('error', String(e)));
  });

  requestAnimationFrame(() => {
    try {
      fit?.fit();
    } catch {
      /* 容器还没尺寸 */
    }
    written = 0;
    flushNew();
    xterm?.focus();
  });

  ro = new ResizeObserver(() => {
    try {
      fit?.fit();
    } catch {
      /* ignore */
    }
  });
  ro.observe(host.value);

  const status = state.sessions[props.id]?.status;
  if (status === 'stopped' || status === 'failed') {
    xterm.writeln('\x1b[90m# 会话当前未运行，点下方「启动组件」\x1b[0m');
  }
}

function destroy() {
  disposed = true;
  ro?.disconnect();
  ro = null;
  xterm?.dispose();
  xterm = null;
  fit = null;
  written = 0;
}

onMounted(create);
onBeforeUnmount(destroy);

watch(
  () => props.id,
  () => {
    if (disposed) return;
    destroy();
    disposed = false;
    requestAnimationFrame(create);
  }
);

watch(
  () => term[props.id]?.raw.length ?? 0,
  () => {
    const t = term[props.id];
    if (!t || !xterm) return;
    // 「清空输出」后 raw 变短：重置屏幕再从 0 写
    if (t.raw.length < written) {
      written = 0;
      xterm.reset();
    }
    flushNew();
  }
);
</script>

<template>
  <div class="ptywrap">
    <div ref="host" class="ptyhost"></div>
  </div>
</template>

<style scoped>
.ptywrap {
  flex: 1;
  min-height: 0;
  display: flex;
  background: #0a0b0c;
  border: 1px solid var(--stroke);
  border-top: none;
  border-radius: 0 0 6px 6px;
  padding: 6px 8px;
  overflow: hidden;
}
.ptyhost {
  flex: 1;
  min-width: 0;
  min-height: 0;
}
</style>
