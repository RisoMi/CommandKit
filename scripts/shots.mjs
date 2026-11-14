// 用 Playwright 打开构建产物（mock 演示模式），逐屏截图用于视觉走查与设计稿对照
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright';

const ROOT = fileURLToPath(new URL('../dist/', import.meta.url));
const OUT = fileURLToPath(new URL('../shots/', import.meta.url));
mkdirSync(OUT, { recursive: true });

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
};

const server = http.createServer(async (req, res) => {
  try {
    const path = normalize(decodeURIComponent(req.url.split('?')[0]));
    const file = join(ROOT, path === '/' ? 'index.html' : path);
    const body = await readFile(file);
    res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end('not found');
  }
});

await new Promise((r) => server.listen(4599, '127.0.0.1', r));

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1360, height: 880 },
  deviceScaleFactor: 2,
  colorScheme: 'dark',
});
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(m.text());
});

await page.goto('http://127.0.0.1:4599/', { waitUntil: 'networkidle' });
await page.waitForTimeout(900);
await page.screenshot({ path: join(OUT, '01-workbench.png') });

// 一键运行 → 批量运行（运行态）
await page.getByRole('button', { name: '一键运行' }).click();
// 一键运行后应自动切到批量运行页（不能靠手动点导航）
await page.waitForTimeout(1500);
const autoSwitched = await page.getByText('批量运行 · 多终端').count();
if (!autoSwitched) throw new Error('一键运行后没有自动切到批量运行页');
await page.screenshot({ path: join(OUT, '02-bulk.png') });
await page.waitForTimeout(2600);
await page.screenshot({ path: join(OUT, '03-bulk-done.png') });

// 切换标签：终端内容必须跟着换，不能残留上一个会话的缓冲
await page.locator('.tab', { hasText: 'fishery-agent' }).first().click();
await page.waitForTimeout(400);
const termText = await page.locator('.term').first().innerText();
if (!termText.includes('fishery-agent')) {
  throw new Error(`切换标签后终端内容未更新：${termText.slice(0, 120)}`);
}
await page.screenshot({ path: join(OUT, '03b-term-switch.png') });

// 进程监管
await page.getByText('进程监管', { exact: true }).first().click();
await page.waitForTimeout(900);
await page.screenshot({ path: join(OUT, '04-monitor.png') });

// 模板库
await page.getByText('模板库', { exact: true }).first().click();
await page.waitForTimeout(500);
await page.screenshot({ path: join(OUT, '05-templates.png') });

// 设置
await page.getByText('设置', { exact: true }).first().click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '06-settings.png') });

// 项目编辑弹层
await page.getByText('项目工作台', { exact: true }).first().click();
await page.waitForTimeout(300);
await page.getByRole('button', { name: '+ 新建项目' }).click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '07-dialog.png') });
await page.keyboard.press('Escape');

// 分类管理
await page.getByText('管理', { exact: true }).first().click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '08-categories.png') });
await page.keyboard.press('Escape');

console.log(errors.length ? `console errors:\n${errors.join('\n')}` : 'no console errors');
await browser.close();
server.close();
