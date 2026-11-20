/**
 * 浏览器演示适配层（仅在缺少 Tauri 运行时环境时启用）。
 *
 * 用途：`dist/index.html` 可以直接在浏览器打开做视觉走查 / 设计稿对照，
 * 不会启动真实进程，数据是内存里的示例集。在 Tauri 窗口内永远不会走到这里。
 */

import type {
  BulkView,
  CommandStep,
  Component,
  Environment,
  LogChunk,
  MetricRow,
  ProcessDetail,
  Project,
  SessionView,
  Settings,
  Snapshot,
  Template,
} from './types';

type Handler = (ev: { payload: unknown }) => void;
const listeners = new Map<string, Set<Handler>>();

export async function listen(event: string, cb: Handler) {
  const set = listeners.get(event) ?? new Set<Handler>();
  set.add(cb);
  listeners.set(event, set);
  return () => {
    set.delete(cb);
  };
}

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((fn) => fn({ payload }));
}

const now = () => Date.now();
const uid = (p: string) => `${p}${now().toString(36)}`;

const projects: Project[] = [
  { id: 'all', name: '全部项目', order: 0, color: null },
  { id: 'frontend', name: '前端', order: 1, color: '#5B9DFF' },
  { id: 'backend', name: '后端', order: 2, color: '#9EFF00' },
  { id: 'middleware', name: '中间件', order: 3, color: '#E8A33D' },
  { id: 'data', name: '数据服务', order: 4, color: '#B27BFF' },
];

const settings: Settings = {
  shell: '/bin/zsh',
  logLinesCap: 3000,
  confirmBulk: true,
  refreshMs: 2000,
  persistLogs: true,
  sidebarCollapsed: false,
};

function mkComponent(p: Partial<Component> & { id: string; name: string }): Component {
  return {
    projectId: 'all',
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
    ...p,
  };
}

const environments: Environment[] = [
  {
    id: 'env-java17',
    name: 'Java 17',
    desc: 'JDK 17 + JAVA_HOME 注入',
    color: '#E8A33D',
    env: {
      JAVA_HOME: '/Library/Java/JavaVirtualMachines/jdk-17.jdk/Contents/Home',
      PATH: '$JAVA_HOME/bin:$PATH',
    },
  },
  {
    id: 'env-python',
    name: 'Python UTF-8',
    desc: '强制 UTF-8 输出，避免中文乱码',
    color: '#5B9DFF',
    env: { PYTHONUTF8: '1', PYTHONIOENCODING: 'utf-8' },
  },
  {
    id: 'env-node',
    name: 'Node 大内存',
    desc: '前端构建时放宽堆上限',
    color: '#9EFF00',
    env: { NODE_OPTIONS: '--max-old-space-size=4096' },
  },
];

