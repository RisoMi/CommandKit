export type Status =
  | 'stopped'
  | 'starting'
  | 'running'
  | 'restarting'
  | 'stopping'
  | 'exited'
  | 'failed';

export interface HealthCheck {
  url: string | null;
  port: number | null;
  timeoutMs: number;
  retry: number;
  intervalMs: number;
}

export interface Policy {
  failFast: boolean;
  restartOnCrash: boolean;
  maxRestarts: number;
}

export type Join = 'and' | 'or';

export interface CommandStep {
  cmd: string;
  join: Join;
}

/** 项目：组件的分组容器 */
export interface Project {
  id: string;
  name: string;
  order?: number;
  color?: string | null;
}

/** 组件：可启动单元 */
export interface Component {
  id: string;
  name: string;
  projectId: string;
  stack: string;
  workdir: string;
  commands: CommandStep[];
  env: Record<string, string>;
  ports: number[];
  dependsOn: string[];
  health: HealthCheck | null;
  policy: Policy;
  autoStart: boolean;
  shell: string | null;
  note: string;
}

export interface TemplateCommand {
  cmd: string;
  tag: string;
}

export interface Template {
  id: string;
  name: string;
  desc: string;
  platform: string;
  shell: string | null;
  workdir: string;
  env: Record<string, string>;
  commands: TemplateCommand[];
  updatedAt: number;
}

export interface Settings {
  shell: string;
  logLinesCap: number;
  confirmBulk: boolean;
  refreshMs: number;
  persistLogs: boolean;
  sidebarCollapsed: boolean;
}

export interface SessionView {
  id: string;
  name: string;
  kind: string;
  status: Status;
  pid: number | null;
  startedAt: number | null;
  endedAt: number | null;
  exitCode: number | null;
  error: string | null;
  restarts: number;
  healthOk: boolean;
  command: string;
  workdir: string;
  ports: number[];
  projectId: string;
}

export interface MetricRow {
  id: string;
  pid: number | null;
  cpu: number;
  mem: number;
  children: number;
}

export interface ProcNode {
  pid: number;
  ppid: number | null;
  name: string;
  cmd: string;
  cpu: number;
  mem: number;
}

export interface ProcessDetail {
  id: string;
  tree: ProcNode[];
  env: [string, string][];
  listen: string[];
  logTail: string[];
}

export interface BulkView {
  kind: string;
  total: number;
  index: number;
  order: string[];
  status: Record<string, string>;
  cancelled: boolean;
  finished: boolean;
  message: string | null;
  startedAt: number;
}

export interface ShellOption {
  value: string;
  label: string;
  kind: 'pwsh' | 'powershell' | 'cmd' | 'posix';
  available: boolean;
}

export interface AppInfo {
  platform: string;
  version: string;
  dataRoot: string;
  writable: boolean;
  portable: boolean;
  shell: string;
  shells: ShellOption[];
  webview: string;
}

export interface ImportResult {
  imported: number;
  skipped: string[];
  snapshot: Snapshot;
}

export interface Snapshot {
  info: AppInfo;
  projects: Project[];
  components: Component[];
  templates: Template[];
  settings: Settings;
  sessions: SessionView[];
  metrics: Record<string, MetricRow>;
  bulk: BulkView | null;
}

export interface LogChunk {
  seq: number;
  stream: 'out' | 'err';
  data: string;
}

export interface TermChunk extends LogChunk {
  id: string;
}

export type ViewKey = 'workbench' | 'bulk' | 'monitor' | 'settings';
