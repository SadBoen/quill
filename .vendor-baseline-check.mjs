#!/usr/bin/env node
/**
 * `ui/web/src/index.css` 必须与移植基准 `vendor/openoctopus-frontend/src/index.css`
 * **逐字节一致**，所以本地改动一律走 `overrides.css`。这条约束以前只写在注释里，
 * 真正的检查却只躺在几个一次性脚本（`.wsl-css-check.sh` 等）里 ——
 * `gates.sh` 没跑、CI 没跑。也就是说「有 sha256 门禁」这句话本身是假的。
 *
 * **为什么这里钉哈希、而不是直接比对两个文件**：后者要 `vendor/` 在手边，
 * 而 `vendor/` 是 gitignore 的，fresh clone 和 CI 里根本没有。
 * 钉住移植那一刻的哈希，才能在任何地方跑 —— 包括没有任何 vendor 检出的时候。
 *
 * 为什么值得单独一道：`index.css` 是整套样式的地基，它一旦被本地改动悄悄改掉，
 * 界面就会跟设计基准漂移，而这种漂移肉眼看不出来（只是「有点不一样」）。
 *
 * 真要改 `index.css`，改完把 EXPECTED 换成新值，并在提交信息里说清为什么。
 * 正常情况下该改的是 `overrides.css`。
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const HERE = fileURLToPath(new URL('.', import.meta.url));
const TARGET = join(HERE, 'ui', 'web', 'src', 'index.css');

// 移植那一刻 `index.css` 的 sha256。这个值同时记在
// `vendor/openoctopus-frontend/src/index.css` 的同内容副本上，
// 以及 `overrides.css` 文件头的说明里 —— 三处必须一致。
const EXPECTED = 'FF5A2B1832ABEEB183C28197080C8184919E36EC2C5043248BD4A24515D8718D';

export function check(actualHex) {
  return {
    ok: actualHex === EXPECTED,
    expected: EXPECTED,
    actual: actualHex,
  };
}

/**
 * 自测：证明这道检查**会红**。
 *
 * 为什么非有不可：这个仓库已经有过三次「检查从来没真正生效过」
 * （一道门禁把 failed 恒算成 0，一道门禁压根没接线，还有一道门禁的检查只躺在
 * 一次性脚本里、gates.sh 与 CI 都没跑）。本检查的价值全在「基准被动就会红」，
 * 所以判据是：**能不能指出一个改动？**
 */
function selfTest() {
  const cases = [
    ['与记录一致 → 通过', EXPECTED, true],
    ['改一个字节 → 必须不通过', EXPECTED.slice(0, -1) + (EXPECTED.endsWith('D') ? 'C' : 'D'), false],
    ['大小写不同 → 必须不通过（真的会被绕过）', EXPECTED.toLowerCase(), false],
    ['空值 → 必须不通过（不能当成「没差异」）', '', false],
    ['恒真实现（永远返回 ok）会被这四条抓住', EXPECTED, true],
  ];
  let bad = 0;
  for (const [label, input, want] of cases) {
    const got = check(input).ok;
    const ok = got === want;
    if (!ok) bad++;
    console.log(`  ${ok ? '✓' : '✗'} ${label}`);
  }
  // 单独钉一条：恒真的实现必须被抓住，否则前四条里凡期望 true 的都会蒙混过关。
  const alwaysOk = () => check(EXPECTED.slice(0, -1) + '0').ok;
  const caught = !alwaysOk();
  if (!caught) bad++;
  console.log(`  ${caught ? '✓' : '✗'} 「永远通过」的实现会被本自测抓住`);
  console.log(bad === 0 ? '\n移植基准自测：判定会红，可信' : `\n移植基准自测：${bad} 条不符`);
  process.exit(bad === 0 ? 0 : 1);
}

function main() {
  console.log('=== 移植基准 · index.css ===');
  let bytes;
  try {
    bytes = readFileSync(TARGET);
  } catch (e) {
    console.error(`  ✗ 读不到 ${TARGET}`);
    console.error('    没有这个文件就没法核对 —— 报告成「读不到」，不假装通过。');
    process.exit(1);
  }
  const actual = createHash('sha256').update(bytes).digest('hex').toUpperCase();
  const r = check(actual);
  if (!r.ok) {
    console.error(`  ✗ index.css 与移植基准不一致`);
    console.error(`      期望 ${r.expected}`);
    console.error(`      实际 ${r.actual}`);
    console.error('');
    console.error('    两件事只有一件是对的：');
    console.error('      · 你确实想改基准 → 改完把本文件里的 EXPECTED 换成新值，并在提交信息里说清为什么；');
    console.error('      · 你不想改基准  → 把 index.css 还原，本地样式改动应该全部落在 overrides.css。');
    process.exit(1);
  }
  console.log(`  ✓ index.css 与移植基准逐字节一致（${r.actual.slice(0, 16)}…）`);
  console.log('    本地样式改动请落在 ui/web/src/overrides.css，不要动这个文件。');
  process.exit(0);
}

const isMain = process.argv[1] && fileURLToPath(import.meta.url) === join(process.argv[1]);
if (isMain) {
  if (process.argv.includes('--self-test')) selfTest();
  else main();
}