import { computed, reactive } from 'vue';
import { listen } from './bridge';
import { api } from './api';
import { normalizeCr, parseAnsi, stripAnsi, type Seg } from './ansi';
import type {
  AppInfo,
  BulkView,
  Environment,
  Project,
  LogChunk,
  MetricRow,
  Component,
  SessionView,
  Settings,
  Snapshot,
  Status,
  Template,
  TermChunk,
  ViewKey,
} from './types';

export interface TermLine {
  n: number;
  segs: Seg[];
}

interface TermState {
  lines: TermLine[];
  pending: string;
  counter: number;
  loaded: boolean;
}

interface ToastItem {
  id: number;
  level: 'ok' | 'error' | 'warn' | 'info';
  text: string;
}

export const state = reactive({
  booting: true,
  ready: false,
  info: null as AppInfo | null,
  projects: [] as Project[],
  components: [] as Component[],
  environments: [] as Environment[],
  templates: [] as Template[],
  settings: null as Settings | null,
  sessions: {} as Record<string, SessionView>,
  metrics: {} as Record<string, MetricRow>,
  bulk: null as BulkView | null,
  view: 'workbench' as ViewKey,
  activeProjectId: 'all',
  activeSession: null as string | null,
  activeGroup: null as string | null,
  activeTemplateId: null as string | null,
  selected: [] as string[],
  monitorFilter: 'all' as 'all' | 'running' | 'error' | 'stopped',
  autoRefresh: true,
  now: Date.now(),
  toasts: [] as ToastItem[],
  busy: {} as Record<string, boolean>,
});

export const editor = reactive({
  open: false,
  isNew: false,
  project: null as Component | null,
});

export const confirmState = reactive({
  open: false,
  title: '',
  message: '',
  detail: '',
  okText: '确认',
  danger: false,
});

let confirmResolver: ((v: boolean) => void) | null = null;

export function askConfirm(opts: {
  title: string;
  message: string;
  detail?: string;
  okText?: string;
  danger?: boolean;
}): Promise<boolean> {
  confirmState.title = opts.title;
  confirmState.message = opts.message;
  confirmState.detail = opts.detail ?? '';
  confirmState.okText = opts.okText ?? '确认';
  confirmState.danger = opts.danger ?? true;
  confirmState.open = true;
  return new Promise((resolve) => {
    confirmResolver = resolve;
  });
}

export function resolveConfirm(v: boolean) {
  confirmState.open = false;
  const r = confirmResolver;
  confirmResolver = null;
  r?.(v);
}

// ---------------------------------------------------------------- toast

let toastSeq = 0;
export function toast(level: ToastItem['level'], text: string) {
  const item: ToastItem = { id: ++toastSeq, level, text };
  state.toasts.push(item);
  const ttl = level === 'error' ? 6000 : 3200;
  window.setTimeout(() => {
    const idx = state.toasts.findIndex((t) => t.id === item.id);
    if (idx >= 0) state.toasts.splice(idx, 1);
  }, ttl);
}

// ---------------------------------------------------------------- 终端缓冲

export const term = reactive<Record<string, TermState>>({});
const rawQueue: Record<string, string> = {};
let flushScheduled = false;

function ensureTerm(id: string): TermState {
  let t = term[id];
  if (!t) {
    t = { lines: [], pending: '', counter: 0, loaded: false };
    term[id] = t;
  }
  return t;
}

function lineCap(): number {
  return state.settings?.logLinesCap ?? 3000;
}

function pushLine(t: TermState, raw: string) {
  // Windows 输出是 \r\n：切行后残留的 \r 必须丢掉，否则整行会被当成空的进度条刷新
  const clean = normalizeCr(raw);
  t.lines.push({ n: t.counter++, segs: clean.trim() === '' ? [] : parseAnsi(clean) });
  const cap = lineCap();
  if (t.lines.length > cap + 200) {
    t.lines.splice(0, t.lines.length - cap);
  }
}