const components: Component[] = [
  mkComponent({
    id: 'fishery-web',
    name: 'fishery-web',
    projectId: 'frontend',
    stack: 'npm dev',
    workdir: '~/work/ocean-fishery-code/web',
    commands: [{ cmd: 'npm run dev -- --port 5173', join: 'and' }],
    ports: [5173],
    dependsOn: ['fishery-agent'],
  }),
  mkComponent({
    id: 'admin-vue',
    name: 'admin-vue',
    projectId: 'frontend',
    stack: 'vite',
    workdir: '~/work/admin-vue',
    commands: [{ cmd: 'npm run dev -- --port 5174', join: 'and' }],
    ports: [5174],
    dependsOn: ['fishery-agent'],
  }),
  mkComponent({
    id: 'fishery-agent',
    name: 'fishery-agent',
    projectId: 'backend',
    stack: 'Python 3.11',
    workdir: '~/work/fishery-agent',
    commands: [{ cmd: 'py -3.11 -m uvicorn main:app --host 0.0.0.0 --port 8000', join: 'and' }],
    env: {
      PYTHON_HOME: '~/pyenv/versions/3.11',
      OLLAMA_HOST: 'http://127.0.0.1:11434',
      JWT_SECRET: 'ocean-data-system-jwt-secret-key-2024',
    },
    ports: [8000],
    dependsOn: ['redis-local', 'fishery-backend-java'],
    health: { url: 'http://127.0.0.1:8000/health', port: null, timeoutMs: 60000, retry: 30, intervalMs: 1000 },
  }),
  mkComponent({
    envProfile: 'env-java17',
    id: 'fishery-backend-java',
    name: 'fishery-backend-java',
    projectId: 'backend',
    stack: 'Java 17',
    workdir: '~/work/fishery-backend-java',
    commands: [{ cmd: './gradlew bootRun --args=--server.port=8080', join: 'and' }],
    ports: [8080],
    dependsOn: ['redis-local'],
  }),
  mkComponent({
    id: 'redis-local',
    name: 'redis-local',
    projectId: 'middleware',
    stack: 'Redis 7',
    workdir: '~/Tools/redis',
    commands: [{ cmd: 'redis-server --port 6379 --save ""', join: 'and' }],
    ports: [6379],
  }),
  mkComponent({
    id: 'postgres-local',
    name: 'postgres-local',
    projectId: 'data',
    stack: 'PostgreSQL',
    workdir: '~/Tools/pgsql',
    commands: [{ cmd: 'pg_ctl -D ./data start', join: 'and' }],
    ports: [5432],
  }),
];

const templates: Template[] = [
  {
    id: 'tpl-webgis',
    name: 'WebGIS 服务启动',
    desc: '为 fishery-agent 准备 JDK 17、Python 与 Maven 构建环境，切目录后按序执行版本校验与构建。',
    platform: 'macos',
    shell: null,
    workdir: '~/work/fishery-agent',
    env: {
      JAVA_HOME: '/Library/Java/JavaVirtualMachines/jdk-17.jdk/Contents/Home',
      MAVEN_HOME: '/opt/homebrew/opt/maven',
      PATH: '$PATH:$JAVA_HOME/bin:$MAVEN_HOME/bin',
    },
    commands: [
      { cmd: 'java -version', tag: '前置校验' },
      { cmd: 'python3 --version', tag: '前置校验' },
      { cmd: 'mvn -v', tag: '前置校验' },
      { cmd: './gradlew clean build', tag: '构建' },
    ],
    updatedAt: now() - 86400000,
  },
  {
    id: 'tpl-node',
    name: 'Node 前端构建',
    desc: '锁定 Node 版本、清理缓存后构建，产物输出到 dist。',
    platform: 'macos',
    shell: null,
    workdir: '~/work/ocean-fishery-code/web',
    env: {},
    commands: [
      { cmd: 'node -v', tag: '校验' },
      { cmd: 'npm ci', tag: '依赖' },
      { cmd: 'npm run build', tag: '构建' },
    ],
    updatedAt: now() - 3600000,
  },
];

