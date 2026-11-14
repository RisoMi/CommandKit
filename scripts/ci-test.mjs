#!/usr/bin/env node
/**
 * CI 里的 cargo test 包装器。
 *
 *   node scripts/ci-test.mjs             全量跑（macOS 用）
 *   node scripts/ci-test.mjs --per-case  逐个用例跑（Windows 用：进程被异常中断时能定位到具体用例）
 *
 * 失败时把关键行打成 ::error:: 工作流命令 → 生成 check-run annotation，
 * 公开仓库不登录也能在 commit / PR 页面看到原因。
 */
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const MP = 'src-tauri/Cargo.toml';
const PER_CASE = process.argv.includes('--per-case');

function cargo(args) {
  const res = spawnSync('cargo', args, {
    encoding: 'utf8',
    shell: true,
    maxBuffer: 128 * 1024 * 1024,
    env: process.env,
  });
  return { out: `${res.stdout ?? ''}${res.stderr ?? ''}`, code: res.status ?? 1 };
}

const hex = (code) => (code < 0 ? `0x${(code >>> 0).toString(16).toUpperCase()}` : String(code));

function annotate(text, code, scope) {
  const key = text
    .split(/\r?\n/)
    .filter((l) =>
      /^error(\[|:)|panicked at|^test .* FAILED|^---- .* stdout ----|assertion .* failed|^failures:$/.test(l)
    );
  for (const line of key.slice(0, 20)) {
    console.log(`::error::[${scope}] ${line.replace(/[\r\n]+/g, ' ').slice(0, 360)}`);
  }
  if (!key.length) {
    console.log(`::error::[${scope}] 退出码 ${hex(code)}，未匹配到错误行（进程可能被外部中断）`);
  }
  console.log(`--- ${scope} 日志尾部 ---`);
  console.log(text.split(/\r?\n/).slice(-60).join('\n'));
}

if (!PER_CASE) {
  const { out, code } = cargo(['test', '--manifest-path', MP, '--tests', '--', '--test-threads=1']);
  writeFileSync('cargo-test.log', out);
  console.log(
    out
      .split(/\r?\n/)
      .filter((l) => l.startsWith('test result') || l.startsWith('running '))
      .join('\n') || out.slice(-2000)
  );
  if (code !== 0) {
    annotate(out, code, 'cargo test');
    process.exit(code);
  }
  process.exit(0);
}

// ------------------------------------------------------------------ 逐用例

const targets = ['kernel', 'runtime'];
let failed = 0;

for (const target of targets) {
  const listing = cargo(['test', '--manifest-path', MP, '--test', target, '--', '--list']);
  if (listing.code !== 0) {
    // 二进制起不来（例如 Windows 上 STATUS_ENTRYPOINT_NOT_FOUND）时 --list 会失败，必须显式报错
    failed += 1;
    console.log(`FAIL ${target}::--list (${hex(listing.code)})`);
    annotate(listing.out, listing.code, `${target}::--list`);
    continue;
  }
  const names = listing.out
    .split(/\r?\n/)
    .filter((l) => l.trim().endsWith(': test'))
    .map((l) => l.replace(/:\s*test$/, '').trim());

  if (!names.length) {
    console.log(`[${target}] 没有可运行用例（可能全部被 cfg 排除）`);
    continue;
  }
  console.log(`[${target}] ${names.length} 个用例：${names.join(', ')}`);

  for (const name of names) {
    const { out, code } = cargo([
      'test',
      '--manifest-path',
      MP,
      '--test',
      target,
      name,
      '--',
      '--exact',
      '--test-threads=1',
    ]);
    const tag = `${target}::${name}`;
    if (code === 0) {
      console.log(`PASS ${tag}`);
    } else {
      failed += 1;
      console.log(`FAIL ${tag} (${hex(code)})`);
      annotate(out, code, tag);
    }
  }
}

process.exit(failed ? 1 : 0);
