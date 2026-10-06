#!/usr/bin/env node
/**
 * 当场算出「现在到底什么状态」，不读任何手工维护的状态标记。
 *
 * 用法：
 *   node scripts/status.mjs              全量评估（会跑 cargo test 与 vitest，分钟级）
 *   node scripts/status.mjs --quick      只跑便宜的判据（秒级）；test 类判据标为「未复查」
 *   node scripts/status.mjs --json       机器可读输出，供别的工具消费
 *   node scripts/status.mjs --self-check 只校验「判据声明本身是否成立」，不跑判据
 *
 * **它凭什么可信**：唯一的输入是「跑一下看结果」。没有任何一条结论来自文档，
 * 也没有任何一条结论来自上一次跑的结果（不缓存、不继承）。所以它不可能说谎，
 * 除非判定逻辑本身写错了 —— 那个由 status-selftest.mjs 钉住。
 *
 * **为什么 `--self-check` 是必须的**：一条绑错了名字的 test 判据（比如测试改名了）
 * 会永远显示「未通过」，看着很刺眼所以会有人去修；而一条绑了**不存在**的名字的
 * 判据如果被写成「存在即通过」，就会永远显示「已验证」—— 那才是要命的一种。
 * 所以 self-check 专门抓这一类。
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync, execSync } from 'node:child_process';

import { ITEMS, MILESTONE_ORDER, MILESTONE_NAMES } from '../project/items.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const WSL = 'wsl.exe';

/**
 * 我们是不是**已经在 WSL 里面**了？
 *
 * 这不是优化，是修一个会卡死门禁的缺陷。`gates.sh` 的常规跑法就是
 * `wsl.exe -e bash -lc 'bash .scripts/gates.sh'` —— 也就是门禁本身跑在 WSL 里，
 * 而它新接的那道 `--self-check` 会调 `status.mjs`。如果 `status.mjs` 无条件
 * 再去 fork 一个 `wsl.exe`，就成了 **WSL 套 WSL**：实测从 WSL 内部这样调，
 * 五分钟零输出，最后只能靠 30 分钟的 timeout 兜底。
 *
 * 那道 timeout 是 `runWsl` 里写死的 30 分钟。也就是说这个缺陷不会报错，
 * 它只会让每次门禁**静悄悄地多花半小时**，然后在超时后把判据报成
 * 「无法收集测试清单」。这正是本仓库吃过两次的那种亏的第三个变种：
 * 不是判错了，是压根没跑成，却看起来像跑过了。
 *
 * 判据用两个独立信号，避免单点误判：
 *   · `/proc/version` 含 microsoft —— WSL1/WSL2 都有
 *   · `WSL_DISTRO_NAME` 环境变量 —— WSL2 注入，WSL1 不注入
 * 两者都不成立，才走 wsl.exe 那条路（即从 Windows 原生侧跑）。
 */
function detectInsideWsl() {
  if (process.platform !== 'linux') return false;
  if (process.env.WSL_DISTRO_NAME || process.env.WSL_INTEROP) return true;
  try {
    return /microsoft/i.test(fs.readFileSync('/proc/version', 'utf8'));
  } catch {
    return false;
  }
}

const INSIDE_WSL = detectInsideWsl();

const argv = new Set(process.argv.slice(2));
const QUICK = argv.has('--quick');
const AS_JSON = argv.has('--json');
const SELF_CHECK = argv.has('--self-check');

export const VERDICT = {
  PASS: '已验证',
  FAIL: '未通过',
  MANUAL: '需人工',
  UNCHECKED: '未复查',
  BROKEN: '判据本身坏了',
};

function log(...a) {
  if (!AS_JSON) console.log(...a);
}

function run(cmd, opts = {}) {
  try {
    return { ok: true, out: execSync(cmd, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts }) };
  } catch (e) {
    return { ok: false, out: (e.stdout || '') + (e.stderr || ''), code: e.status ?? 1 };
  }
}

/**
 * 跑一条 bash 命令，**只跑一次**。
 *
 * 已经在 WSL 里就直接 `bash -lc`，别再套一层 `wsl.exe`（理由见 detectInsideWsl）。
 * 顺带解决第二个坑：从 WSL 内部调用时 `ROOT` 是个 Windows 路径
 * （`D:\96_CoderWorld\quill`），把它当 cwd 传给 execSync 会直接报
 * ENOENT。所以 WSL 内部不传 cwd —— 每条命令自己 `cd` 到绝对路径。
 */