const t0 = now();
const sessions: Record<string, SessionView> = {
  'fishery-web': {
    id: 'fishery-web',
    name: 'fishery-web',
    kind: 'project',
    status: 'running',
    pid: 18422,
    startedAt: t0 - 5046000,
    endedAt: null,
    exitCode: null,
    error: null,
    restarts: 0,
    healthOk: true,
    command: 'npm run dev -- --port 5173',
    workdir: '~/work/ocean-fishery-code/web',
    ports: [5173],
    projectId: 'frontend',
  },
  'admin-vue': {
    id: 'admin-vue',
    name: 'admin-vue',
    kind: 'project',
    status: 'starting',
    pid: null,
    startedAt: t0 - 18000,
    endedAt: null,
    exitCode: null,
    error: null,
    restarts: 0,
    healthOk: false,
    command: 'npm run dev -- --port 5174',
    workdir: '~/work/admin-vue',
    ports: [5174],
    projectId: 'frontend',
  },
  'fishery-agent': {
    id: 'fishery-agent',
    name: 'fishery-agent',
    kind: 'project',
    status: 'running',
    pid: 18104,
    startedAt: t0 - 5201000,
    endedAt: null,
    exitCode: null,
    error: null,
    restarts: 0,
    healthOk: true,
    command: 'py -3.11 -m uvicorn main:app --host 0.0.0.0 --port 8000',
    workdir: '~/work/fishery-agent',
    ports: [8000],
    projectId: 'backend',
  },
  'fishery-backend-java': {
    id: 'fishery-backend-java',
    name: 'fishery-backend-java',
    kind: 'project',
    status: 'running',
    pid: 17980,
    startedAt: t0 - 5292000,
    endedAt: null,
    exitCode: null,
    error: null,
    restarts: 0,
    healthOk: true,
    command: './gradlew bootRun --args=--server.port=8080',
    workdir: '~/work/fishery-backend-java',
    ports: [8080],
    projectId: 'backend',
  },
  'redis-local': {
    id: 'redis-local',
    name: 'redis-local',
    kind: 'project',
    status: 'running',
    pid: 17502,
    startedAt: t0 - 5515000,
    endedAt: null,
    exitCode: null,
    error: null,
    restarts: 0,
    healthOk: true,
    command: 'redis-server --port 6379 --save ""',
    workdir: '~/Tools/redis',
    ports: [6379],
    projectId: 'middleware',
  },
  'postgres-local': {
    id: 'postgres-local',
    name: 'postgres-local',
    kind: 'project',
    status: 'stopped',
    pid: null,
    startedAt: null,
    endedAt: null,
    exitCode: 0,
    error: null,
    restarts: 0,
    healthOk: false,
    command: 'pg_ctl -D ./data start',
    workdir: '~/Tools/pgsql',
    ports: [5432],
    projectId: 'data',
  },
};

const metrics: Record<string, MetricRow> = {
  'fishery-web': { id: 'fishery-web', pid: 18422, cpu: 3.2, mem: 724 * 1024 * 1024, children: 4 },
  'admin-vue': { id: 'admin-vue', pid: null, cpu: 0, mem: 0, children: 0 },
  'fishery-agent': { id: 'fishery-agent', pid: 18104, cpu: 5.1, mem: 386 * 1024 * 1024, children: 2 },
  'fishery-backend-java': { id: 'fishery-backend-java', pid: 17980, cpu: 2.8, mem: 612 * 1024 * 1024, children: 1 },
  'redis-local': { id: 'redis-local', pid: 17502, cpu: 1.3, mem: 42 * 1024 * 1024, children: 0 },
  'postgres-local': { id: 'postgres-local', pid: null, cpu: 0, mem: 0, children: 0 },
};

const logs: Record<string, LogChunk[]> = {
  'fishery-agent': [
    { seq: 1, stream: 'out', data: '\x1b[90m# 独立会话 · cwd=~/work/fishery-agent · env 由 Rust 注入\x1b[0m\n' },
    { seq: 2, stream: 'out', data: '\x1b[32m$\x1b[0m py -3.11 -m uvicorn main:app --host 0.0.0.0 --port 8000\n' },
    { seq: 3, stream: 'out', data: '\x1b[90m[13:02:41] INFO  Loaded env: PYTHON_HOME, OLLAMA_HOST, JWT_SECRET\x1b[0m\n' },
    { seq: 4, stream: 'out', data: '\x1b[90m[13:02:42] INFO  Connecting redis://127.0.0.1:6379 ... \x1b[0m\x1b[32mok\x1b[0m\n' },
    { seq: 5, stream: 'out', data: '\x1b[90m[13:02:43] INFO  LangGraph graph compiled: 7 nodes, 9 edges\x1b[0m\n' },
    { seq: 6, stream: 'out', data: '\x1b[32m[13:02:44] INFO  Application startup complete.\x1b[0m\n' },
    { seq: 7, stream: 'out', data: '\x1b[33m[13:03:10] WARN 上游 fishery-backend-java 健康检查重试 1/3\x1b[0m\n' },
    { seq: 8, stream: 'out', data: '\x1b[32m[13:03:12] INFO  Upstream ready, health check passed\x1b[0m\n' },
    { seq: 9, stream: 'out', data: '\x1b[90m[13:04:02] INFO  127.0.0.1:51234 - "GET /health 200 OK"\x1b[0m\n' },
  ],
  'fishery-web': [],

};