function feed(t: TermState, data: string) {
  t.pending += data;
  const parts = t.pending.split('\n');
  t.pending = parts.pop() ?? '';
  for (const p of parts) pushLine(t, p);
  if (t.pending.length > 4096) {
    pushLine(t, t.pending);
    t.pending = '';
  }
}

function scheduleFlush() {
  if (flushScheduled) return;
  flushScheduled = true;
  requestAnimationFrame(() => {
    flushScheduled = false;
    for (const id of Object.keys(rawQueue)) {
      const data = rawQueue[id];
      delete rawQueue[id];
      const t = ensureTerm(id);
      feed(t, data);
    }
  });
}

export function appendTerm(chunk: TermChunk) {
  rawQueue[chunk.id] = (rawQueue[chunk.id] ?? '') + chunk.data;
  scheduleFlush();
}

export async function hydrateTerm(id: string) {
  const t = ensureTerm(id);
  if (t.loaded) return;
  t.loaded = true;
  try {
    const chunks: LogChunk[] = await api.sessionLog(id);
    if (!chunks.length) return;
    t.lines = [];
    t.pending = '';
    t.counter = 0;
    for (const c of chunks) feed(t, c.data);
  } catch {
    /* 会话可能已销毁 */
  }
}

export function termText(id: string): string {
  const t = term[id];
  if (!t) return '';
  return t.lines.map((l) => stripAnsi(l.segs.map((s) => s.t).join(''))).join('\n');
}

export async function clearTerm(id: string) {
  const t = ensureTerm(id);
  t.lines = [];
  t.pending = '';
  t.counter = 0;
  await api.clearLog(id).catch(() => undefined);
}

// ---------------------------------------------------------------- 读取 / 订阅

function applySnapshot(snap: Snapshot) {
  state.info = snap.info;
  state.projects = snap.projects;
  state.components = snap.components;
  state.environments = snap.environments;
  state.templates = snap.templates;
  state.settings = snap.settings;
  state.metrics = snap.metrics;
  state.bulk = snap.bulk;
  const map: Record<string, SessionView> = {};
  for (const s of snap.sessions) map[s.id] = s;
  state.sessions = map;
  if (
    state.activeSession &&
    !snap.sessions.some((s) => s.id === state.activeSession) &&
    !snap.projects.some((p) => p.id === state.activeSession)
  ) {
    state.activeSession = null;
  }
}

export async function refresh() {
  applySnapshot(await api.bootstrap());
}

export async function bootstrapApp() {
  try {
    await refresh();
    state.ready = true;
  } catch (e) {
    toast('error', `初始化失败：${e}`);
  } finally {
    state.booting = false;
  }
  window.setInterval(() => (state.now = Date.now()), 1000);

  await listen<SessionView>('ck:session', (ev) => {
    state.sessions[ev.payload.id] = ev.payload;
  });
  await listen<TermChunk>('ck:term', (ev) => appendTerm(ev.payload));
  await listen<Record<string, MetricRow>>('ck:metrics', (ev) => {
    state.metrics = ev.payload;
  });
  await listen<BulkView>('ck:bulk', (ev) => {
    state.bulk = ev.payload;
  });
  await listen<{ level: ToastItem['level']; text: string }>('ck:toast', (ev) => {
    toast(ev.payload.level, ev.payload.text);
  });
  await listen<{ id: string; ok: boolean; attempts: number }>('ck:health', (ev) => {
    if (!ev.payload.ok && ev.payload.attempts === 1) {
      const name = state.sessions[ev.payload.id]?.name ?? ev.payload.id;
      toast('warn', `${name}：健康检查重试中`);
    }
  });
}

// ---------------------------------------------------------------- 选择器

export const visibleComponents = computed(() => {
  if (state.activeProjectId === 'all') return state.components;
  return state.components.filter((c) => (c.projectId || 'all') === state.activeProjectId);
});

export const batchIds = computed(() => {
  if (state.selected.length) return [...state.selected];
  return visibleComponents.value.map((p) => p.id);
});

export function sessionOf(id: string): SessionView | null {
  return state.sessions[id] ?? null;
}

export function statusOf(id: string): Status {
  return state.sessions[id]?.status ?? 'stopped';
}