/**
 * 给「本机命令」一个**当前平台认得**的 cwd。
 *
 * ROOT 是从 import.meta.url 算出来的，恒为 Windows 形态（`D:\...`）。
 * 从 WSL 内部运行时把它直接当 cwd 传给 execSync 会 ENOENT —— 命令根本没跑，
 * 却被记成「未通过」。那是把环境问题伪装成结论，比不跑更坏。
 * 所以在 WSL 内部把它换成 /mnt/d/... 形态。
 */
function hostCwd() {
  if (!INSIDE_WSL) return ROOT;
  const m = /^([A-Za-z]):[\\/](.*)$/.exec(ROOT);
  if (!m) return undefined;
  return `/mnt/${m[1].toLowerCase()}/${m[2].replace(/\\/g, '/')}`;
}

function runWsl(bashCmd) {
  if (INSIDE_WSL) {
    return run(`bash -lc ${JSON.stringify(bashCmd)}`, { timeout: 30 * 60 * 1000 });
  }
  return run(`"${WSL}" -e bash -lc ${JSON.stringify(bashCmd)}`, { cwd: ROOT, timeout: 30 * 60 * 1000 });
}

/**
 * 跑一条**判据命令**：base64 写进临时脚本，再 `bash` 它。
 *
 * **为什么不能直接把判据塞进 `bash -lc "…"`**（这个坑踩过）：
 * 判据字符串会被 `JSON.stringify` 包一层双引号，而判据自己往往还含
 * `$(...)`、`%(...)` 和引号 —— 套了好几层 shell，每一层都要重新解释一次引号。
 *
 * 实测 B0-6（`test $(git branch --format="%(refname:short)" | grep -cvx main) -eq 0`）
 * 在终端里手工跑退出 0（通过），经这条路跑却退出 2，被如实报成「未通过」。
 * 那是环境问题伪装成结论 —— 正是这套机制最不该出的错。
 *
 * base64 里只有 `A-Za-z0-9+/=`，任何一层 shell 都不会改它。
 * 顺带在脚本里显式 `cd` 到仓库根：否则判据跑在「谁敲的 node 命令」的目录上，
 * 而判据本来是跟目录无关的。
 */
function runWslScript(body) {
  const repo = hostCwd() || '/mnt/d/96_CoderWorld/quill';
  const wrapped = `cd ${JSON.stringify(repo)} || exit 1\n${body}\n`;
  const b64 = Buffer.from(wrapped, 'utf8').toString('base64');
  return runWsl(
    `echo ${b64} | base64 -d > /tmp/quill-status-cmd.sh && bash /tmp/quill-status-cmd.sh`,
  );
}

/* ————————————————————————————————————————————————————————————————
 * 一、收集证据（每样只收集一次，绝不为每条判据各跑一遍）
 * ———————————————————————————————————————————————————————————————— */

function collectRustTests() {
  const r = runWsl(
    'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/d/96_CoderWorld/quill && cargo test --workspace -- --list 2>/dev/null | grep -E ": test$" | sed "s/: test$//" | sort -u',
  );
  if (!r.ok) return null;
  return new Set(r.out.split('\n').map((s) => s.trim()).filter(Boolean));
}

const FE_JSON = '/tmp/quill-status-vitest.json';

/**
 * 前端用例全名 = `仓库相对路径 > describe... > 用例名`。
 *
 * **单独抽出来，因为这段曾经是错的，而且自测没抓到**：
 * 它原来只取文件名，于是生成 `ChatPage.test.tsx > 用例名`，
 * 而 `project/items.mjs` 里写的是 `src/chat/ChatPage.test.tsx > 用例名` ——
 * 两边对不上，三条判据全被判成「判据本身坏了」。
 *
 * 为什么自测没抓到：旧自测把 `'fe > 用例甲'` 直接**手写**进证据集，
 * 名字是假的，从来没经过这个函数。测的是判定，不是拼名字 —— 测错了层。
 * 所以现在这个函数必须导出、自测必须用它的真实输出当判据名。
 *
 * 纯函数，不碰文件不跑命令，所以能被喂穷尽。
 */
export function feFullName(absPath, ancestorTitles, title) {
  const norm = String(absPath || '').replace(/\\/g, '/');
  // 砍到 `ui/web/` 为止；找不到就退化成原样（不猜，宁可让判据绑不上，
  // 那会被显式报成「判据坏了」，而不是悄悄匹配错对象）。
  const i = norm.lastIndexOf('ui/web/');
  const rel = i >= 0 ? norm.slice(i + 'ui/web/'.length) : norm;
  const anc = Array.isArray(ancestorTitles) ? ancestorTitles.filter(Boolean) : [];
  return [rel, ...anc, title].join(' > ');
}

