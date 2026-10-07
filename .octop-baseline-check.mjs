#!/usr/bin/env node
/**
 * 禁止把 `vendor/openoctopus-frontend/` 当成 Octop 引用。
 *
 * ## 这道门禁为什么存在
 *
 * 2026-10-08 出了一次真事故：实现「微信通道」时照着
 * `vendor/openoctopus-frontend/src/channels/api.ts` 写了三百行，事后才发现
 * 那个目录是 **另一个项目**（Zpoteiti/OpenOctopus，21 星的个人版），
 * 里面只有 discord/dingtalk 两个通道，**根本没有微信、没有 personalization**。
 * 用户开着的就是这个功能。
 *
 * 事故之所以能发生，是因为那份 vendor 树**在项目里躺了很久，并且看起来像权威**：
 * - `UPSTREAM.md:136` 把它正式列成「移植基准」
 * - `.vendor-baseline-check.mjs` 为它建了 sha256 门禁
 * - 多处文档特意注明「这是第三个项目，不是 Octop」
 *
 * 也就是说项目**早就知道**自己手上多了一个来路不明的第三方（BACKLOG B6-4
 * 专门记了「它来自哪个仓库全项目没记」），却没人回头问一句
 * 「它凭什么在这儿」。免责声明写得越详细，读的人反而越容易以为
 * 「既然有门禁有说明，那它就是基准」。
 *
 * 真 Octop 是 `.octop-ref/octop/`（`github.com/TencentCloud/Octop`）。
 *
 * ## 判据
 *
 * 全仓扫一遍：任何把 `vendor/openoctopus-frontend` 当作「Octop 功能参考」
 * 使用的引用都报出来。允许存在的位置只有两类，且都必须在白名单里：
 * 1. `UPSTREAM.md` / `UPSTREAM-USAGE.md` / `README` 等**说明它是什么**的文档
 * 2. `.vendor-baseline-check.mjs` 与 `overrides.css` 的**CSS 字节一致性**约束
 *
 * 「有白名单」是关键：不列白名单的话，连本文件自己都会把自己报红。
 * 白名单里逐条写了为什么允许，白名单扩大需要有人在提交里说明理由 ——
 * 这正是这道门禁想逼出来的那句话。
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, relative, sep } from 'node:path';

// `new URL('.', …)` 已经以尾斜杠结尾，HERE 就是**本文件所在目录**。
// 这里曾经写成 `join(HERE, '..')`，于是扫的是仓库的父目录 —— 那里除了
// `.` 和 `..` 没有文本文件，于是这道门禁永远输出 OK，从没扫到过任何东西。
// 是靠变异验证抓出来的（注入一处违规引用，门禁照样报绿）。
// 不要为了「再退一层」再加 `..`。
const HERE = fileURLToPath(new URL('.', import.meta.url));
const ROOT = HERE;
const NEEDLE = 'vendor/openoctopus-frontend';

/** 真 Octop 的路径。指向它的引用才是在学 Octop。 */
const REAL_OCTOP = '.octop-ref/octop';

/**
 * 允许提到 `vendor/openoctopus-frontend` 的文件。
 *
 * 每条都要写清「为什么允许」—— 白名单是这道门禁唯一可能被滥用的地方，
 * 所以它必须让人看见自己加了什么。
 *
 * **粒度说明**：这里按**文件**放行。第一版试图做成行级（只放行「解释它
 * 是什么」的那几行），但那样白名单会变成一串行号，任何人插一行就能绕过，
 * 而且改个行号就要改门禁。所以停在文件级：
 * - 它拦住的是「把这份 vendor 当 Octop 功能参考」这个真实错误用法，
 *   那种引用**只可能出现在源码与设计文档里**；
 * - 代价是白名单文件内部不再检查。那几个文件都是说明性的（记录它是什么、
 *   或者做 CSS 字节校验），没人会在里面藏功能引用。
 *
 * 真要更严，得改成「按引用意图判定」而不是按路径 —— 那是另一件事，
 * 不该靠加行号硬凑。
 */