export function metricOf(id: string): MetricRow {
  return state.metrics[id] ?? { id, pid: null, cpu: 0, mem: 0, children: 0 };
}

/** 环境实体 */
export function environmentById(id: string | null | undefined): Environment | undefined {
  if (!id) return undefined;
  return state.environments.find((e) => e.id === id);
}

/** 引用了某个环境的组件数 */
export function environmentUsage(id: string): number {
  return state.components.filter((c) => c.envProfile === id).length;
}

/** 组件最终生效的环境变量：环境打底 + 自身覆盖 */
export function resolvedEnv(c: Component): Record<string, string> {
  const out: Record<string, string> = {};
  const profile = environmentById(c.envProfile);
  if (profile) Object.assign(out, profile.env);
  Object.assign(out, c.env ?? {});
  return out;
}

export function projectName(id: string): string {
  if (id === 'template') return '模板运行';
  if (id === 'all') return '未归属项目';
  return state.projects.find((p) => p.id === id)?.name ?? id;
}

/** 某个项目下的组件 */
export interface TerminalGroup {
  id: string;
  name: string;
  components: string[];
}

/** 终端页左侧「会话」列表：按项目聚合当前批量 / 已有会话 */
export function terminalGroups(): TerminalGroup[] {
  const out: TerminalGroup[] = [];
  const add = (componentId: string) => {
    const c = componentById(componentId);
    const session = state.sessions[componentId];
    const pid = c?.projectId ?? session?.projectId ?? 'all';
    let g = out.find((x) => x.id === pid);
    if (!g) {
      g = { id: pid, name: projectName(pid), components: [] };
      out.push(g);
    }
    if (!g.components.includes(componentId)) g.components.push(componentId);
  };
  const b = state.bulk;
  if (b && !b.finished) b.order.forEach(add);
  for (const s of Object.values(state.sessions)) add(s.id);
  return out;
}

/** 某个分组里最「活跃」的状态，用于会话列表的状态点 */
export function groupStatus(group: TerminalGroup): Status {
  const rank: Status[] = ['running', 'starting', 'restarting', 'failed', 'stopping', 'stopped', 'exited'];
  for (const st of rank) {
    if (group.components.some((id) => (state.sessions[id]?.status ?? 'stopped') === st)) return st;
  }
  return 'stopped';
}

export function componentsOfProject(projectId: string): Component[] {
  return state.components.filter((c) => (c.projectId || 'all') === projectId);
}

/** 有组件的项目 id（用于终端页的会话列表） */
export function projectIdsInPlay(): string[] {
  const ids = new Set<string>();
  for (const c of state.components) ids.add(c.projectId || 'all');
  return [...ids];
}

/** 按项目分组（未分类归到「未归属项目」） */
export function componentGroups() {
  const groups: { id: string; name: string; components: Component[] }[] = [];
  const order = ['all', ...state.projects.map((p) => p.id)];
  const list = visibleComponents.value;

  if (state.activeProjectId !== 'all') {
    return [
      {
        id: state.activeProjectId,
        name: projectName(state.activeProjectId),
        components: list,
      },
    ].filter((g) => g.components.length);
  }

  for (const c of list) {
    const key = c.projectId || 'all';
    let g = groups.find((x) => x.id === key);
    if (!g) {
      g = { id: key, name: key === 'all' ? '未归属项目' : projectName(key), components: [] };
      groups.push(g);
    }
    g.components.push(c);
  }
  groups.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
  return groups;
}

export function componentById(id: string): Component | undefined {
  return state.components.find((p) => p.id === id);
}

export function sessionIds(): string[] {
  const ids = new Set<string>();
  for (const p of state.components) ids.add(p.id);
  for (const s of Object.values(state.sessions)) ids.add(s.id);
  return [...ids];
}

// ---------------------------------------------------------------- 动作

