/**
 * 极简 ANSI SGR 解析：把子 shell 输出的着色文本转成可渲染的片段。
 * 只处理最常用的 SGR（前景色 / 加粗 / 变暗 / 斜体 / 下划线 / 反色），
 * 其余 CSI 序列（光标移动、清屏等）直接丢弃，避免在日志面板里出现乱码。
 */

export interface Seg {
  t: string;
  s: string;
}

const BASIC: Record<number, string> = {
  30: '#6b7075',
  31: '#ff6b6b',
  32: '#9eff00',
  33: '#e8a33d',
  34: '#5b9dff',
  35: '#c792ea',
  36: '#4dd0c4',
  37: '#c9cdd2',
  90: '#8b9096',
  91: '#ff9a9a',
  92: '#b6ff5c',
  93: '#f0be6a',
  94: '#7fb6ff',
  95: '#d7abff',
  96: '#6fe0d4',
  97: '#f2f3f4',
};

/**
 * 归一化一行里的回车符：
 * - `\r\n` 的行尾 `\r` 直接丢弃（否则整行会被误判为「空进度条」）
 * - 进度条式的 `50%\r60%\r` 只保留最后一次刷新的内容
 */
export function normalizeCr(raw: string): string {
  if (!raw.includes('\r')) return raw;
  const parts = raw.split('\r');
  for (let i = parts.length - 1; i >= 0; i--) {
    if (parts[i] !== '') return parts[i];
  }
  return '';
}

export function stripAnsi(text: string): string {
  return text.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "").replace(/\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, "");
}

interface SgrState {
  fg: string | null;
  bg: string | null;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  inverse: boolean;
}

function reset(s: SgrState) {
  s.fg = null;
  s.bg = null;
  s.bold = false;
  s.dim = false;
  s.italic = false;
  s.underline = false;
  s.inverse = false;
}

function styleOf(s: SgrState): string {
  const parts: string[] = [];
  let fg = s.fg;
  let bg = s.bg;
  if (s.inverse) {
    const tmp = fg ?? '#0a0b0c';
    fg = bg ?? '#c9cdd2';
    bg = tmp;
  }
  if (fg) parts.push(`color:${fg}`);
  if (bg) parts.push(`background:${bg}`);
  if (s.bold) parts.push('font-weight:600');
  if (s.dim) parts.push('opacity:.62');
  if (s.italic) parts.push('font-style:italic');
  if (s.underline) parts.push('text-decoration:underline');
  return parts.join(';');
}

function applyCode(s: SgrState, code: number) {
  if (code === 0 || Number.isNaN(code)) return reset(s);
  if (code === 1) s.bold = true;
  else if (code === 2) s.dim = true;
  else if (code === 3) s.italic = true;
  else if (code === 4) s.underline = true;
  else if (code === 7) s.inverse = true;
  else if (code === 22) {
    s.bold = false;
    s.dim = false;
  } else if (code === 23) s.italic = false;
  else if (code === 24) s.underline = false;
  else if (code === 27) s.inverse = false;
  else if (code === 39) s.fg = null;
  else if (code === 49) s.bg = null;
  else if (BASIC[code]) s.fg = BASIC[code];
  else if (code >= 40 && code <= 47) s.bg = BASIC[code - 10] ?? null;
  else if (code >= 100 && code <= 107) s.bg = BASIC[code - 10] ?? null;
}

const RE_SGR = /\x1b\[([0-9;]*)m/g;

export function parseAnsi(input: string): Seg[] {
  const segs: Seg[] = [];
  const state: SgrState = {
    fg: null,
    bg: null,
    bold: false,
    dim: false,
    italic: false,
    underline: false,
    inverse: false,
  };
  let last = 0;
  let match: RegExpExecArray | null;
  RE_SGR.lastIndex = 0;
  while ((match = RE_SGR.exec(input)) !== null) {
    if (match.index > last) {
      const text = input.slice(last, match.index);
      segs.push({ t: text, s: styleOf(state) });
    }
    const codes = match[1] === '' ? [0] : match[1].split(';').map((x) => parseInt(x, 10));
    // 处理 38;5;n / 48;5;n / 38;2;r;g;b
    for (let i = 0; i < codes.length; i++) {
      const code = codes[i];
      if ((code === 38 || code === 48) && codes[i + 1] === 5) {
        const n = codes[i + 2];
        const hex = n !== undefined ? palette256(n) : null;
        if (hex) {
          if (code === 38) state.fg = hex;
          else state.bg = hex;
        }
        i += 2;
        continue;
      }
      if ((code === 38 || code === 48) && codes[i + 1] === 2) {
        const [r, g, b] = [codes[i + 2], codes[i + 3], codes[i + 4]];
        if (r !== undefined && g !== undefined && b !== undefined) {
          const hex = `rgb(${r},${g},${b})`;
          if (code === 38) state.fg = hex;
          else state.bg = hex;
        }
        i += 4;
        continue;
      }
      applyCode(state, code);
    }
    last = RE_SGR.lastIndex;
  }
  if (last < input.length) {
    segs.push({ t: input.slice(last), s: styleOf(state) });
  }
  // 去掉其它控制序列与不可见字符
  return segs
    .map((seg) => ({ t: seg.t.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').replace(/\x1b[\]P^_].*?(\x07|\x1b\\)/g, ''), s: seg.s }))
    .filter((seg) => seg.t.length > 0);
}

function palette256(n: number): string | null {
  if (n < 16) return BASIC[n < 8 ? 30 + n : 90 + (n - 8)] ?? null;
  if (n >= 232) {
    const v = 8 + (n - 232) * 10;
    return `rgb(${v},${v},${v})`;
  }
  const i = n - 16;
  const r = Math.floor(i / 36);
  const g = Math.floor((i % 36) / 6);
  const b = i % 6;
  const cv = (x: number) => (x === 0 ? 0 : 55 + x * 40);
  return `rgb(${cv(r)},${cv(g)},${cv(b)})`;
}
