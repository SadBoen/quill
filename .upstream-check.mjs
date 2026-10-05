// 上游版本自检：把 UPSTREAM.md 里记录���版本和真实来源对一遍。
//
// 用法：node .upstream-check.mjs            （只查本地，不联网）
//      node .upstream-check.mjs --online   （顺带查上游最新，需要联网）
//
// 目的：UPSTREAM.md 是人写的，可能忘了更新；这个脚本是它的对照面。

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';

const ONLINE = process.argv.includes('--online');
const root = new URL('.', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');

function read(path) {
  return readFileSync(root + path, 'utf8');
}

function sh(cmd, args, cwd) {
  return execFileSync(cmd, args, { cwd: cwd ?? root, encoding: 'utf8' }).trim();
}

const doc = read('UPSTREAM.md');
const problems = [];
const lines = [];

// ---------- 记录里的值 ----------
// 版本号两边写法可能差一个 `v` 前缀，比对前先归一，否则会误报。
const stripV = (s) => (s ?? '').replace(/^v/, '').trim();
const gooseDocVersion = stripV(doc.match(/\*\*我们跟的版本\*\* \| \*\*([^*|]+)\*\*/)?.[1]);
const octopDocCommit = doc.match(/\*\*我们跟的 commit\*\* \| \*\*`([0-9a-f]{40})`\*\*/)?.[1];

// ---------- goose：真实来源是 Cargo.toml ----------
const gooseToml = read('vendor/goose/Cargo.toml');
const gooseRealVersion = gooseToml.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
if (!gooseRealVersion) problems.push('读不到 vendor/goose/Cargo.toml 的 version');
lines.push(`goose   记录=${gooseDocVersion ?? '?'}  实际=${gooseRealVersion ?? '?'}`);
if (gooseDocVersion && gooseRealVersion && gooseDocVersion !== gooseRealVersion) {
  problems.push(`goose 版本对不上：UPSTREAM.md 写 ${gooseDocVersion}，Cargo.toml 是 ${gooseRealVersion}`);
}

// ---------- goose：确认它确实不是 git 检出（限制条件要在文件里写对） ----------
// 注意：不能用 `git -C vendor/goose rev-parse`，git 会一路向上找到 quill 仓库根，
// 那样永远「成功」，判断就废了。只能看目录里有没有 .git。
const gooseHasGit = existsSync(root + 'vendor/goose/.git');
if (gooseHasGit && doc.includes('**纯文件拷贝，没有 `.git`**')) {
  problems.push('vendor/goose 现在是 git 检出了，UPSTREAM.md 里「做不到 diff」那段要改');
}
if (!gooseHasGit && !doc.includes('**纯文件拷贝，没有 `.git`**')) {
  problems.push('vendor/goose 确实不是 git 检出，但 UPSTREAM.md 没写这条限制');
}

// ---------- Octop：真实来源是 sparse checkout ----------
let octopRealCommit = null;
try {
  octopRealCommit = sh('git', ['-C', '.octop-ref/octop', 'rev-parse', 'HEAD']);
} catch {
  problems.push('.octop-ref/octop 不存在或不是 git 检出，跑一次 sparse checkout 初始化');
}
lines.push(`octop   记录=${octopDocCommit ?? '?'}  实际=${octopRealCommit ?? '?'}`);
if (octopDocCommit && octopRealCommit && octopDocCommit !== octopRealCommit) {
  problems.push(`octop commit 对不上：UPSTREAM.md 写 ${octopDocCommit}，实际 ${octopRealCommit}`);
}

// ---------- 联网：比上游最新 ----------
if (ONLINE) {
  const gooseLatest = fetch('https://api.github.com/repos/aaif-goose/goose/releases/latest')
    .then((r) => r.json())
    .then((r) => `v${r.tag_name.replace(/^v/, '')} (${r.published_at.slice(0, 10)})`)
    .catch((e) => `查询失败: ${e.message}`);
  const octopLatest = fetch('https://api.github.com/repos/TencentCloud/Octop/commits/main')
    .then((r) => r.json())
    .then((r) => `${r.sha.slice(0, 12)} (${r.commit.committer.date.slice(0, 10)})`)
    .catch((e) => `查询失败: ${e.message}`);
  lines.push(`goose   上游最新=${await gooseLatest}`);
  lines.push(`octop   上游 main=${await octopLatest}`);
}

function fetch(url) {
  // Node 18+ 有全局 fetch；老环境退回 https 模块。
  if (globalThis.fetch) return globalThis.fetch(url, { headers: { 'user-agent': 'quill-upstream-check' } });
  return import('node:https').then(
    (https) =>
      new Promise((resolve, reject) => {
        https
          .get(url, { headers: { 'user-agent': 'quill-upstream-check' } }, (res) => {
            let body = '';
            res.on('data', (c) => (body += c));
            res.on('end', () => resolve({ json: async () => JSON.parse(body) }));
          })
          .on('error', reject);
      }),
  );
}

console.log('=== 上游版本对照 ===');
for (const line of lines) console.log(line);
console.log('');
if (problems.length) {
  console.log('有问题：');
  for (const p of problems) console.log('  - ' + p);
  process.exitCode = 1;
} else {
  console.log('OK: UPSTREAM.md 的记录和实际一致。');
  if (!ONLINE) console.log('（加 --online 可顺带比对上游最新版本）');
}
