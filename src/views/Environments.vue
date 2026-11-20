<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { api } from '../api';
import {
  apply,
  environmentUsage,
  state,
  toast,
} from '../store';
import type { Environment } from '../types';

const COLORS = ['#9EFF00', '#5B9DFF', '#E8A33D', '#B27BFF', '#FF7B7B', '#3AD6A8'];

const editing = ref<Environment | null>(null);
const rows = ref<{ k: string; v: string }[]>([]);
const busy = ref(false);

const sorted = computed(() =>
  [...state.environments].sort((a, b) => a.name.localeCompare(b.name))
);

const totalVars = computed(() =>
  state.environments.reduce((n: number, e: Environment) => n + Object.keys(e.env).length, 0)
);

function blank(): Environment {
  return { id: '', name: '', desc: '', color: COLORS[0], env: {} };
}

function openNew() {
  editing.value = blank();
  rows.value = [{ k: '', v: '' }];
}

function openEdit(e: Environment) {
  editing.value = JSON.parse(JSON.stringify(e));
  const list = Object.entries(e.env).map(([k, v]) => ({ k, v }));
  rows.value = list.length ? list : [{ k: '', v: '' }];
}

function duplicate(e: Environment) {
  editing.value = { ...JSON.parse(JSON.stringify(e)), id: '', name: `${e.name} 副本` };
  rows.value = Object.entries(e.env).map(([k, v]) => ({ k, v }));
}

function close() {
  editing.value = null;
  rows.value = [];
}

function addRow() {
  rows.value.push({ k: '', v: '' });
}

function removeRow(i: number) {
  rows.value.splice(i, 1);
  if (!rows.value.length) rows.value.push({ k: '', v: '' });
}

async function save() {
  const draft = editing.value;
  if (!draft) return;
  if (!draft.name.trim()) {
    toast('warn', '环境名称不能为空');
    return;
  }
  const env: Record<string, string> = {};
  for (const row of rows.value) {
    if (row.k.trim()) env[row.k.trim()] = row.v;
  }
  if (!Object.keys(env).length) {
    toast('warn', '至少填一个环境变量');
    return;
  }
  busy.value = true;
  try {
    const snap = await api.saveEnvironment({ ...draft, name: draft.name.trim(), env });
    apply(snap);
    close();
    toast('ok', '环境已保存');
  } catch (e) {
    toast('error', String(e));
  } finally {
    busy.value = false;
  }
}

async function remove(e: Environment) {
  const used = environmentUsage(e.id);
  if (used > 0) {
    const ok = window.confirm(
      `「${e.name}」正被 ${used} 个组件引用。删除后这些组件会回落到「不使用环境」，组件自身的覆盖变量仍保留。确定删除？`
    );
    if (!ok) return;
  }
  try {
    apply(await api.deleteEnvironment(e.id));
    toast('ok', '环境已删除');
  } catch (err) {
    toast('error', String(err));
  }
}
</script>

<template>
  <div class="view">
    <header class="head">
      <div>
        <h2>环境</h2>
        <p>
          一组可复用的环境变量。组件在「编辑组件 → 环境」里引用它，环境改动对所有引用它的组件同时生效；
          组件自身的同名变量作为覆盖优先。
        </p>
      </div>
      <div class="actions">
        <button class="btn primary" @click="openNew">+ 新建环境</button>
      </div>
    </header>

    <div class="summary">
      <div><span>环境数</span><strong>{{ state.environments.length }}</strong></div>
      <div><span>变量总数</span><strong>{{ totalVars }}</strong></div>
      <div>
        <span>被引用</span>
        <strong>{{ state.components.filter((c) => c.envProfile).length }} 个组件</strong>
      </div>
    </div>

    <div class="grid">
      <div v-for="e in sorted" :key="e.id" class="proj envcard">
        <div class="prow">
          <span class="cdot" :style="{ background: e.color ?? '#62686d' }"></span>
          <span class="pname">{{ e.name }}</span>
          <span class="tag">{{ Object.keys(e.env).length }} 个变量</span>
          <span class="tag">{{ environmentUsage(e.id) }} 个组件</span>
        </div>
        <div class="pmeta">{{ e.desc || '—' }}</div>
        <div class="envpreview mono">
          <div v-for="(v, k) in e.env" :key="k" class="envrow">
            <span class="ek">{{ k }}</span>
            <span class="ev">{{ v }}</span>
          </div>
        </div>
        <div class="pops">
          <span class="pop" @click="openEdit(e)">编辑</span>
          <span class="pop" @click="duplicate(e)">复制</span>
          <span class="pop stop" @click="remove(e)">删除</span>
        </div>
      </div>
    </div>

    <div v-if="!sorted.length" class="empty">
      还没有环境。<span class="link" @click="openNew">新建一个</span>，之后在组件里就能直接选。
    </div>

    <div v-if="editing" class="mask" @click.self="close">
      <div class="modal">
        <div class="modal-h">
          <h3>{{ editing.id ? `编辑环境 · ${editing.name}` : '新建环境' }}</h3>
          <span class="x" @click="close">✕</span>
        </div>
        <div class="modal-b">
          <div class="row">
            <div class="field" style="flex: 2">
              <label>环境名称 *</label>
              <input v-model="editing.name" placeholder="Java 17 / Python UTF-8" />
            </div>
            <div class="field" style="flex: 2">
              <label>说明</label>
              <input v-model="editing.desc" placeholder="给团队成员看的用途说明" />
            </div>
          </div>

          <div class="field">
            <label>颜色</label>
            <div class="flex" style="gap: 8px">
              <button
                v-for="c in COLORS"
                :key="c"
                class="swatch"
                :class="{ on: editing.color === c }"
                :style="{ background: c }"
                @click="editing.color = c"
              ></button>
            </div>
          </div>

          <div class="section-t">变量 *</div>
          <div v-for="(row, i) in rows" :key="i" class="row" style="margin-bottom: 6px">
            <input v-model="row.k" style="flex: 0 0 220px" placeholder="KEY" />
            <input v-model="row.v" placeholder="VALUE（支持 %PATH% / $PATH 引用现有环境）" />
            <button class="btn xs" @click="removeRow(i)">删除</button>
          </div>
          <div>
            <button class="btn xs" @click="addRow">+ 添加变量</button>
          </div>
          <span class="hint">这里的变量会注入到引用该环境的组件子进程，不修改系统环境与 shell 配置。</span>
        </div>
        <div class="modal-f">
          <span class="spacer"></span>
          <button class="btn" @click="close">取消</button>
          <button class="btn primary" :disabled="busy" @click="save">保存</button>
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
.cdot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex: none;
}
.envcard {
  gap: 8px;
}
.envpreview {
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 92px;
  overflow: auto;
  border-top: 1px solid var(--stroke-soft);
  border-bottom: 1px solid var(--stroke-soft);
  padding: 7px 0;
}
.envrow {
  display: flex;
  gap: 8px;
  font-size: 10.5px;
  min-width: 0;
}
.ek {
  color: var(--t3);
  flex: 0 0 auto;
}
.ev {
  color: var(--t4);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.swatch {
  width: 20px;
  height: 20px;
  border-radius: 5px;
  border: 2px solid transparent;
  cursor: pointer;
  padding: 0;
}
.swatch.on {
  border-color: var(--t1);
}
</style>