function err(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

export async function act<T>(fn: () => Promise<T>, label?: string): Promise<T | undefined> {
  try {
    return await fn();
  } catch (e) {
    toast('error', label ? `${label}：${err(e)}` : err(e));
    return undefined;
  }
}

export async function runComponent(id: string) {
  state.busy[id] = true;
  const r = await act(() => api.runComponent(id), `启动 ${componentById(id)?.name ?? id}`);
  delete state.busy[id];
  return r;
}

export async function stopComponent(id: string) {
  state.busy[id] = true;
  await act(() => api.stopComponent(id), `停止 ${componentById(id)?.name ?? id}`);
  delete state.busy[id];
}

export async function restartComponent(id: string) {
  state.busy[id] = true;
  await act(() => api.restartComponent(id), '重启');
  window.setTimeout(() => delete state.busy[id], 1200);
}

export async function toggleRun(id: string) {
  const st = statusOf(id);
  if (st === 'running' || st === 'starting' || st === 'restarting') return stopComponent(id);
  return runComponent(id);
}

export async function runBatch(kind: 'run' | 'stop' | 'restart') {
  const ids = batchIds.value;
  if (!ids.length) {
    toast('warn', '当前范围没有项目');
    return;
  }
  if (kind !== 'run' && state.settings?.confirmBulk) {
    const verb = kind === 'stop' ? '停止' : '重启';
    const ok = await askConfirm({
      title: `一键${verb}`,
      message: `将对 ${ids.length} 个项目执行${verb}，停止 / 重启按依赖逆序终止整棵进程树。`,
      detail: ids.map((id) => componentById(id)?.name ?? id).join(' · '),
      okText: `确认${verb}`,
      danger: true,
    });
    if (!ok) return;
  }
  const fn = kind === 'run' ? api.runMany : kind === 'stop' ? api.stopMany : api.restartMany;
  // act() 透传的是 void，用哨兵值判断是否成功
  const ok = await act(async () => {
    await fn(ids);
    return true;
  }, kind === 'run' ? '一键运行' : kind === 'stop' ? '一键停止' : '一键重启');
  if (ok) state.view = 'bulk';
}

export function openBulk(sessionId: string) {
  state.activeSession = sessionId;
  state.activeGroup = componentById(sessionId)?.projectId ?? state.sessions[sessionId]?.projectId ?? 'all';
  state.view = 'bulk';
  hydrateTerm(sessionId);
}

export function toggleSelect(id: string) {
  const i = state.selected.indexOf(id);
  if (i >= 0) state.selected.splice(i, 1);
  else state.selected.push(id);
}

export function clearSelection() {
  state.selected = [];
}

export function newComponent(): Component {
  const pid = state.activeProjectId === 'all' ? state.projects[1]?.id ?? 'all' : state.activeProjectId;
  return {
    id: '',
    name: '',
    projectId: pid,
    stack: '',
    workdir: '',
    envProfile: null,
    commands: [],
    env: {},
    ports: [],
    dependsOn: [],
    health: null,
    policy: { failFast: true, restartOnCrash: false, maxRestarts: 2 },
    autoStart: false,
    shell: null,
    note: '',
  };
}

export function openEditor(project?: Component) {
  editor.project = project ? JSON.parse(JSON.stringify(project)) : newComponent();
  editor.isNew = !project;
  editor.open = true;
}

export function closeEditor() {
  editor.open = false;
  editor.project = null;
}

export async function saveComponentFromEditor() {
  if (!editor.project) return;
  const p = editor.project as Component;
  try {
    const snap = await api.saveComponent(p);
    applySnapshot(snap);
    closeEditor();
    toast('ok', `已保存 ${p.name}`);
  } catch (e) {
    toast('error', err(e));
  }
}

export async function deleteComponentById(id: string) {
  const p = componentById(id);
  const ok = await askConfirm({
    title: '删除项目',
    message: `确认删除「${p?.name ?? id}」？该操作不可撤销（配置有自动备份）。`,
    okText: '删除',
    danger: true,
  });
  if (!ok) return;
  const r = await act(() => api.deleteComponent(id), '删除项目');
  if (r) {
    applySnapshot(r);
    toast('ok', '已删除');
  }
}

export function apply(snap: Snapshot) {
  applySnapshot(snap);
}
