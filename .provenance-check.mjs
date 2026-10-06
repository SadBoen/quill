#!/usr/bin/env node
// 出处自检：把 UPSTREAM-USAGE.md 里每一个上游引用都真的打开核一遍。
//
// 用途：索引的价值全在「它不会悄悄烂掉」。这个脚本就是那份价值本身。
//
// 用法：node .provenance-check.mjs
//      node .provenance-check.mjs --index <别的索引文件>
//      node .provenance-check.mjs --root <另一个根> --index <索引文件>
//
// --index 只给自测用：判定逻辑要能对着一份**合成索引**跑，否则自测只能测纯函数，
// 测不到「解析 markdown → 逐条判定 → 定退出码」这条真实路径。
// --root 同理：让自测能在临时目录里造出被引用的上游文件，而不是指望本机
// 恰好跑过 fetch-vendor.sh。
//
// 判定分四类，绝不混为一谈：
//   ok                     文件在、行号在范围内
//   offline-unverifiable   引用落在 octop 的 sparse 集合之外 —— 可见，**不**判失败
//   broken                 文件在集合内却读不到、或行号越界、或引用格式不对 —— 判失败
//   ours-missing           指向本仓库自己的文件却找不到 —— 判失败
//
// 退出码：0 没有 broken；1 有 broken；2 索引本身读不到 / 结构不对。
//
// **为什么「格式不对」也算 broken**：octop 有两个同名 skillhub_market.py，
// 裸写文件名时读者会落到另一份上，从而把「上游缺陷」误判成假的。
// 这已经真实发生过一次（BACKLOG.md 的 B6-3），所以格式错必须红。
//
// 只读：不写文件、不 fetch、不联网。sparse 集合只读 `git sparse-checkout list`（本地）。

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// ————————————————————————————————————————————————————————————————
// 纯判定逻辑：不读文件、不碰 git，导出给 .scripts/provenance-selftest.mjs 喂合成数据。
// 拆出来的理由和 .upstream-check.mjs 一样：判定逻辑写错了不会自己暴露。
// ————————————————————————————————————————————————————————————————

// 只认这些扩展名。放宽到「任何含点的 token」会把 `m..s`、`.md` 这类散文碎片
// 当成引用，误报一串没有意义的失败。
const REF_EXT = /\.(md|mjs|cjs|js|jsx|ts|tsx|rs|py|css|less|json|sql|sh|txt|toml|ya?ml|html)$/;

/** 判定「这段反引号内容是不是一条引用」。宁可漏判也不误判。 */
export function looksLikeRef(s) {
  const t = s.trim();
  // 文件名部分必须有一个点、扩展名在白名单里，且点前面至少有一个词字符
  // —— `.md`（没有词字符）不算，`index.module.less` 算。
  const file = t.replace(/:\d+(-\d+)?$/, '');
  if (!file || !REF_EXT.test(file)) return false;
  if (!/[\w-]\.[A-Za-z0-9]+$/.test(file)) return false;
  // 反引号里带空格或花括号的是散文片段（`node .provenance-check.mjs`、
  // `skills/{slug}/SKILL.md`），不是路径。反引号在这份文件里等于「这是一条引用」，
  // 所以散文的正确写法是别加反引号。
  return /^[\w.@/+-]+$/.test(file);
}

/**
 * 把一条引用拆成 { path, from, to, malformed, why }。
 * 不查文件 —— 「行号在不在范围内」要等调用方给出行数才知道。
 *
 * 两种合法形态：
 *   path:line       有行号 —— 要核行号在不在范围内
 *   path            没行号 —— 只核文件在不在（「我们的落点」允许这么写）
 */
export function parseRef(raw) {
  const t = String(raw).trim().replace(/^`|`$/g, '');
  const m = /^(.*?):(\d+)(?:-(\d+))?$/.exec(t);
  if (!m) return { path: t, from: null, to: null, malformed: false, why: '' };
  const [, path, a, b] = m;
  if (!path) return { path: t, from: null, to: null, malformed: true, why: '行号前面没有路径' };
  const from = +a;
  const to = b ? +b : from;
  if (to < from) return { path, from: to, to: from, malformed: true, why: `行号区间反了（${from}-${to}）` };
  return { path, from, to, malformed: false, why: '' };
}

/**
 * 这条引用指的是不是「上游」。按路径形状判，不靠它出现在哪一列 ——
 * 靠列判的话，把上游路径错写进「我们的落点」那一列就查不出来了。
 */
