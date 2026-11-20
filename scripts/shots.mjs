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
const context = await browser.newContext({
  viewport: { width: 1360, height: 880 },
  deviceScaleFactor: 2,
  colorScheme: 'dark',
  permissions: ['clipboard-read', 'clipboard-write'],
});
const page = await context.newPage();
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
const heading = await page.locator('.head h2').first().innerText();
if (heading.trim() !== '终端') throw new Error(`一键运行后没有自动切到终端页：${heading}`);
// 会话列表应按「项目」聚合：一键运行后至少有两个项目分组
const groupCount = await page.locator('.side .side-session').count();
if (groupCount < 2) throw new Error(`终端会话应按项目分组，实际只有 ${groupCount} 个`);

// 选中「前端」项目：它下面有 2 个组件，页签应跟着变成 2 个
await page.locator('.side-session', { hasText: '前端' }).first().click();
await page.waitForTimeout(300);
const tabCount = await page.locator('.tab').count();
if (tabCount !== 2) throw new Error(`「前端」项目下应有 2 个组件页签，实际 ${tabCount}`);
await page.screenshot({ path: join(OUT, '02-bulk.png') });

// 点最后一个页签：命令行与终端内容必须跟着切换
const beforeCmd = await page.locator('.cmdbar .cmdtext').first().innerText();
await page.locator('.tab').last().click();
await page.waitForTimeout(300);
const afterCmd = await page.locator('.cmdbar .cmdtext').first().innerText();
if (beforeCmd === afterCmd) {
  throw new Error(`切换页签后命令行没有变化：${afterCmd}`);
}
await page.screenshot({ path: join(OUT, '02b-tab-switch.png') });
// 回到第一个页签继续后续断言
await page.locator('.tab').first().click();
await page.waitForTimeout(200);
await page.waitForTimeout(2600);
await page.screenshot({ path: join(OUT, '03-bulk-done.png') });

// 切换标签：终端内容必须跟着换，不能残留上一个会话的缓冲
await page.locator('.tab', { hasText: 'fishery-web' }).first().click();
await page.waitForTimeout(400);
const termText = await page.locator('.term').first().innerText();
if (!termText.includes('ocean-fishery-code/web')) {
  throw new Error(`切换标签后终端内容未更新：${termText.slice(0, 120)}`);
}

// CRLF（Windows 输出）必须正常渲染成行，不能被当成空的进度条刷新吞掉
if (!termText.includes('应用启动完成')) {
  throw new Error(`CRLF 输出没有渲染出来：${JSON.stringify(termText.slice(0, 200))}`);
}

// 复制输出：剪贴板里必须拿到该会话的内容（且不含 ANSI 转义）
await page.getByRole('button', { name: '复制输出' }).click();
await page.waitForTimeout(300);
const clip = await page.evaluate(() => navigator.clipboard.readText());
if (!clip.includes('ocean-fishery-code/web') || clip.includes('\u001b[')) {
  throw new Error(`复制内容不正确：${clip.slice(0, 120)}`);
}
await page.screenshot({ path: join(OUT, '03b-term-switch.png') });

// 进程监管
await page.getByText('进程监管', { exact: true }).first().click();
await page.waitForTimeout(900);
await page.screenshot({ path: join(OUT, '04-monitor.png') });

// 环境
await page.getByText('环境', { exact: true }).first().click();
await page.waitForTimeout(500);
const envCards = await page.locator('.envcard').count();
if (envCards < 3) throw new Error(`环境页应有内置环境，实际 ${envCards} 个`);
await page.screenshot({ path: join(OUT, '04c-environments.png') });

// 从模板新建（原「模板库」已并入项目工作台）
await page.getByText('项目工作台', { exact: true }).first().click();
await page.waitForTimeout(400);
await page.getByRole('button', { name: '从模板新建' }).click();
await page.waitForTimeout(400);
const tplRows = await page.locator('.tplrow').count();
if (!tplRows) throw new Error('从模板新建弹层没有模板条目');
await page.screenshot({ path: join(OUT, '05-templates.png') });

// 模板直接运行：终端左侧应出现「模板运行」会话（曾经完全看不到）
await page.locator('.tplrow').first().getByRole('button', { name: '直接运行' }).click();
await page.waitForTimeout(700);
const tplGroup = await page.locator('.side-session', { hasText: '模板运行' }).count();
if (!tplGroup) throw new Error('模板运行后，终端里没有「模板运行」会话');
const tplTabs = await page.locator('.tab').count();
if (!tplTabs) throw new Error('模板运行后，终端没有页签');
await page.screenshot({ path: join(OUT, '05b-template-run.png') });

// 设置
await page.getByText('设置', { exact: true }).first().click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '06-settings.png') });

// 项目编辑弹层
await page.getByText('项目工作台', { exact: true }).first().click();
await page.waitForTimeout(300);
await page.getByRole('button', { name: '+ 新建组件' }).click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '07-dialog.png') });
await page.keyboard.press('Escape');

// 分类管理
await page.getByText('管理', { exact: true }).first().click();
await page.waitForTimeout(400);
await page.screenshot({ path: join(OUT, '08-categories.png') });
await page.keyboard.press('Escape');

// 浅色系统背景下 WebView 的原生控件会变白 —— 这里强制 light 再验一遍表单控件仍是深色
const lightCtx = await browser.newContext({
  viewport: { width: 1360, height: 880 },
  deviceScaleFactor: 2,
  colorScheme: 'light',
});
const lightPage = await lightCtx.newPage();
await lightPage.goto('http://127.0.0.1:4599/', { waitUntil: 'networkidle' });
await lightPage.waitForTimeout(800);
await lightPage.getByRole('button', { name: '+ 新建组件' }).click();
await lightPage.waitForTimeout(400);
await lightPage.screenshot({ path: join(OUT, '09-dialog-light.png') });

const probe = await lightPage.evaluate(() => {
  const input = document.querySelector('.modal input:not([type="checkbox"])');
  const select = document.querySelector('.modal select');
  return {
    input: input ? getComputedStyle(input).backgroundColor : 'none',
    select: select ? getComputedStyle(select).backgroundColor : 'none',
    appearance: select ? getComputedStyle(select).appearance : 'none',
  };
});
const dark = (v) => v === 'rgb(16, 18, 20)';
if (!dark(probe.input) || !dark(probe.select)) {
  throw new Error(`浅色环境下表单控件不是深色：${JSON.stringify(probe)}`);
}
console.log('form controls stay dark in light scheme:', JSON.stringify(probe));


// 设置页（下拉最多）也在浅色环境下留一张
await lightPage.keyboard.press('Escape');
await lightPage.getByText('设置', { exact: true }).first().click();
await lightPage.waitForTimeout(400);
await lightPage.screenshot({ path: join(OUT, '10-settings-light.png') });

await lightCtx.close();

console.log(errors.length ? `console errors:\n${errors.join('\n')}` : 'no console errors');
await browser.close();
server.close();
