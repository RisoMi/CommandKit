import { computed, reactive } from 'vue';
import { listen } from './bridge';
import { api } from './api';
import { parseAnsi, stripAnsi, type Seg } from './ansi';
import type {
  AppInfo,
  BulkView,
  Category,
  LogChunk,
  MetricRow,
  Project,
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
  categories: [] as Category[],
  templates: [] as Template[],
  settings: null as Settings | null,
  sessions: {} as Record<string, SessionView>,
  metrics: {} as Record<string, MetricRow>,
  bulk: null as BulkView | null,
  view: 'workbench' as ViewKey,
  activeCategory: 'all',
  activeSession: null as string | null,
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
  project: null as Project | null,
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
  // 进度条常用 \r 原地刷新，只保留最后一段
  const clean = raw.includes('\r') ? raw.slice(raw.lastIndexOf('\r') + 1) : raw;
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
  state.categories = snap.categories;
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

export const visibleProjects = computed(() => {
  if (state.activeCategory === 'all') return state.projects;
  return state.projects.filter((p) => p.category === state.activeCategory);
});

export const batchIds = computed(() => {
  if (state.selected.length) return [...state.selected];
  return visibleProjects.value.map((p) => p.id);
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

export function categoryName(id: string): string {
  if (id === 'template') return '模板运行';
  return state.categories.find((c) => c.id === id)?.name ?? '全部项目';
}

export function categoryGroups() {
  const groups: { id: string; name: string; projects: Project[] }[] = [];
  const order = ['all', ...state.categories.map((c) => c.id)];
  const list = visibleProjects.value;
  for (const p of list) {
    const key = p.category || 'all';
    let g = groups.find((x) => x.id === key);
    if (!g) {
      g = { id: key, name: key === 'all' ? '未分类' : categoryName(key), projects: [] };
      groups.push(g);
    }
    g.projects.push(p);
  }
  if (state.activeCategory !== 'all') {
    return [{ id: state.activeCategory, name: categoryName(state.activeCategory), projects: list }].filter(
      (g) => g.projects.length
    );
  }
  groups.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
  return groups;
}

export function projectById(id: string): Project | undefined {
  return state.projects.find((p) => p.id === id);
}

export function sessionIds(): string[] {
  const ids = new Set<string>();
  for (const p of state.projects) ids.add(p.id);
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

export async function runProject(id: string) {
  state.busy[id] = true;
  const r = await act(() => api.runProject(id), `启动 ${projectById(id)?.name ?? id}`);
  delete state.busy[id];
  return r;
}

export async function stopProject(id: string) {
  state.busy[id] = true;
  await act(() => api.stopProject(id), `停止 ${projectById(id)?.name ?? id}`);
  delete state.busy[id];
}

export async function restartProject(id: string) {
  state.busy[id] = true;
  await act(() => api.restartProject(id), '重启');
  window.setTimeout(() => delete state.busy[id], 1200);
}

export async function toggleRun(id: string) {
  const st = statusOf(id);
  if (st === 'running' || st === 'starting' || st === 'restarting') return stopProject(id);
  return runProject(id);
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
      detail: ids.map((id) => projectById(id)?.name ?? id).join(' · '),
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

export function newProject(): Project {
  const cat = state.activeCategory === 'all' ? state.categories[1]?.id ?? 'all' : state.activeCategory;
  return {
    id: '',
    name: '',
    category: cat,
    stack: '',
    workdir: '',
    command: '',
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

export function openEditor(project?: Project) {
  editor.project = project ? JSON.parse(JSON.stringify(project)) : newProject();
  editor.isNew = !project;
  editor.open = true;
}

export function closeEditor() {
  editor.open = false;
  editor.project = null;
}

export async function saveProjectFromEditor() {
  if (!editor.project) return;
  const p = editor.project as Project;
  try {
    const snap = await api.saveProject(p);
    applySnapshot(snap);
    closeEditor();
    toast('ok', `已保存 ${p.name}`);
  } catch (e) {
    toast('error', err(e));
  }
}

export async function deleteProjectById(id: string) {
  const p = projectById(id);
  const ok = await askConfirm({
    title: '删除项目',
    message: `确认删除「${p?.name ?? id}」？该操作不可撤销（配置有自动备份）。`,
    okText: '删除',
    danger: true,
  });
  if (!ok) return;
  const r = await act(() => api.deleteProject(id), '删除项目');
  if (r) {
    applySnapshot(r);
    toast('ok', '已删除');
  }
}

export function apply(snap: Snapshot) {
  applySnapshot(snap);
}
