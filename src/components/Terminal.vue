<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { api } from '../api';
import { normalizeCr, stripAnsi } from '../ansi';
import { statusText } from '../format';
import { state, term, toast } from '../store';

const props = defineProps<{ id: string; emptyHint?: string }>();

const box = ref<HTMLElement | null>(null);
const stick = ref(true);
const input = ref('');

const t = computed(() => term[props.id]);
/** 行数直接从 t 取：二级 computed 在「键后建」的场景下不会失效 */
const lineCount = () => t.value?.lines.length ?? 0;
// 尾部未换行的内容（进度条等）同样要处理 \r 与 ANSI，避免显示成乱码
const pending = computed(() => stripAnsi(normalizeCr(t.value?.pending ?? '')));

function scrollBottom() {
  const el = box.value;
  if (el) el.scrollTop = el.scrollHeight;
}

function onScroll() {
  const el = box.value;
  if (!el) return;
  stick.value = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
}

watch(
  () => lineCount(),
  async () => {
    if (!stick.value) return;
    await nextTick();
    scrollBottom();
  }
);

watch(
  () => props.id,
  async () => {
    stick.value = true;
    await nextTick();
    scrollBottom();
  }
);

onMounted(scrollBottom);

async function send() {
  const text = input.value;
  input.value = '';
  try {
    await api.sendInput(props.id, `${text}\n`).catch(() => undefined);
  } catch (e) {
    toast('error', String(e));
  }
}
</script>

<template>
  <div class="termwrap">
    <div ref="box" class="term" @scroll="onScroll">
      <!-- v-memo 必须带上会话 id：不同会话的行号会重复，否则切换标签会复用上一条会话的 DOM -->
      <div v-for="line in t?.lines ?? []" :key="`${id}-${line.n}`" class="ln" v-memo="[id, line.n]">
        <span v-for="(seg, i) in line.segs" :key="i" :style="seg.s">{{ seg.t }}</span>
        <span v-if="!line.segs.length">&#8203;</span>
      </div>
      <div v-if="pending" class="ln pending">{{ pending }}</div>
      <div v-if="!lineCount() && !pending" class="term-empty">
        {{ emptyHint ?? '还没有输出。启动项目后，子 shell 的 stdout / stderr 会实时出现在这里。' }}
      </div>
    </div>
    <button v-if="!stick" class="jump" @click="scrollBottom">↓ 回到最新</button>
    <div class="term-input">
      <span>›</span>
      <input
        v-model="input"
        type="text"
        spellcheck="false"
        placeholder="向该会话 stdin 输入内容后回车（Ctrl+C 请用「停止」按钮）"
        @keyup.enter="send"
      />
      <span v-if="state.sessions[id]?.status !== 'running'" class="term-status muted">
        {{ state.sessions[id] ? statusText(state.sessions[id].status) : '未启动' }}
      </span>
    </div>
  </div>
</template>

<style scoped>
.termwrap {
  position: relative;
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1;
}
.term {
  flex: 1;
  min-height: 120px;
}
.pending {
  opacity: 0.7;
}
.term-status {
  font-size: 10.5px;
  white-space: nowrap;
  flex: none;
}
.term-empty {
  color: var(--t4);
  font-family: var(--sans);
  font-size: 11.5px;
  line-height: 1.9;
  padding: 18px 4px;
}
</style>
