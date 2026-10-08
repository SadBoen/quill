// 上游版本自检：把 UPSTREAM.md 里记录的版本和真实来源对一遍；加 --online 再比上游最新。
//
// 用法：node .upstream-check.mjs            （只查本地，不联网）
//      node .upstream-check.mjs --online   （顺带查上游最新，需要联网）
//
// 目的：UPSTREAM.md 是人写的，可能忘了更新；这个脚本是它的对照面。
//
// 退出码：
//   0  记录与本地一致；带 --online 时还要求上游也在记录之内
//   1  记录漂移、或上游已领先、或 --online 下查不到上游
//      ——「查不到」也算 1：查不出来源的失败不能被当成结论（同 toolchain 预检的纪律）。
//
// 只读：不 fetch、不 checkout，不写 vendor/ 与 .octop-ref/。

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const ONLINE = process.argv.includes('--online');
const root = new URL('.', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');

const GOOSE_REPO = 'https://api.github.com/repos/aaif-goose/goose';
const OCTOP_REPO = 'https://api.github.com/repos/TencentCloud/Octop';

function read(path) {
  return readFileSync(root + path, 'utf8');
}

function sh(cmd, args, cwd) {
  return execFileSync(cmd, args, { cwd: cwd ?? root, encoding: 'utf8' }).trim();
}

// ————————————————————————————————————————————————————————————————
// 以下是纯判定逻辑：不读文件、不联网，导出给自测直接喂合成数据。
// 拆出来是因为这个脚本出过一次「永远退出 0」的毛病，而那种毛病只有
// 用合成输入钉住判定才抓得住。
// ————————————————————————————————————————————————————————————————

// 版本号归一 + 比较。两边写法可能差一个 `v` 前缀，也可能一边少一段（v1.53 对 v1.53.0），
// 不归一就会误报「不一致」。
const stripV = (s) => (s ?? '').replace(/^v/, '').trim();
const vnum = (s) => stripV(s).split(/[.\-+]/).map((x) => parseInt(x, 10)).filter(Number.isFinite);

export function compareSemver(a, b) {
  const x = vnum(a);
  const y = vnum(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d > 0 ? 1 : -1;
  }
  return 0;
}

// 依赖清单 → 路径列表。清单是别的文档写的，格式不归我们管，
// 所以只认两种稳定信号：反引号里的代码片段、表格首列。取不到就报「没清单」，
// 不猜。
export function parseInventory(text) {
  const goose = [];
  const octop = [];
  let owner = 'goose';
  const isPath = (s) => /^[\w][\w./@-]*\/[\w./@-]*\.[A-Za-z0-9]+$/.test(s);
  const push = (p) => {
    const bag = owner === 'octop' ? octop : goose;
    if (isPath(p) && !bag.includes(p)) bag.push(p);
  };
  for (const raw of text.split(/\r?\n/)) {
    const heading = raw.match(/^#{1,4}\s+(.*)$/);
    if (heading) {
      owner = /octop/i.test(heading[1]) ? 'octop' : 'goose';
      continue;
    }
    for (const m of raw.matchAll(/`([^`\n]+)`/g)) push(m[1].trim());
    for (const m of raw.matchAll(/\]\(([^)\s]+)\)/g)) push(m[1].trim());
    const cell = raw.match(/^\|\s*([^|`]+?)\s*\|/);
    if (cell) push(cell[1].replace(/[`*]/g, '').trim());
  }
  return { goose, octop, source: 'UPSTREAM-USAGE.md' };
}

// 清单里的路径与上游改动文件名是否指同一个文件。两边写法前缀不同
// （一边可能带 `vendor/goose/`），所以按后缀与末两段比对。
export function samePath(path, file) {
  if (!path || !file) return false;
  if (path === file || file.endsWith('/' + path) || path.endsWith('/' + file)) return true;
  const tail = (s) => s.split('/').slice(-2).join('/');
  return tail(path) === tail(file);
}

const cap = (arr, n) => (arr.length > n ? arr.slice(0, n) : arr);

/**
 * 判定「我们是否落后上游」。输入是观测结果，不是 IO。
 *
 *   goose: { recorded, upstream: { tag, publishedAt, newer:[{tag,publishedAt}] }
 *                              | { error } }
 *          附加（尽力而为，拿不到不算失败）：diff: { files:[], truncated } | { error }
 *   octop: { pinned, upstream: { sha, date, aheadBy, files, truncated }
 *                              | { error } | { diverged: true } }
 *   inventory: { goose:[], octop:[], source:string|null }
 *
 * 返回 { lines, problems, behind, unknown }：
 *   behind/unknown 是给上层拼人话用的；problems 每一条都足以让门禁失败。
 */
export function decideUpstream({ goose, octop, inventory }) {
  const lines = [];
  const problems = [];
  const behind = { goose: false, octop: false };
  const unknown = { goose: false, octop: false };

  // —— goose ——
  if (!goose?.recorded) {
    problems.push('读不到记录里的 goose 版本，无从判断是否落后');
    unknown.goose = true;
  } else if (goose.upstream?.error) {
    problems.push(`查不到 goose 上游（${goose.upstream.error}）。这是「查不到」，不是「已是最新」`);
    unknown.goose = true;
  } else if (compareSemver(goose.upstream.tag, goose.recorded) <= 0) {
    lines.push(`goose   上游最新=${goose.upstream.tag}（${goose.upstream.publishedAt}）→ 与记录一致`);
  } else {
    behind.goose = true;
    const newer = goose.upstream.newer ?? [];
    const count = newer.length ? newer.length : null;
    problems.push(
      `goose 落后上游${count === null ? '（至少 1 个 release，拿不到发布列表）' : ` ${count} 个 release`}：` +
        `记录 ${goose.recorded}，上游最新 ${goose.upstream.tag}`,
    );
    lines.push(`goose   落后上游：记录 ${goose.recorded} → 上游 ${goose.upstream.tag}（${goose.upstream.publishedAt}）`);
    if (newer.length) lines.push(`        新出的 release：${cap(newer.map((r) => r.tag), 10).join('、')}`);
    lines.push('        读变化：https://github.com/aaif-goose/goose/releases');
    lines.push(...inventoryNotes(inventory, 'goose', { files: goose.diff?.files, truncated: goose.diff?.truncated, error: goose.diff?.error }, '        '));
  }

  // —— octop ——
  if (!octop?.pinned) {
    problems.push('读不到记录里的 octop commit，无从判断是否落后');
    unknown.octop = true;
  } else if (octop.upstream?.error) {
    problems.push(`查不到 octop 上游（${octop.upstream.error}）。这是「查不到」，不是「就是最新」`);
    unknown.octop = true;
  } else if (octop.upstream.diverged) {
    problems.push(
      `octop 的 main 与记录的 commit 不在同一条线上（compare 状态 diverged）：记录 ${octop.pinned}，` +
        `上游 ${octop.upstream.sha}。这不是「落后」，得人工看`,
    );
    unknown.octop = true;
  } else if (octop.upstream.sha === octop.pinned) {
    lines.push(`octop   上游 main=${octop.upstream.sha.slice(0, 12)}（${octop.upstream.date}）→ 与记录一致`);
  } else if (!Number.isInteger(octop.upstream.aheadBy)) {
    problems.push(
      `octop 上游 main 已不是记录的 commit（记录 ${octop.pinned.slice(0, 12)}，上游 ` +
        `${octop.upstream.sha.slice(0, 12)}），但算不出落后几个 commit` +
        `${octop.upstream.compareError ? `：${octop.upstream.compareError}` : '：compare 接口没给出'}`,
    );
    behind.octop = true;
  } else if (octop.upstream.aheadBy === 0) {
    lines.push(`octop   上游 main=${octop.upstream.sha.slice(0, 12)}（${octop.upstream.date}）→ 与记录一致`);
  } else {
    behind.octop = true;
    problems.push(
      `octop 落后上游 ${octop.upstream.aheadBy} 个 commit：记录 ${octop.pinned.slice(0, 12)}，` +
        `上游 main=${octop.upstream.sha.slice(0, 12)}（${octop.upstream.date}）`,
    );
    lines.push(`octop   落后上游：记录 ${octop.pinned.slice(0, 12)} → 上游 main=${octop.upstream.sha.slice(0, 12)}（${octop.upstream.date}）`);
    lines.push(`        落后 ${octop.upstream.aheadBy} 个 commit`);
    lines.push(`        读变化：https://github.com/TencentCloud/Octop/compare/${octop.pinned}...main`);
    lines.push(...inventoryNotes(inventory, 'octop', { files: octop.upstream.files, truncated: octop.upstream.truncated }, '        '));
  }

  return { lines, problems, behind, unknown };
}

// 落后时把「哪些改动真的和我们有关」摆出来：清单里的文件 ∩ 上游改动过的文件。
// 没有交集也是结论 —— 说明这批改动碰不到我们依赖的东西。
function inventoryNotes(inventory, key, diff, pad) {
  const list = inventory?.[key] ?? [];
  const src = inventory?.source ? `依赖清单：${inventory.source}` : '依赖清单见 UPSTREAM-USAGE.md（若已落地）';
  const shown = cap(list, 12);
  const head = shown.length
    ? `${pad}${src} —— 我们依赖的 ${list.length} 个文件：\n${shown.map((p) => `${pad}  ${p}`).join('\n')}`
    : `${pad}${src}（本机读不到，上游改了哪些我们依赖的文件查不出来）`;
  if (diff?.error) return [head, `${pad}上游改动文件清单拿不到：${diff.error}`];
  if (!Array.isArray(diff?.files)) return [head];
  const hit = diff.files.filter((f) => list.some((p) => samePath(p, f.filename ?? f)));
  if (!hit.length) return [head, `${pad}上游这批改动没有碰到上面任何依赖文件`];
  return [
    head,
    `${pad}其中被上游这批改动碰到的（要看的是这些）：\n${cap(hit, 20).map((f) => `${pad}  ${f.filename ?? f}`).join('\n')}`,
    diff.truncated ? `${pad}（GitHub 最多返回 300 个改动文件，更早的可能没列出来）` : '',
  ].filter(Boolean);
}

// ————————————————————————————————————————————————————————————————
// 联网观测：只有 GET，没有一个写操作。
// ————————————————————————————————————————————————————————————————
async function fetchJson(url) {
  if (!globalThis.fetch) throw new Error('这个 Node 没有全局 fetch（需要 Node 18+）');
  const res = await fetch(url, {
    headers: { 'user-agent': 'quill-upstream-check', accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(20_000),
  });
  if (res.status === 403 || res.status === 429) {
    const left = res.headers.get('x-ratelimit-remaining');
    throw new Error(`GitHub 限流或拒绝访问（HTTP ${res.status}${left !== null ? `，剩余额度 ${left}` : ''}）`);
  }
  if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
  return res.json();
}

async function observeGoose(recorded, pinCommit) {
  let latest;
  try {
    const r = await fetchJson(`${GOOSE_REPO}/releases/latest`);
    if (!r?.tag_name) throw new Error('releases/latest 没有 tag_name');
    latest = { tag: stripV(r.tag_name), publishedAt: (r.published_at ?? '').slice(0, 10) };
  } catch (e) {
    return { upstream: { error: e.message } };
  }
  // 落后几个 release 要靠列表数；列表拿不到就只说「至少 1 个」，不猜数字。
  let newer = null;
  try {
    const all = await fetchJson(`${GOOSE_REPO}/releases?per_page=100`);
    newer = (Array.isArray(all) ? all : [])
      .filter((r) => !r.draft && !r.prerelease)
      .map((r) => ({ tag: stripV(r.tag_name), publishedAt: (r.published_at ?? '').slice(0, 10) }))
      .filter((r) => compareSemver(r.tag, recorded) > 0);
  } catch {
    /* 列表是锦上添花 */
  }
  // 依赖文件是否被改过：pin 的 commit 与 main 比。纯 API，不碰本地仓库。
  // goose 本地是文件拷贝，这里是唯一能拿到改动文件列表的途径。
  let diff = null;
  if (pinCommit) {
    try {
      const c = await fetchJson(`${GOOSE_REPO}/compare/${pinCommit}...main`);
      diff = { files: (c.files ?? []).map((f) => ({ filename: f.filename })), truncated: !!c.files?.length && c.files.length >= 300 };
    } catch (e) {
      diff = { error: e.message };
    }
  }
  return { upstream: { ...latest, newer }, diff };
}

async function observeOctop(pinned) {
  let head;
  try {
    const c = await fetchJson(`${OCTOP_REPO}/commits/main`);
    if (!c?.sha) throw new Error('commits/main 没有 sha');
    head = { sha: c.sha, date: (c?.commit?.committer?.date ?? '').slice(0, 10) };
  } catch (e) {
    return { upstream: { error: e.message } };
  }
  if (head.sha === pinned) return { upstream: head };
  try {
    const c = await fetchJson(`${OCTOP_REPO}/compare/${pinned}...main`);
    if (c?.status === 'diverged') return { upstream: { ...head, diverged: true } };
    const files = Array.isArray(c?.files) ? c.files : [];
    return {
      upstream: {
        ...head,
        aheadBy: Number.isInteger(c?.ahead_by) ? c.ahead_by : undefined,
        files: files.map((f) => ({ filename: f.filename })),
        truncated: files.length >= 300,
      },
    };
  } catch (e) {
    return { upstream: { ...head, compareError: e.message } };
  }
}

// 清单是另一个 agent 在写的，读不到不算失败。
function readInventory() {
  if (!existsSync(root + 'UPSTREAM-USAGE.md')) return { goose: [], octop: [], source: null };
  return parseInventory(read('UPSTREAM-USAGE.md'));
}

/**
 * 从 UPSTREAM.md 里读出「我们跟的版本 / commit」两行值。
 *
 * **为什么单独抽出来并导出**（queue Q076）：这两行原来是两段内联正则，而 octop
 * 那行的值在表格里**没有加粗**（写作 `| **我们跟的 commit** | \`sha\` |`），原正则
 * 却要求值两边都有 `**`，于是它**永远读不到** —— 脚本照样打印 `记录=?` 与
 * `OK: UPSTREAM.md 的记录和实际一致`。这正是本仓库最忌讳的那类检查：
 * **读不到输入却报通过**（「多一份记录、多一处漂移的可能」，2026-10-08 的原话）。
 * 抽出来是为了让自测能拿真文档与坏文档各喂一次（见 upstream-check-selftest 场景 9）。
 *
 * 值的写法允许带或不带 `**`：Markdown 里加粗与否不是契约，读到值才是。
 */
export function parseUpstreamDoc(doc) {
  const gooseRaw = doc.match(/\*\*我们跟的版本\*\*\s*\|\s*\*{0,2}\s*([^*|\n]+?)\s*\*{0,2}\s*\|/)?.[1];
  const octopCommit = doc.match(/\*\*我们跟的 commit\*\*\s*\|\s*\*{0,2}\s*`?([0-9a-f]{40})`?/)?.[1];
  // 读不到就是 `undefined`（**不是空串**）：调用方靠 `!value` 判「没核过」，
  // 而空串会让日志里出现 `记录=` 这种看不出是缺值的写法。
  return {
    gooseVersion: gooseRaw === undefined ? undefined : stripV(gooseRaw),
    octopCommit,
  };
}

async function main() {
  const doc = read('UPSTREAM.md');
  const problems = [];
  const lines = [];

  // ---------- 记录里的值 ----------
  const { gooseVersion: gooseDocVersion, octopCommit: octopDocCommit } = parseUpstreamDoc(doc);
  // **读不到就等于没核**（2026-10-08 的教训：这个脚本曾经读不到 octop 那行、
  // 却照样打印一致）。所以缺任一值都算问题，不当成「跳过这一项」。
  if (!gooseDocVersion) {
    problems.push(
      'UPSTREAM.md 读不到 goose 的「我们跟的版本」—— 表格那行格式变了？读不到就没核过，不算通过',
    );
  }
  if (!octopDocCommit) {
    problems.push(
      'UPSTREAM.md 读不到 octop 的「我们跟的 commit」（40 位 hex）—— 读不到就没核过，不算通过',
    );
  }

  // ---------- goose：真实来源是 Cargo.toml ----------
  const gooseToml = read('vendor/goose/Cargo.toml');
  const gooseRealVersion = gooseToml.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!gooseRealVersion) problems.push('读不到 vendor/goose/Cargo.toml 的 version');
  lines.push(`goose   记录=${gooseDocVersion ?? '?'}  实际=${gooseRealVersion ?? '?'}`);
  if (gooseDocVersion && gooseRealVersion && gooseDocVersion !== gooseRealVersion) {
    problems.push(`goose 版本对不上：UPSTREAM.md 写 ${gooseDocVersion}，Cargo.toml 是 ${gooseRealVersion}`);
  }

  // ---------- goose：`vendor/goose` 是纯拷贝还是 git 检出，文档不许猜 ----------
  //
  // 这两条原本是反着的（2026-10-08 在 CI 上第一次真跑就撞上了）：
  //   有 .git  → 报错，要文档删掉「纯文件拷贝，没有 .git」
  //   没 .git  → 报错，要文档写上「纯文件拷贝，没有 .git」
  // 同一份静态文档不可能两头都对。而实际上这两种状态**都不是**文档该描述的东西：
  // `vendor/` 是 gitignore 的，fresh clone 里这个目录压根不存在；
  // 跑过 fetch-vendor.sh 它又变成一个正经 git 检出。
  // 「本机当时有没有跑过取码脚本」不是一条该写进文档的事实。
  //
  // 所以现在只查两件与本机状态无关的事：那两句已经过期的话不许再出现，
  // 以及文档得告诉一个刚 clone 的人怎么把 goose 取回来。
  if (doc.includes('**纯文件拷贝，没有 `.git`**') || doc.includes('做不到：对本地快照做文件级 diff')) {
    problems.push('UPSTREAM.md 还在说 goose 是纯拷贝、不能 diff —— 取回来之后它就是 git 检出，能 diff。那段要改');
  }
  if (!doc.includes('fetch-vendor.sh')) {
    problems.push('UPSTREAM.md 没写怎么取回 vendor/goose —— fresh clone 里那个目录不存在');
  }

  // ---------- goose：.upstream-pin 是机器可读的记录，必须与前两处一致 ----------
  //
  // 为什么加这一段：`.upstream-pin` 一直躺在仓库里没人读，于是它记的版本
  // 可以和 UPSTREAM.md、Cargo.toml 各说各话而不被发现。**多一份记录、多一处
  // 漂移的可能**，所以要么它被门禁读，要么它不该存在 —— 现在选前者。
  //
  // 注意它的 hash **本地无法验证**：vendor/goose 没有 .git，没法确认这份拷贝
  // 真的就是那个 commit。它记的是「我们打算用哪一版」，不是「这份文件就是
  // 那一版」的证明 —— UPSTREAM.md 里那句限制仍然成立，别因为这里能读出 hash
  // 就以为能做文件级 diff。
  const pinRaw = read('.upstream-pin').trim();
  const pinVersion = stripV(pinRaw.split(/\s+/)[0]);
  const pinCommit = pinRaw.match(/\b([0-9a-f]{40})\b/)?.[1];
  lines.push(`goose   pin=${pinVersion ?? '?'}${pinCommit ? ` @${pinCommit.slice(0, 12)}` : ''}`);
  if (!pinCommit) {
    problems.push('.upstream-pin 没有 40 位 commit hash，格式应为「vX.Y.Z <sha1>」');
  }
  if (pinVersion && gooseRealVersion && pinVersion !== gooseRealVersion) {
    problems.push(`goose 版本三方对不上：Cargo.toml=${gooseRealVersion}，UPSTREAM.md=${gooseDocVersion ?? '?'}，.upstream-pin=${pinVersion}`);
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

  // ---------- 联网：比上游最新。落后即失败，查不到也算失败 ----------
  // **为什么落后要判失败**：原版打印完「上游最新 = vX」就退出 0，
  // 上游走了我们没跟，这道门禁照样绿 —— 那是「一个不可能失败的检查」。
  if (ONLINE) {
    const inventory = readInventory();
    const [g, o] = await Promise.all([
      observeGoose(pinVersion, pinCommit),
      observeOctop(octopRealCommit ?? octopDocCommit),
    ]);
    const verdict = decideUpstream({
      goose: { recorded: pinVersion, ...g },
      // 记录与本地不一致时以本地实际值为准：否则一次 UPSTREAM.md 打错字，
      // 会同时让「记录漂移」和「落后 N 个 commit」两条一起报，读的人得自己分辨。
      octop: { pinned: octopRealCommit ?? octopDocCommit, ...o },
      inventory,
    });
    lines.push(...verdict.lines);
    problems.push(...verdict.problems);
  }

  console.log('=== 上游版本对照 ===');
  for (const line of lines) console.log(line);
  console.log('');
  if (problems.length) {
    console.log(problems.some((p) => /落后|查不到|diverged|无从判断/.test(p)) ? '有问题（其中含上游差异）：' : '有问题：');
    for (const p of problems) console.log('  - ' + p);
    process.exitCode = 1;
  } else {
    console.log('OK: UPSTREAM.md 的记录和实际一致。');
    if (!ONLINE) console.log('（加 --online 可顺带比对上游最新版本）');
    else console.log('（已联网核对：上游也在记录之内。）');
  }
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) await main();