function collectFrontendTests() {
  const r = runWsl(
    'export PATH="$HOME/.cargo/bin:$PATH"; [ -x "$HOME/.local/node/bin/node" ] && export PATH="$HOME/.local/node/bin:$PATH"; ' +
      `cd /mnt/d/96_CoderWorld/quill/ui/web && npx vitest run --reporter=json --outputFile=${FE_JSON} >/dev/null 2>&1`,
  );
  if (!r.ok) return null;
  let doc;
  try {
    doc = JSON.parse(fs.readFileSync(FE_JSON, 'utf8'));
  } catch (e) {
    log(`  ⚠ 读不到 vitest 的 json 输出（${e.message}），前端判据不可信`);
    return null;
  }
  const known = new Set();
  const passing = new Set();
  for (const file of doc.testResults || []) {
    for (const a of file.assertionResults || []) {
      const full = feFullName(file.name, a.ancestorTitles, a.title);
      known.add(full);
      if (a.status === 'passed') passing.add(full);
    }
  }
  return { known, passing };
}

/**
 * Rust 侧：跑一次全量测试，解析出「哪些真的 ok」。
 *
 * 用一次全量而不是逐个 `--exact`：几百条判据就是几百次编译，那不叫门禁，
 * 叫折磨。cargo 的 `test <名字> ... ok` 这一行格式稳定，解析它足够。
 */
function collectPassingRust() {
  const r = runWsl(
    'export PATH="$HOME/.cargo/bin:$PATH"; cd /mnt/d/96_CoderWorld/quill && cargo test --workspace 2>&1',
  );
  if (!r.ok) return null;
  const out = new Set();
  for (const line of r.out.split('\n')) {
    const m = /^\s*test\s+(\S+)\s+\.\.\.\s+ok\s*$/.exec(line);
    if (m) out.add(m[1]);
  }
  return out;
}

/* ————————————————————————————————————————————————————————————————
 * 二、判定（纯函数 —— 这段被 status-selftest.mjs 直接测）
 * ———————————————————————————————————————————————————————————————— */

/**
 * 给单条判据下结论。**必须是纯函数**：不读文件、不跑命令、不看时间。
 * 它的输入全部由调用方收集好 —— 这样自测才能喂合成数据把它测穷尽。
 *
 * 返回 { verdict, detail }
 */
export function decide(item, evidence) {
  const v = item.verify;
  switch (v.kind) {
    case 'test': {
      if (!evidence.testsKnown) {
        return { verdict: VERDICT.UNCHECKED, detail: evidence.quick ? 'quick 模式未跑测试' : '无法收集测试清单' };
      }
      // 先判「这个名字存不存在」，再判「过没过」。这两件事必须分开报：
      // 测试改名之后，一条绑旧名字的判据会永远显示「未通过」——
      // 刺眼、会有人去修；而把两者混成一句话，就看不出是改名还是回归。
      if (!evidence.knownTests.has(v.name)) {
        return { verdict: VERDICT.BROKEN, detail: `测试「${v.name}」不存在 —— 判据绑错了名字` };
      }
      if (!evidence.passing.has(v.name)) {
        return { verdict: VERDICT.FAIL, detail: `测试「${v.name}」存在但没通过` };
      }
      return { verdict: VERDICT.PASS, detail: `测试「${v.name}」通过` };
    }
    case 'cmd': {
      if (evidence.quick && v.slow) {
        return { verdict: VERDICT.UNCHECKED, detail: 'quick 模式跳过（该命令较慢）' };
      }
      return evidence.cmdResults[v.cmd] ?? { verdict: VERDICT.UNCHECKED, detail: '未执行' };
    }
    case 'absent': {
      const exists = evidence.existingPaths.has(v.path);
      return exists
        ? { verdict: VERDICT.FAIL, detail: `${v.path} 又出现了` }
        : { verdict: VERDICT.PASS, detail: `${v.path} 确实不存在` };
    }
    case 'manual':
      return { verdict: VERDICT.MANUAL, detail: v.how || '只能人工验证' };
    default:
      return { verdict: VERDICT.BROKEN, detail: `未知的判据种类：${v.kind}` };
  }
}

/* ————————————————————————————————————————————————————————————————
 * 三、self-check：验证「判据声明本身」是否成立
 * ———————————————————————————————————————————————————————————————— */