let bulk: BulkView | null = null;

function snapshot(): Snapshot {
  return {
    info: {
      platform: 'macos',
      version: '1.3.0',
      dataRoot: '~/Library/Application Support/CommandKit',
      writable: true,
      portable: false,
      shell: settings.shell,
      shells: [
        { value: '/bin/zsh', label: 'zsh（macOS 默认）', kind: 'posix', available: true },
        { value: '/bin/bash', label: 'bash', kind: 'posix', available: true },
        { value: '/bin/sh', label: 'sh', kind: 'posix', available: true },
        { value: 'pwsh', label: 'PowerShell 7（pwsh，若已安装）', kind: 'pwsh', available: false },
      ],
      webview: 'WKWebView',
    },
    projects: JSON.parse(JSON.stringify(projects)),
    components: JSON.parse(JSON.stringify(components)),
    environments: JSON.parse(JSON.stringify(environments)),
    templates: JSON.parse(JSON.stringify(templates)),
    settings: { ...settings },
    sessions: Object.values(sessions).map((s) => ({ ...s })),
    metrics: JSON.parse(JSON.stringify(metrics)),
    bulk: bulk ? JSON.parse(JSON.stringify(bulk)) : null,
  };
}

function setSession(id: string, patch: Partial<SessionView>) {
  const cur = sessions[id];
  if (!cur) return;
  sessions[id] = { ...cur, ...patch };
  emit('ck:session', sessions[id]);
}