export function isUpstreamPath(p) {
  return (
    p.startsWith('vendor/') ||
    p.startsWith('.octop-ref/') ||
    p.startsWith('dashboard/') ||
    p.startsWith('src/octop/') ||
    p.startsWith('documentation/') ||
    p.startsWith('crates/goose/')
  );
}

/** octop 引用：可能落在 sparse 集合之外，判失败前要多问一句。 */
export function isOctopPath(p) {
  return p.startsWith('.octop-ref/octop/') || p.startsWith('dashboard/') || p.startsWith('src/octop/');
}

/** octop 相对路径 → 该路径在不在 sparse 集合内。 */
export function inSparseSet(p, sparseDirs) {
  const rel = p.replace(/^\.octop-ref\/octop\//, '');
  const dirOf = rel.split('/').slice(0, -1).join('/'); // 该文件所在的目录
  for (const dir of sparseDirs) {
    if (rel === dir || rel.startsWith(dir + '/')) return true;
    // cone 模式会把每个 sparse 目录的**各级祖先目录里的紧邻文件**也带出来
    // （`dashboard/package.json` 就是这样才在手边，尽管 `dashboard` 本身不在集合里）。
    const parts = dir.split('/');
    for (let i = 1; i <= parts.length; i += 1) {
      if (parts.slice(0, i).join('/') === dirOf) return true;
    }
  }
  return false;
}

/**
 * 一条引用的最终判定。
 *
 * ctx = { lineCount(path) -> number|null, inSparse(path) -> bool, rootHit(path) -> bool }
 *   lineCount 返回 null = 文件读不到（存在性由 rootHit 与 sparse 判定共同区分原因）。
 *   rootHit = 这条路径在**仓库根**下就有一个同名文件（bare name 能不能被钉死的判据）。
 */
export function classifyRef(raw, ctx) {
  const r = parseRef(raw);
  const base = { raw: String(raw).trim(), ...r };
  if (r.malformed) return { ...base, status: 'broken', reason: `引用格式不对：${r.why}` };

  // 带行号却没有目录：只有当它在仓库根上就有一个同名文件时才能定位
  // （`WORKING.md`、`.library-check.mjs` —— 没有上游与它们同名）。
  // 定位不到就是 broken：octop 有两个同名 skillhub_market.py，裸写文件名会让读者
  // 落到另一份上，把「上游缺陷」误判成假的 —— 这已经真实发生过一次。
  if (!r.path.includes('/') && !ctx.rootHit(r.path)) {
    return { ...base, status: 'broken', reason: '只有文件名没有目录，定位不到是哪一份' };
  }

  const octop = isOctopPath(r.path);
  const upstream = isUpstreamPath(r.path);
  const sparse = octop ? ctx.inSparse(r.path) : true;

  const n = ctx.lineCount(r.path);
  if (n === null) {
    if (octop && !sparse) {
      return {
        ...base,
        status: 'offline-unverifiable',
        reason: '落在 octop 的 sparse 集合之外，离线核不到（不是引用腐烂）',
      };
    }
    // vendor 整棵树都没取回来 → 说不上引用烂不烂，如实报「核不到」。
    // 这不是放宽标准：取回来了还找不到仍然是 broken（下面那行）。
    if (upstream && typeof ctx.treePresent === 'function' && !ctx.treePresent(r.path)) {
      return {
        ...base,
        status: 'offline-unverifiable',
        reason: `${treeRootOf(r.path)} 还没取回来，离线核不到（不是引用腐烂）。取回：bash .scripts/fetch-vendor.sh`,
      };
    }
    return { ...base, status: upstream ? 'broken' : 'ours-missing', reason: '文件读不到' };
  }
  if (r.from === null) {
    return { ...base, status: 'ok', reason: '（只核了文件存在，这条引用没写行号）' };
  }
  if (r.to > n) {
    return {
      ...base,
      status: 'broken',
      reason: `行号越界：文件只有 ${n} 行，引到了 ${r.from}${r.to === r.from ? '' : '-' + r.to}`,
    };
  }
  return { ...base, status: 'ok', reason: '' };
}

/**
 * 汇总一批判定。返回 { counts, broken, unverifiable, ok }。
 * `unverifiable` 单独成一类，**不进 broken** —— 门禁不该因为本地检出就少了目录而红。
 */
export function decideProvenance(results) {
  const counts = { ok: 0, broken: 0, 'offline-unverifiable': 0, 'ours-missing': 0 };
  const ok = [];
  const broken = [];
  const unverifiable = [];
  for (const r of results) {
    counts[r.status] = (counts[r.status] ?? 0) + 1;
    if (r.status === 'ok') ok.push(r);
    else if (r.status === 'offline-unverifiable') unverifiable.push(r);
    else broken.push(r);
  }
  return { counts, ok, broken, unverifiable };
}

// ————————————————————————————————————————————————————————————————
// 解析 markdown：只取表格单元格里与散落正文里的反引号片段
// ————————————————————————————————————————————————————————————————

/** 抽出所有反引号片段。表格与散文里约定的引用写法都是 `path:line`。 */
export function extractRefs(text) {
  const out = [];
  for (const line of text.split(/\r?\n/)) {
    // 表格行：整行都扫（列位置不影响判定，见 isUpstreamPath 的说明）
    for (const m of line.matchAll(/`([^`\n]+)`/g)) {
      if (looksLikeRef(m[1])) out.push(m[1].trim());
    }
  }
  return out;
}

/** 表头：`| 机制 | 我们的落点 | ... |` → 列名数组。 */
export function tableHeader(line) {
  return line
    .split('|')
    .slice(1, -1)
    .map((c) => c.trim());
}

/** 一行是不是表格分隔行（`|---|---|`）。 */
function isSeparator(line) {
  return /^\s*\|[\s:|-]+\|\s*$/.test(line);
}

// ————————————————————————————————————————————————————————————————
// 真实 IO：sparse 集合 + 行数
// ————————————————————————————————————————————————————————————————

const HERE = fileURLToPath(new URL('.', import.meta.url));
// 引用路径按「仓库根」解析，根默认就是脚本所在目录。
//
// **`--root` 是给自测用的**（理由见 main() 里的注释）：自测必须在**临时目录**里
// 造出被引用的上游文件，才能证明「真脚本读到真文件时判 ok」——
// 而不能靠本机恰好跑过 fetch-vendor.sh、仓库里恰好有 vendor/goose。
// 那个依赖曾经让 CI 红过一次，而本地一直是绿的。
let ROOT = HERE;
let OCTOP_DIR = join(ROOT, '.octop-ref', 'octop');

// 与 UPSTREAM.md 记录的 sparse 集合一致。git 读不到时用它兜底，并在输出里说明用了兜底 ——
// 静默用一个可能过期的集合会把「文件不在」判成「不在集合内」，那正好是本脚本要防的那类假通过。
const FALLBACK_SPARSE = [
  'dashboard/src/api',
  'dashboard/src/components',
  'dashboard/src/hooks',
  'dashboard/src/locales',
  'dashboard/src/pages/Chat',
  'dashboard/src/pages/Experts',
  'dashboard/src/routes',
  'dashboard/src/utils',
  'src/octop/infra/agents/experts',
];

function readSparseDirs() {
  try {
    const out = execFileSync('git', ['-C', OCTOP_DIR, 'sparse-checkout', 'list'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    const dirs = out.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
    if (dirs.length) return { dirs, source: 'git sparse-checkout list' };
  } catch {
    /* 落到兜底 */
  }
  return { dirs: FALLBACK_SPARSE, source: '内置兜底列表（git 读不到 sparse 集合）' };
}

/** 引用路径 → 本机绝对路径。octop/goose 相对路径分别挂在各自的上游根下。 */
function resolve(p) {
  const candidates = [join(ROOT, p), join(OCTOP_DIR, p), join(ROOT, 'vendor', 'goose', p)];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

const lineCache = new Map();
function lineCountOf(p) {
  if (lineCache.has(p)) return lineCache.get(p);
  const abs = resolve(p);
  let n = null;
  if (abs) {
    try {
      n = readFileSync(abs, 'utf8').split('\n').length;
    } catch {
      n = null;
    }
  }
  lineCache.set(p, n);
  return n;
}

/** 裸文件名能不能被钉死：仓库根上就有一个同名文件就算。 */
function rootHit(p) {
  if (p.includes('/')) return false;
  return existsSync(join(ROOT, p));
}

/** 一条 vendor 引用的「哪棵树」：`vendor/goose/…` → `vendor/goose`。 */
function treeRootOf(p) {
  return p.split('/').slice(0, 2).join('/');
}

/**
 * 那棵树取回来了吗。
 *
 * 「树整个都不在」与「树在、就这个文件没了」是两件事：
 * 前者是我们没跑 fetch-vendor.sh，说不上引用是不是腐烂；
 * 后者才是引用真的烂了。把前者报成 broken，等于把「没跑」说成「测了，而且坏了」。
 */
function treePresent(p) {
  if (!p.startsWith('vendor/')) return true;
  return existsSync(join(ROOT, treeRootOf(p)));
}

async function main() {
  // `--root` 只给自测用（见 ROOT 的注释）。不接受也没关系：默认就是脚本所在目录，
  // 本地跑 `node .provenance-check.mjs` 的行为一个字都没变。
  const rootAt = process.argv.indexOf('--root');
  const rootArg = rootAt >= 0 ? process.argv[rootAt + 1] : null;
  if (rootArg) {
    ROOT = isAbsolute(rootArg) ? rootArg : join(HERE, rootArg);
    OCTOP_DIR = join(ROOT, '.octop-ref', 'octop');
  }

  const flagAt = process.argv.indexOf('--index');
  const arg = flagAt >= 0 ? process.argv[flagAt + 1] : null;
  const indexPath = arg ? (isAbsolute(arg) ? arg : join(ROOT, arg)) : join(ROOT, 'UPSTREAM-USAGE.md');
  const indexLabel = arg ?? 'UPSTREAM-USAGE.md';
  if (!existsSync(indexPath)) {
    console.error(`读不到索引：${indexLabel} —— 没有索引可核。先写索引，再谈它会不会烂。`);
    process.exit(2);
  }
  const text = readFileSync(indexPath, 'utf8');

  // 结构性预检：没有「机制索引」表头就说明表格被改坏了，这时候说「引用都对」是谎话。
  const headers = text.split(/\r?\n/).filter((l) => l.startsWith('|')).map(tableHeader);
  const INDEX_COLS = ['机制', '我们的落点', '上游出处', '状态', '差别'];
  const hasIndex = headers.some((h) => INDEX_COLS.every((c, i) => h[i] === c));
  if (!hasIndex) {
    console.error(
      `索引里找不到列序为「${INDEX_COLS.join(' | ')}」的表头（${indexLabel}）。\n` +
        '索引的列序被改了或表被拆了 —— 这时候报「引用都对」没有任何意义。',
    );
    process.exit(2);
  }

  const sparse = readSparseDirs();
  const ctx = {
    lineCount: lineCountOf,
    inSparse: (p) => inSparseSet(p, sparse.dirs),
    rootHit,
    treePresent,
  };

  const rawRefs = extractRefs(text);
  if (!rawRefs.length) {
    console.error(`索引里一条引用都没解析出来（${indexLabel}）—— 解析器坏了，不是索引干净。`);
    process.exit(2);
  }

  const results = rawRefs.map((r) => classifyRef(r, ctx));
  const { counts, broken, unverifiable } = decideProvenance(results);

  console.log('=== 上游出处自检 ===');
  console.log(`索引：${indexLabel}，共解析出 ${rawRefs.length} 条引用（去重前）`);
  console.log(`octop sparse 集合来源：${sparse.source}（${sparse.dirs.length} 个目录）`);
  console.log('');
  console.log(
    `  已核且通过      ${counts.ok}\n` +
      `  核不到（sparse 外）${counts['offline-unverifiable']}\n` +
      `  坏了（判失败）   ${counts.broken + counts['ours-missing']}`,
  );

  if (unverifiable.length) {
    console.log('\n-- 核不到的地方（可见，不判失败） --');
    for (const r of unverifiable) {
      const loc = r.from ? `${r.path}:${r.from}${r.to !== r.from ? '-' + r.to : ''}` : r.path;
      console.log(`  ~ ${loc}`);
    }
  }

  if (broken.length) {
    console.log('\n-- 坏掉的引用（判失败） --');
    for (const r of broken) console.log(`  ! ${r.raw} —— ${r.reason}`);
    console.log('\n修法：把路径写全（含目录），行号对准真实文件；或者确认这条机制已经不用了，从索引里删掉。');
    console.log('编一个看起来对的行号比留空更坏 —— 它会让下一个人去核一个不存在的地方。');
    process.exitCode = 1;
    return;
  }

  console.log(`\nOK: ${counts.ok} 条引用逐行核过，没有一条坏的。`);
  if (unverifiable.length) {
    console.log(
      `另有 ${unverifiable.length} 条落在 octop 的 sparse 集合之外，**没能核**（上面已列出）。` +
        '「没核」不等于「核过没问题」。',
    );
  }
  process.exitCode = 0;
}

const isMain = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) await main();