export function selfCheck(items, knownTests) {
  const problems = [];
  for (const it of items) {
    const v = it.verify;
    if (!v || !v.kind) {
      problems.push(`${it.id}: 缺 verify`);
      continue;
    }
    if (v.kind === 'test' && knownTests && !knownTests.has(v.name)) {
      problems.push(`${it.id}: 绑的测试「${v.name}」不存在 —— 这条判据永远不可能通过`);
    }
    if (v.kind === 'cmd' && !v.cmd) problems.push(`${it.id}: kind=cmd 但没有 cmd`);
    if (v.kind === 'absent' && !v.path) problems.push(`${it.id}: kind=absent 但没有 path`);
    if (v.kind === 'manual' && !v.how) problems.push(`${it.id}: kind=manual 但没写怎么人工验`);
  }
  // 重复 id 会让「按 id 找一条待办」变得有歧义。
  const seen = new Set();
  for (const it of items) {
    if (seen.has(it.id)) problems.push(`id 重复：${it.id}`);
    seen.add(it.id);
  }
  return problems;
}

/* ————————————————————————————————————————————————————————————————
 * 四、主流程
 * ———————————————————————————————————————————————————————————————— */

function main() {
  // cmd 判据去重后只跑一次。
  const cmdSpecs = [...new Set(ITEMS.filter((i) => i.verify.kind === 'cmd').map((i) => i.verify.cmd))];

  // 「有哪些测试」与「哪些测试过了」必须**分别**收集：
  //   · 存在的清单要包含失败的测试（否则一条测试挂了，判据会误报成
  //     「判据绑错了名字」，把「回归」说成「改名」）；
  //   · 通过的清单只收真的 ok 的。
  const knownTests = new Set();
  const passing = new Set();

  if (!QUICK) {
    const rustKnown = collectRustTests();
    if (rustKnown) for (const t of rustKnown) knownTests.add(t);
    else log('  ⚠ 收集不到 Rust 测试清单，Rust 侧判据不可信');

    // 自检只需要「有哪些测试」，不需要「哪些过了」——
    // 所以 self-check 模式下跳过全量测试那次跑。
    //
    // **为什么必须跳**：`--self-check` 是 `bash .scripts/gates.sh` 里的一道，
    // 而 gates.sh 本身又是 `project/items.mjs` 里一条 cmd 判据的命令
    // （B0-2 全量门禁 / B0-3 fast 门禁）。于是全量跑 status 时，
    // gates.sh 会把 `--self-check` 拉成子进程。
    // 若它自己也要跑一遍全量 cargo test，一次 status 就会在 gates.sh
    // 已经在跑测试的同时再拉起一份，两边互相等cargo 锁 ——
    // 第一次跑就是这么卡住的。
    if (!SELF_CHECK) {
      const r = collectPassingRust();
      if (r) for (const t of r) passing.add(t);
      else log('  ⚠ cargo test 未成功，Rust 侧判据不可信');
    }

    const fe = collectFrontendTests();
    if (fe) {
      for (const t of fe.known) knownTests.add(t);
      if (!SELF_CHECK) for (const t of fe.passing) passing.add(t);
    } else {
      log('  ⚠ vitest 未成功，前端侧判据不可信');
    }
  }

  if (SELF_CHECK) {
    const problems = selfCheck(ITEMS, knownTests.size ? knownTests : null);
    if (problems.length === 0) {
      log('判据声明自检：全部成立');
      return 0;
    }
    for (const p of problems) log(`  ✗ ${p}`);
    log(`判据声明自检：${problems.length} 处有问题`);
    return 1;
  }

  const existingPaths = new Set();
  for (const it of ITEMS) {
    if (it.verify.kind === 'absent') {
      // 只需要知道「在不在」。两处都要用当前平台认得的根目录：
      // 从 WSL 内部跑时 ROOT 是 `D:\...`，`git -C` 与 fs.existsSync 都不认，
      // 于是判据会假报「不存在 → 已验证」—— 那正是本判据最不能出的错。
      const base = hostCwd() ?? ROOT;
      try {
        execFileSync('git', ['-C', base, 'ls-files', '--error-unmatch', it.verify.path], { stdio: 'ignore' });
        existingPaths.add(it.verify.path);
      } catch {
        try {
          if (fs.existsSync(path.join(base, it.verify.path))) existingPaths.add(it.verify.path);
        } catch { /* 不存在就对了 */ }
      }
    }
  }

  const cmdResults = {};
  for (const cmd of cmdSpecs) {
    const spec = ITEMS.find((i) => i.verify.kind === 'cmd' && i.verify.cmd === cmd).verify;
    // **quick 模式下真的不执行**，而不是执行完再报「跳过」。
    //
    // 这里曾经只让 `decide()` 把 slow 判据显示成「跳过（该命令较慢）」，
    // 可命令照样跑了一遍 —— 于是一句「跳过」是句谎话，
    // `node scripts/status.mjs --quick` 实测要跑 7 分半。
    // 报告说自己跳过了、实际没跳过，比如实说「很慢」坏得多。
    if (QUICK && spec.slow) continue;
    const r = spec.cwd === 'wsl' ? runWslScript(cmd) : run(cmd, { cwd: hostCwd() });
    cmdResults[cmd] = r.ok
      ? { verdict: VERDICT.PASS, detail: `命令退出 0：${cmd}` }
      : { verdict: VERDICT.FAIL, detail: `命令失败（退出码 ${r.code}）：${cmd}` };
  }

  const evidence = {
    quick: QUICK,
    testsKnown: knownTests.size > 0,
    knownTests,
    passing,
    existingPaths,
    cmdResults,
  };

  const results = ITEMS.map((it) => ({ ...it, ...decide(it, evidence) }));

  if (AS_JSON) {
    console.log(
      JSON.stringify(
        {
          generated_at: new Date().toISOString(),
          quick: QUICK,
          counts: countBy(results.map((r) => r.verdict)),
          items: results.map((r) => ({
            id: r.id, milestone: r.milestone, title: r.title,
            kind: r.verify.kind, verdict: r.verdict, detail: r.detail,
          })),
        },
        null,
        2,
      ),
    );
  } else {
    render(results);
  }

  // 退出码：只有「全部已验证/需人工」才算 0。
  // 需人工不算失败 —— 它是**公开的缺口**，会单列一栏提醒，不该把门禁刷红，
  // 但也绝不能被算成「验过了」。
  const bad = results.filter((r) => r.verdict === VERDICT.FAIL || r.verdict === VERDICT.BROKEN);
  return bad.length === 0 ? 0 : 1;
}

