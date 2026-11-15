export function fmtUptime(startedAt: number | null | undefined, now: number): string {
  if (!startedAt) return '—';
  const sec = Math.max(0, Math.floor((now - startedAt) / 1000));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}

export function fmtMem(bytes: number): string {
  if (!bytes) return '—';
  if (bytes > 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}

export function fmtCpu(v: number): string {
  if (!v) return '0%';
  return `${v.toFixed(1)}%`;
}

export function fmtTime(ms: number | null | undefined): string {
  if (!ms) return '—';
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const STATUS_TEXT: Record<string, string> = {
  stopped: '已停止',
  starting: '启动中',
  running: '运行中',
  restarting: '重启中',
  stopping: '停止中',
  exited: '已退出',
  failed: '异常',
};

export function statusText(s: string): string {
  return STATUS_TEXT[s] ?? s;
}

export function portText(ports: number[]): string {
  if (!ports.length) return '';
  return ports.map((p) => `:${p}`).join(' ');
}

export type ShellKind = 'pwsh' | 'powershell' | 'cmd' | 'posix';

/** 与 Rust 侧 shell_kind 保持同一规则 */
export function shellKind(shell: string): ShellKind {
  const base = (shell.split(/[\\/]/).pop() ?? shell).toLowerCase().replace(/\.exe$/, '');
  if (base.includes('pwsh')) return 'pwsh';
  if (base.includes('powershell')) return 'powershell';
  if (base === 'cmd') return 'cmd';
  return 'posix';
}

/** 给人看的「实际执行形式」，与 Rust shell_invocation 对齐 */
export function shellInvocation(shell: string, command: string): string {
  const cmd = command.length > 90 ? `${command.slice(0, 90)}…` : command;
  const kind = shellKind(shell);
  if (kind === 'cmd') return `${shell} /D /C "${cmd}"`;
  if (kind === 'posix') return `${shell} -lc "${cmd}"`;
  return `${shell} -NoProfile -Command "${cmd}"`;
}

export function shellShortName(shell: string): string {
  const base = shell.split(/[\\/]/).pop() ?? shell;
  const kind = shellKind(shell);
  if (kind === 'pwsh') return 'PowerShell 7';
  if (kind === 'powershell') return 'Windows PowerShell';
  if (kind === 'cmd') return 'CMD';
  return base.replace(/\.exe$/, '');
}