const ALLOWED = new Map([
  // 门禁自己：它就是 CSS 字节一致性的那道检查，也是本题的主角。
  ['.vendor-baseline-check.mjs', '它守的正是这棵树的 index.css'],
  ['.octop-baseline-check.mjs', '这道门禁自己 —— 它必须能说出那个路径，否则无法解释自己在查什么'],
  // 门禁接线：跑门禁的地方要说明这道门禁的来历。
  ['.github/workflows/gates.yml', 'CI 里跑这道门禁，并引用 CSS 基准'],
  ['.scripts/gates.sh', '本地跑这道门禁，并引用 CSS 基准'],
  // 样式侧说明：index.css 不可改的原因就是与这棵树逐字节一致。
  ['ui/web/src/overrides.css', 'CSS 字节一致性约束的落点'],
  ['ui/web/src/chat/chatShell.css', '同一约束的转述'],
  ['ui/web/README.md', '登记前端代码的出处（含 CSS 移植基准）'],
  // 文档：说明「这是另一个项目，不是 Octop」。
  ['UPSTREAM.md', '登记它是什么，并写明 Octop ≠ OpenOctopus'],
  ['UPSTREAM-USAGE.md', '同上'],
  ['project/items.mjs', 'B6-4 记着它的来源问题'],
  ['BACKLOG.md', 'B6-4 记着它的来源问题'],
  ['TESTSETS/ISSUES.md', '事故记录：那份 vendor 从哪来、为什么不能当 Octop 用'],
  ['.provenance-check.mjs', '引用分类器要认识这棵树'],
  ['.scripts/provenance-selftest.mjs', '同上（自测）'],
  // 一次性 vendor 辅助脚本：它们只做 CSS 校验与 vendor 搬移。
  ['.wsl-css-check.sh', '校验 index.css 与这棵树逐字节一致'],
  ['.wsl-restore-vendor.sh', '把 vendor 搬回本机（按树名分工）'],
  ['.wsl-unvendor.sh', '从 vendor 反向核对'],
  ['.wsl-verify-vendor.sh', '检查 vendor 是否在位'],
  ['.gitignore', '忽略 vendor'],
]);

const SKIP_DIRS = new Set([
  '.git', 'node_modules', 'target', 'vendor', '.octop-ref',
  'dist', '.scratch', 'coverage', 'test-results', 'playwright-report',
  'out', '.next', '.cache', '.venv', 'venv', '__pycache__',
  '.pytest_cache', '.mypy_cache', '.turbo', '.parcel-cache', '.gradle',
]);

const TEXT_EXT = new Set([
  '.md', '.mjs', '.js', '.ts', '.tsx', '.json', '.rs', '.sql', '.css',
  '.yml', '.yaml', '.toml', '.sh', '.ps1', '.txt',
]);

/** 文件数上限。这道门禁要进 CI，慢了就不会有人跑。 */
const SCAN_LIMIT = 20_000;

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const full = join(dir, name);
    let st;
    try {
      st = statSync(full);
    } catch {
      continue;
    }
    if (st.isDirectory()) yield* walk(full);
    else if (isText(name)) yield full;
  }
}

function isText(name) {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return false;
  // 集合里存的是带点的形式（'.md'），而 slice 出来的不带点（'md'）。
  // 写成 `TEXT_EXT.has('.' + ext)` 或把集合里的点去掉，二者必须一致 ——
  // 这里曾经两边不一致，于是每个文件都判成「非文本」，一道门禁扫了 0 个文件。
  return TEXT_EXT.has(`.${name.slice(dot + 1)}`);
}

export function classify(filePath) {
  const rel = filePath.split(sep).join('/');
  if (ALLOWED.has(rel)) return { allowed: true, why: ALLOWED.get(rel) };
  return { allowed: false };
}