function countBy(list) {
  const c = {};
  for (const x of list) c[x] = (c[x] || 0) + 1;
  return c;
}

function render(results) {
  const counts = countBy(results.map((r) => r.verdict));
  log('');
  log('='.repeat(72));
  log('项目状态 —— 由 scripts/status.mjs 当场算出，不读任何手工维护的标记');
  if (QUICK) log('（quick 模式：只跑便宜的判据，test 类标为「未复查」）');
  log('='.repeat(72));

  for (const m of MILESTONE_ORDER) {
    const rows = results.filter((r) => r.milestone === m);
    if (!rows.length) continue;
    const bad = rows.filter((r) => r.verdict === VERDICT.FAIL || r.verdict === VERDICT.BROKEN).length;
    const man = rows.filter((r) => r.verdict === VERDICT.MANUAL).length;
    log('');
    log(`${m} · ${MILESTONE_NAMES[m]}  —— ${rows.length} 条` +
        (bad ? `，${bad} 条未通过` : '') + (man ? `，${man} 条需人工` : ''));
    for (const r of rows) {
      const mark = { [VERDICT.PASS]: '✓', [VERDICT.FAIL]: '✗', [VERDICT.MANUAL]: '○', [VERDICT.UNCHECKED]: '·', [VERDICT.BROKEN]: '!' }[r.verdict];
      log(`  ${mark} ${r.id.padEnd(7)} ${r.title}`);
      if (r.verdict !== VERDICT.PASS) log(`      ${r.detail}`);
    }
  }

  log('');
  log('—'.repeat(72));
  log(`合计 ${results.length} 条：` +
      Object.entries(counts).map(([k, v]) => `${k} ${v}`).join('，'));
  if (counts[VERDICT.MANUAL]) {
    log('');
    log(`⚠ ${counts[VERDICT.MANUAL]} 条判据只能人工验证。它们**没有被机器检查过**，`);
    log('  「需人工」既不是通过也不是失败 —— 它是一个公开的缺口。');
  }
  if (counts[VERDICT.BROKEN]) {
    log('');
    log(`✗ ${counts[VERDICT.BROKEN]} 条判据本身坏了（绑的名字不存在等）。`);
    log('  这比未通过更严重：它意味着那条待办的真实状态是未知的。');
  }
  log('—'.repeat(72));
}

// 被别的脚本 import 时（status-selftest.mjs 要拿 decide/selfCheck 喂合成数据）
// 绝不能顺手把 main 跑一遍 —— 那是 CLI 的事，导入不是。
const isMain = process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]);
if (isMain) process.exit(main());