function pushLog(id: string, text: string) {
  // 故意用 CRLF：模拟 Windows 子进程输出，回归「\r 被当成进度条刷新而吞掉整行」的问题
  const crlf = text.replace(/\r?\n/g, '\r\n');
  const chunk: LogChunk = { seq: (logs[id]?.length ?? 0) + 1, stream: 'out', data: crlf };
  logs[id] = [...(logs[id] ?? []), chunk];
  emit('ck:term', { id, seq: chunk.seq, stream: 'out', data: crlf });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function startOne(id: string) {
  const p = components.find((x) => x.id === id);
  if (!p) throw new Error(`未知项目：${id}`);
  setSession(id, { status: 'starting', pid: null, error: null, startedAt: now() });
  pushLog(id, `\x1b[90m# 独立会话 · cwd=${p.workdir} · env 由 Rust 注入\x1b[0m\n`);
  pushLog(id, `\x1b[32m$\x1b[0m ${p.commands.map((c) => c.cmd).join(' 且 ')}\n`);
  await sleep(600);
  const pid = 18000 + Math.floor(Math.random() * 900);
  setSession(id, { status: 'running', pid, healthOk: true });
  metrics[id] = { id, pid, cpu: 1.2 + Math.random() * 4, mem: 180 * 1024 * 1024, children: 2 };
  pushLog(id, '\x1b[32m[ready]\x1b[0m 应用启动完成，健康检查通过\n');
}

async function stopOne(id: string) {
  setSession(id, { status: 'stopping' });
  await sleep(400);
  setSession(id, { status: 'stopped', pid: null, startedAt: null });
  metrics[id] = { id, pid: null, cpu: 0, mem: 0, children: 0 };
  pushLog(id, '\x1b[90m# 会话结束（退出码 0）\x1b[0m\n');
}

function topo(ids: string[]): string[] {
  const set = new Set(ids);
  const out: string[] = [];
  const seen = new Set<string>();
  const visit = (id: string) => {
    if (seen.has(id) || !set.has(id)) return;
    seen.add(id);
    const p = components.find((x) => x.id === id);
    p?.dependsOn.forEach(visit);
    out.push(id);
  };
  ids.forEach(visit);
  return out;
}

async function runBulk(ids: string[], kind: string) {
  const order = kind === 'stop' ? topo(ids).reverse() : topo(ids);
  bulk = {
    kind,
    total: order.length,
    index: 0,
    order,
    status: Object.fromEntries(order.map((id) => [id, 'waiting'])),
    cancelled: false,
    finished: false,
    message: null,
    startedAt: now(),
  };
  emit('ck:bulk', structuredClone(bulk));
  for (let i = 0; i < order.length; i++) {
    const id = order[i];
    bulk.index = i;
    bulk.status[id] = 'running';
    emit('ck:bulk', structuredClone(bulk));
    if (kind === 'stop') await stopOne(id);
    else await startOne(id);
    bulk.status[id] = 'done';
    emit('ck:bulk', structuredClone(bulk));
  }
  bulk.finished = true;
  emit('ck:bulk', structuredClone(bulk));
}

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const a = (args ?? {}) as Record<string, any>;
  switch (cmd) {
    case 'bootstrap':
      return snapshot() as T;
    case 'save_environment': {
      const e = a.environment as Environment;
      const i = environments.findIndex((x) => x.id === e.id);
      if (i >= 0) environments[i] = e;
      else {
        e.id = e.id || uid('env');
        environments.push(e);
      }
      return snapshot() as T;
    }
    case 'delete_environment': {
      const i = environments.findIndex((x) => x.id === a.id);
      if (i >= 0) environments.splice(i, 1);
      components.forEach((c) => {
        if (c.envProfile === a.id) c.envProfile = null;
      });
      return snapshot() as T;
    }
    case 'save_component': {
      const p = a.component as Component;
      const i = components.findIndex((x) => x.id === p.id);
      if (i >= 0) components[i] = p;
      else {
        p.id = p.id || uid('c');
        components.push(p);
      }
      return snapshot() as T;
    }
    case 'delete_component': {
      const i = components.findIndex((x) => x.id === a.id);
      if (i >= 0) components.splice(i, 1);
      return snapshot() as T;
    }
    case 'save_project': {
      const p = a.project as Project;
      const i = projects.findIndex((x) => x.id === p.id);
      if (i >= 0) projects[i] = p;
      else {
        p.id = p.id || uid('pj');
        projects.push(p);
      }
      return snapshot() as T;
    }
    case 'delete_project': {
      const i = projects.findIndex((x) => x.id === a.id);
      if (i >= 0) projects.splice(i, 1);
      components.forEach((c) => {
        if (c.projectId === a.id) c.projectId = 'all';
      });
      return snapshot() as T;
    }
    case 'save_template': {
      const t = a.template as Template;
      const i = templates.findIndex((x) => x.id === t.id);
      if (i >= 0) templates[i] = t;
      else {
        t.id = t.id || uid('tpl');
        templates.push(t);
      }
      return snapshot() as T;
    }
    case 'delete_template': {
      const i = templates.findIndex((x) => x.id === a.id);
      if (i >= 0) templates.splice(i, 1);
      return snapshot() as T;
    }
    case 'save_settings':
      Object.assign(settings, a.settings);
      return snapshot() as T;
    case 'run_component':
      void startOne(a.id);
      return undefined as T;
    case 'stop_component':
      void stopOne(a.id);
      return undefined as T;
    case 'restart_component':
      void (async () => {
        await stopOne(a.id);
        await startOne(a.id);
      })();
      return undefined as T;
    case 'run_many':
      void runBulk(a.ids as string[], 'run');
      return undefined as T;
    case 'stop_many_cmd':
      void runBulk(a.ids as string[], 'stop');
      return undefined as T;
    case 'restart_many':
      void (async () => {
        await runBulk(a.ids as string[], 'stop');
        await runBulk(a.ids as string[], 'restart');
      })();
      return undefined as T;
    case 'cancel_bulk_cmd':
      if (bulk) {
        bulk.cancelled = true;
        emit('ck:bulk', structuredClone(bulk));
      }
      return undefined as T;
    case 'export_components':
      return `data/exports/components-${Date.now()}.json` as T;
    case 'import_components': {
      const demo: Component = {
        id: uid('c'),
        name: '导入示例',
        projectId: projects[1]?.id ?? 'all',
        stack: 'imported',
        workdir: '~/work/imported',
        envProfile: null,
        commands: [{ cmd: 'echo imported', join: 'and' }],
        env: {},
        ports: [],
        dependsOn: [],
        health: null,
        policy: { failFast: true, restartOnCrash: false, maxRestarts: 2 },
        autoStart: false,
        shell: null,
        note: '演示：浏览器模式下的导入占位',
      };
      components.push(demo);
      return { imported: 1, skipped: [], snapshot: snapshot() } as T;
    }
    case 'run_template': {
      const t = templates.find((x) => x.id === a.id);
      if (!t) throw new Error('未知模板');
      const id = `tpl:${t.id}`;
      sessions[id] = {
        id,
        name: `模板 · ${t.name}`,
        kind: 'template',
        status: 'starting',
        pid: null,
        startedAt: now(),
        endedAt: null,
        exitCode: null,
        error: null,
        restarts: 0,
        healthOk: false,
        command: t.commands.map((c) => c.cmd).join(' && '),
        workdir: t.workdir,
        ports: [],
        projectId: 'template',
      };
      emit('ck:session', sessions[id]);
      pushLog(id, `\x1b[32m$\x1b[0m ${sessions[id].command}\n`);
      void (async () => {
        await sleep(700);
        setSession(id, { status: 'running', pid: 19999 });
        pushLog(id, '\x1b[32m[done]\x1b[0m 模板执行完成\n');
        await sleep(400);
        setSession(id, { status: 'stopped', pid: null });
      })();
      return undefined as T;
    }
    case 'send_input_cmd':
      pushLog(a.id, `\x1b[90m> ${a.data}\x1b[0m`);
      return undefined as T;
    case 'session_log':
      return (logs[a.id] ?? []) as T;
    case 'clear_session_log':
      logs[a.id] = [];
      return undefined as T;
    case 'process_detail': {
      const s = sessions[a.id];
      const detail: ProcessDetail = {
        id: a.id,
        tree: s?.pid
          ? [
              { pid: s.pid, ppid: 1, name: 'zsh', cmd: `-lc ${s.command}`, cpu: 0.4, mem: 12 * 1024 * 1024 },
              { pid: s.pid + 32, ppid: s.pid, name: 'node', cmd: s.command, cpu: 2.8, mem: 512 * 1024 * 1024 },
              { pid: s.pid + 44, ppid: s.pid + 32, name: 'esbuild', cmd: '--service=0.25.0', cpu: 0.3, mem: 64 * 1024 * 1024 },
            ]
          : [],
        env: Object.entries(components.find((c) => c.id === a.id)?.env ?? {}).map(([k, v]) => [
          k,
          /SECRET|TOKEN|KEY|PASSWORD/.test(k) ? '••••••••（已脱敏）' : v,
        ]),
        listen: (s?.ports ?? []).map((p: number) => `127.0.0.1:${p}  LISTEN  ${s?.pid ?? '-'}`),
        logTail: (logs[a.id] ?? [])
          .slice(-4)
          .flatMap((c) => c.data.replace(/\x1b\[[0-9;]*m/g, '').split('\n').filter(Boolean)),
      };
      return detail as T;
    }
    case 'export_log':
      return `/Users/demo/data/exports/${a.id}.log` as T;
    case 'export_snapshot':
      return '~/Library/Application Support/CommandKit/process-snapshot.json' as T;
    case 'open_data_dir':
    case 'open_workdir':
    case 'open_external_terminal':
      return undefined as T;
    default:
      throw new Error(`[mock] 未实现的命令：${cmd}`);
  }
}