/**
 * 一行文本里提到 vendor/openoctopus-frontend 时，返回它所在行。
 * 返回 null 表示这行没问题。
 */
export function inspectLine(relPath, line) {
  if (!line.includes(NEEDLE)) return null;
  if (classify(relPath).allowed) return null;
  // 真正的 Octop 引用走 .octop-ref/octop，不会命中这个路径。
  // 到这里就是在拿另一个项目当 Octop 的功能参考。
  return { path: relPath, line: line.trim().slice(0, 160) };
}

function selfTest() {
  const cases = [
    ['白名单文件里的提及 → 放行', 'UPSTREAM.md', `x ${NEEDLE}/src/index.css:1`, null],
    ['非白名单文件提到 → 报出', 'crates/quill-server/src/x.rs', `// 照 ${NEEDLE}/src/channels/api.ts`, '非白名单'],
    ['提到真 Octop → 不报', 'crates/quill-server/src/x.rs', `// 照 ${REAL_OCTOP}/src/octop/...`, null],
    ['没提到 → 不报', 'crates/quill-server/src/x.rs', '// 无关键词', null],
  ];
  let bad = 0;
  for (const [label, p, line, expectHit] of cases) {
    const hit = inspectLine(p, line);
    const got = hit ? '非白名单' : null;
    const ok = got === expectHit;
    if (!ok) bad++;
    console.log(`  ${ok ? '✓' : '✗'} ${label}`);
  }
  if (bad) {
    console.error(`\n自测失败：${bad} 条不符合预期`);
    process.exit(1);
  }
  console.log('  自测通过\n');
}

function main() {
  if (process.argv.includes('--self-test')) {
    selfTest();
    return;
  }
  const findings = [];
  let scanned = 0;
  for (const full of walk(ROOT)) {
    const rel = relative(ROOT, full).split(sep).join('/');
    if (classify(rel).allowed) continue;
    scanned++;
    // 扫的目录集写死了（SKIP_DIRS）。文件数突然变多说明有人把新的大目录
    // 引进来了 —— 那时这道门禁会变慢到没人愿意跑，宁可当场报错。
    if (scanned > SCAN_LIMIT) {
      console.error(
        `✗ 扫描超过 ${SCAN_LIMIT} 个文件就停了。\n` +
        '  SKIP_DIRS 少了一个大目录（node_modules / target / .scratch 之类）。\n' +
        '  这道门禁要进 CI，变慢等于失效。',
      );
      process.exit(1);
    }
    let text;
    try {
      text = readFileSync(full, 'utf8');
    } catch {
      continue;
    }
    const lines = text.split('\n');
    for (let i = 0; i < lines.length; i++) {
      const hit = inspectLine(rel, lines[i]);
      if (hit) findings.push({ ...hit, lineNo: i + 1 });
    }
  }

  if (findings.length) {
    console.error('✗ 把另一个项目（Zpoteiti/OpenOctopus）当 Octop 引用了：\n');
    for (const f of findings) {
      console.error(`  ${f.path}:${f.lineNo}`);
      console.error(`    ${f.line}`);
    }
    console.error(
      '\n真 Octop 是 `github.com/TencentCloud/Octop`，本地检出在 .octop-ref/octop/。',
    );
    console.error(
      'vendor/openoctopus-frontend 只提供 CSS 设计系统，不能作为 Octop 功能参考。',
    );
    console.error(
      '若某处确实只是 CSS 字节一致性约束，把它加进 .octop-baseline-check.mjs 的白名单并写明理由。',
    );
    process.exit(1);
  }
  console.log(`OK: 没有把 ${NEEDLE} 当 Octop 引用（扫了 ${scanned} 个文件，根目录 ${ROOT}）`);
}

if (process.argv[1] && process.argv[1].endsWith('octop-baseline-check.mjs')) {
  main();
}