#!/usr/bin/env node
/**
 * CI 里的 cargo test 包装器：
 * - 跨平台拿到 cargo（不依赖 pwsh / bash 的 PATH 差异）
 * - 失败时把关键行打成 ::error:: 工作流命令 → 生成 check-run annotation
 *   （公开仓库不登录也能在 commit / PR 页面看到失败原因）
 */
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const args = [
  'test',
  '--manifest-path',
  'src-tauri/Cargo.toml',
  '--tests',
  '--',
  '--test-threads=1',
];

const res = spawnSync('cargo', args, {
  encoding: 'utf8',
  shell: true,
  maxBuffer: 128 * 1024 * 1024,
  env: process.env,
});

const out = `${res.stdout ?? ''}${res.stderr ?? ''}`;
writeFileSync('cargo-test.log', out);
const code = res.status ?? 1;

const summarize = out
  .split(/\r?\n/)
  .filter((l) => l.startsWith('test result') || l.startsWith('running '))
  .join('\n');
console.log(summarize || out.slice(-2000));

if (code !== 0) {
  const key = out
    .split(/\r?\n/)
    .filter((l) =>
      /^error(\[|:)|panicked at|^test .* FAILED|^---- .* stdout ----|assertion .* failed|^failures:$/.test(l)
    );

  for (const line of key.slice(0, 40)) {
    console.log(`::error::${line.replace(/[\r\n]+/g, ' ').slice(0, 400)}`);
  }
  if (!key.length) {
    console.log(`::error::cargo test 退出码 ${code}（${process.platform}），未匹配到错误行，见下方日志尾部`);
  }
  console.log('--- cargo test 日志尾部 ---');
  console.log(out.split(/\r?\n/).slice(-120).join('\n'));
  process.exit(code);
}
