#!/usr/bin/env node
/**
 * 分层守卫：`docs/ARCHITECTURE.md §3.1` 的硬约束「依赖只能向下」。
 *
 * ## 这道门禁回答什么问题
 *
 * 某 crate 的 `[dependencies]` 里出现的 `quill-*` 依赖，层级是否**严格小于**自己。
 * 为什么必须有门禁：在 `Cargo.toml` 里加一行 `quill-wiki = { path = "../quill-wiki" }`
 * 就能让一个 L2 crate 反向依赖另一个 L2/L3 crate，而 `cargo build` / `cargo test`
 * 会照常全绿 —— 层次倒挂只存在于依赖图里，review 里几乎看不出来。
 * 本仓库吃过一次实亏：`quill-agent -> quill-wiki` 的倒挂（见 docs/ARCHITECTURE.md §1.1/§2）
 * 就是靠人事后翻文档才发现的。
 *
 * ## 分层表（docs/ARCHITECTURE.md §3.1，`quill-testkit` 为按代码事实补定）
 *
 * | 层 | crate |
 * |---|---|
 * | L0 | quill-adapters, quill-store, quill-provider |
 * | L1 | quill-domain |
 * | L2 | quill-agent, quill-control, quill-wiki, quill-backup, quill-upgrade |
 * | L3 | quill-core（Agent 内核层，照 vendor/goose 移植） |
 * | L4 | quill-server |
 * | L5 | quill-cli |
 * | L1 | quill-testkit（测试夹具层） |
 *
 * `quill-testkit` 不在 §3.1 的表里，层号是按**代码里核到的事实**补定的（2026-10-08）：
 *   1. 它自己 `[dependencies]` 只有 `quill-adapters`（L0）——所以它的层必须 > 0；
 *   2. 全仓库只有 `quill-agent` 在 `[dev-dependencies]` 里用它 ——它不参与发布依赖图，
 *      属于「测试/夹具」性质；
 *   3. 取**满足第 1 条的最小整数层 L1**：既不与 L0 叶子平级（那会把自己的 L0 依赖
 *      判成同层违例），也不虚高到挡住将来别的层在测试里引用它（dev 依赖本就放宽）。
 *   复核命令：`grep -rn "quill-" crates/quill-testkit/Cargo.toml crates/<每个crate>/Cargo.toml | grep testkit`
 *
 * ## 判据（扫什么、怎么判）
 *
 * - 只扫 `crates/<crate>/Cargo.toml`（workspace 成员的清单，每个 crate 目录一份）。
 *   刻意**不递归**进 `fixtures/`：那下面曾是合成的假清单（`quill-testkit` 的
 *   boundary 测试数据），不是真依赖声明，扫它等于拿测试数据当事实。
 *   那批数据已按 queue Q101 删除（没有读者）；不递归这条保留是防御性的。
 * - `[dependencies]`（含 `[build-dependencies]`、`[target.*.dependencies]`、
 *   `[dependencies.<name>]` 子表等一切会进入真实依赖图的形式）：
 *   依赖层级必须**严格小于**自己；同层（`L==L`）也是违例。
 * - `[dev-dependencies]`：放宽（测试可以对上/向上，§3.1 只管发布依赖图），
 *   但**逐条如实列出**，不装作没看见。
 * - 出现不在分层表里的 `quill-*` 依赖键 → 报红：先登记层，再谈合规
 *   （否则新 crate 可以借「表里没它」绕过规则）。
 *
 * ## 已知违例的基线（语义：只减不增）
 *
 * 语义：**基线里的违例被修好却还留在清单里（陈旧条目）→ 也退出码 1**，
 * 逼着这份清单只减不增。基线只登记「代码里当场核到」的既成事实，不登记推测。
 *
 * 基线清单（每条附中文理由）：
 *
 * 1. `quill-upgrade(L2) -> quill-backup(L2)`，`[dependencies]`，同层依赖。
 *    理由：既成事实 —— quill-upgrade 的「升级前强制备份」直接调用 quill-backup
 *    的实现（quill-upgrade/src 里 `use quill_backup::…`）。§3.1 把这两者并列为 L2，
 *    「严格小于」之下这就是同层违例。修复方向（留给后续任务，不由本脚本决定）：
 *    把调用改为经 L1/L0 接口，或在 docs/ARCHITECTURE.md §3.1 里承认
 *    quill-upgrade 位于 quill-backup 之上（改分层表要文档与代码同步）。
 *    复现：`grep -n "quill-backup" crates/quill-upgrade/Cargo.toml`
 *
 * **不在基线里的历史违例（2026-10-08 复核）**：`quill-agent -> quill-wiki`。
 * docs/ARCHITECTURE.md §1.1 还记着它，但代码里**已经被修掉**（commit 92a1d73
 * 「删了 quill-agent 对 quill-wiki 的依赖」），所以刻意不登记 —— 把已修复的条目
 * 留在基线里会被本脚本判成「陈旧条目」而报红，这是设计好的自净机制。
 * 复核：`grep -c "quill-wiki" crates/quill-agent/Cargo.toml`（→ 0）。
 *
 * 全量复核（人工核对基线与分层表时跑这个）：
 *   for d in crates/*; do n=$(basename "$d"); \
 *     deps=$(grep -oE 'quill-[a-z]+' "$d/Cargo.toml" | grep -v "^$n$" | sort -u | tr '\n' ' '); \
 *     echo "$n -> $deps"; done
 *
 * 用法：`node .layer-guard.mjs`（退出码 0 = 干净，1 = 有新违例/陈旧条目/未登记 crate）；
 *       `node .layer-guard.mjs --json`（同一判据，机器可读输出）；
 *       `node .layer-guard.mjs --self-test`（判据自测：合成 Cargo.toml 文本，
 *       测「新违例报红 / 陈旧条目报红 / 干净通过」三态）。
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { join, resolve } from 'node:path';

const HERE = fileURLToPath(new URL('.', import.meta.url));

/// 分层表：docs/ARCHITECTURE.md §3.1。数字越小越底层，依赖必须**严格向下**。
/// quill-testkit 的层号是补定的事实（理由见头部注释），不是从文档抄的。
export const LAYERS = new Map([
  // L0 基础（叶子）
  ['quill-adapters', 0],
  ['quill-store', 0],
  ['quill-provider', 0],
  // L1 领域
  ['quill-domain', 1],
  // L1 测试夹具（补定，见头注第 3 条理由）
  ['quill-testkit', 1],
  // L2 应用
  ['quill-agent', 2],
  ['quill-control', 2],
  ['quill-wiki', 2],
  ['quill-backup', 2],
  ['quill-upgrade', 2],
  // L3 内核
  ['quill-core', 3],
  // L4 HTTP
  ['quill-server', 4],
  // L5 CLI
  ['quill-cli', 5],
]);

/// 基线：只登记当场核到的既成事实违例，每条附中文理由（见头注）。
/// 匹配语义：`from`/`to` 对上即算「这条违例在基线内」；修好一条必须删一条，
/// 留着不删 = 陈旧条目 = 报红（只减不增）。
export const BASELINE = [
  {
    from: 'quill-upgrade',
    to: 'quill-backup',
    section: 'runtime',
    reason:
      '同层依赖（L2 -> L2）：quill-upgrade 的「升级前强制备份」直接调用 quill-backup 的实现。' +
      '修复方向：改经 L1/L0 接口，或改 docs/ARCHITECTURE.md §3.1 承认 quill-upgrade 在 quill-backup 之上。',
  },
];

/// 依赖段关键字 → 规则类型。`dev` 放宽，其余（含 build-dependencies、
/// target.*.dependencies）都进真实依赖图，按 runtime 严格判。
const DEP_KEYWORDS = new Map([
  ['dependencies', 'runtime'],
  ['dev-dependencies', 'dev'],
  ['build-dependencies', 'runtime'],
]);

/// 把 `[target.'cfg(unix)'.dependencies.quill-store]` 这类段名切开、去掉引号，
/// 找出依赖关键字以及它后面的子表名（若有）。返回 null 表示这不是依赖段。
function depSection(section) {
  const segs = section.split('.').map((s) => s.replace(/^['"]|['"]$/g, ''));
  for (let i = 0; i < segs.length; i += 1) {
    const kind = DEP_KEYWORDS.get(segs[i].toLowerCase());
    if (kind) return { kind, subtable: segs[i + 1] ?? null };
  }
  return null;
}

/// 解析一份 Cargo.toml 文本，返回 `{ name, deps }`：
/// name = `[package] name`；deps = `{ name, kind: 'runtime'|'dev' }[]`（只收 quill-* 键）。
/// 导出成纯函数，是为了 `--self-test` 能拿合成文本直接验它 —— 不跑真文件系统。
export function parseManifestText(text) {
  let name = null;
  const deps = [];
  const seen = new Set();
  let section = null;
  let depCtx = null;

  const pushDep = (depName, kind) => {
    if (!depName || !depName.startsWith('quill-')) return;
    const key = `${kind}:${depName}`;
    if (seen.has(key)) return;
    seen.add(key);
    deps.push({ name: depName, kind });
  };

  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const header = /^\[([^\]]+)\]$/.exec(line);
    if (header) {
      section = header[1];
      depCtx = depSection(section);
      // 写法一：`[dependencies.quill-store]` —— 依赖名在段名里。
      if (depCtx && depCtx.subtable) pushDep(depCtx.subtable, depCtx.kind);
      continue;
    }
    if (!section || !line || line.startsWith('#')) continue;

    const kv = /^([A-Za-z0-9_-]+)\s*=/.exec(line);
    if (!kv) continue;

    if (section === 'package' && kv[1] === 'name') {
      const val = /=\s*"([^"]*)"/.exec(line);
      if (val) name = val[1];
      continue;
    }
    // 写法二：`[dependencies] quill-store = { … }` —— 键名就是依赖名。
    if (depCtx && !depCtx.subtable) pushDep(kv[1], depCtx.kind);
  }
  return { name, deps };
}

/// 判据本体（纯函数）：给定解析后的清单与基线，返回三态结果。
/// `fresh` 与 `stale` 非空、或 `unregistered` 非空 → 门禁报红。
export function evaluate(manifests, baseline = BASELINE) {
  const violations = [];
  const dev = [];
  const unregistered = [];

  for (const m of manifests) {
    const label = m.label ?? m.name ?? '(未命名)';
    const me = LAYERS.get(m.name);
    if (me === undefined) {
      unregistered.push(`${label}：crate 本体不在分层表里，先登记层`);
      continue;
    }
    for (const dep of m.deps) {
      const depLayer = LAYERS.get(dep.name);
      if (depLayer === undefined) {
        unregistered.push(`${label} -> ${dep.name}（${dep.kind}）：依赖不在分层表里，先登记层`);
        continue;
      }
      if (dep.name === m.name) continue; // 自依赖不可能出现，留着防御

      const relation = depLayer < me ? 'down' : depLayer === me ? 'same' : 'up';
      if (dep.kind === 'dev') {
        // 放宽：只如实列出，不判违例。
        dev.push({ from: m.name, fromLayer: me, to: dep.name, toLayer: depLayer, relation });
        continue;
      }
      if (relation !== 'down') {
        violations.push({
          from: m.name,
          fromLayer: me,
          to: dep.name,
          toLayer: depLayer,
          kind: relation, // 'same' | 'up'
          section: dep.kind,
        });
      }
    }
  }

  const matched = new Set();
  const baselined = [];
  const fresh = [];
  for (const v of violations) {
    const i = baseline.findIndex(
      (b, idx) => !matched.has(idx) && b.from === v.from && b.to === v.to && (!b.section || b.section === v.section),
    );
    if (i >= 0) {
      matched.add(i);
      baselined.push({ ...v, reason: baseline[i].reason });
    } else {
      fresh.push(v);
    }
  }
  const stale = baseline.filter((_, i) => !matched.has(i));

  return { violations, fresh, baselined, stale, dev, unregistered, ok: fresh.length === 0 && stale.length === 0 && unregistered.length === 0 };
}

function discoverManifests() {
  const cratesDir = join(HERE, 'crates');
  if (!existsSync(cratesDir)) {
    throw new Error(`找不到 ${cratesDir}（脚本必须放在仓库根）`);
  }
  const out = [];
  for (const dirName of readdirSync(cratesDir)) {
    // 只认 workspace 成员那一层：`crates/<crate>/Cargo.toml`。
    // 不递归 —— `fixtures/` 下曾放着合成的假清单（queue Q101 已删除）。
    const p = join(cratesDir, dirName, 'Cargo.toml');
    if (!existsSync(p)) continue;
    const parsed = parseManifestText(readFileSync(p, 'utf8'));
    out.push({ ...parsed, label: `crates/${dirName}/Cargo.toml` });
  }
  return out;
}

const LY = (n) => `L${n}`;
const REL = { down: '向下', same: '同层', up: '向上' };

function report(result, scanned) {
  const lines = [];
  lines.push('分层守卫：依赖只能向下（docs/ARCHITECTURE.md §3.1）');
  lines.push(`扫了 ${scanned} 个 crate 的 Cargo.toml（只认 workspace 成员那一层）`);

  if (result.dev.length > 0) {
    lines.push('');
    lines.push('[dev-dependencies] 放宽（测试依赖，逐条如实列出，不判红）：');
    for (const d of result.dev) {
      lines.push(`  - ${d.from}(${LY(d.fromLayer)}) --dev--> ${d.to}(${LY(d.toLayer)})：${REL[d.relation]}`);
    }
  }

  if (result.baselined.length > 0) {
    lines.push('');
    lines.push('基线内已知违例（仍存在，不判红；修好一条必须从 BASELINE 删一条，否则判红）：');
    for (const b of result.baselined) {
      lines.push(`  - ${b.from}(${LY(b.fromLayer)}) --> ${b.to}(${LY(b.toLayer)}) [${b.section}]：${b.kind === 'same' ? '同层' : '向上'}依赖`);
      lines.push(`    理由：${b.reason}`);
    }
  }

  if (result.unregistered.length > 0) {
    lines.push('');
    lines.push(`FAIL: ${result.unregistered.length} 个未登记分层的 crate/依赖：`);
    for (const u of result.unregistered) lines.push(`  - ${u}`);
  }
  if (result.fresh.length > 0) {
    lines.push('');
    lines.push(`FAIL: ${result.fresh.length} 处**不在基线内**的新层次违例：`);
    for (const v of result.fresh) {
      lines.push(`  - ${v.from}(${LY(v.fromLayer)}) --> ${v.to}(${LY(v.toLayer)}) [${v.section}]：${REL[v.kind]}依赖，违反「严格向下」`);
    }
    lines.push('  修法二选一：a) 去掉这条依赖/改走下层接口；b) 若确认是既成事实，在 .layer-guard.mjs 的 BASELINE 里登记并附中文理由。');
  }
  if (result.stale.length > 0) {
    lines.push('');
    lines.push(`FAIL: 基线里 ${result.stale.length} 条**已不复现**（陈旧条目）—— 违例已修好却还留在清单里：`);
    for (const s of result.stale) {
      lines.push(`  - ${s.from} --> ${s.to}（基线理由：${s.reason}）`);
    }
    lines.push('  BASELINE 只减不增：确认修复后请删掉对应条目。');
  }

  lines.push('');
  if (result.ok) {
    lines.push('OK: 没有基线外的新违例，基线没有陈旧条目（依赖只能向下，§3.1）。');
  } else {
    lines.push('FAIL: 分层守卫未通过（见上）。');
  }
  return lines.join('\n');
}

function selfTest() {
  let pass = 0;
  let fail = 0;
  const check = (label, cond, detail = '') => {
    if (cond) {
      pass += 1;
      console.log(`  ok   ${label}`);
    } else {
      fail += 1;
      console.log(`  FAIL ${label}${detail ? `：${detail}` : ''}`);
    }
  };
  const M = (label, text) => ({ ...parseManifestText(text), label });

  console.log('自测：合成 Cargo.toml 文本必须被判对（三态：干净通过 / 新违例报红 / 陈旧条目报红）');

  // 合成清单：L0 adapters、L1 domain、L2 wiki，均为干净的下行依赖。
  const adapters = M('a/Cargo.toml', '[package]\nname = "quill-adapters"\n\n[dependencies]\n');
  const domain = M('d/Cargo.toml', '[package]\nname = "quill-domain"\n\n[dependencies]\nquill-adapters = { path = "../quill-adapters" }\n');
  const wiki = M('w/Cargo.toml', '[package]\nname = "quill-wiki"\n\n[dependencies]\nquill-domain = { path = "../quill-domain" }\n');

  // 1. 解析器：crate 名与依赖键都要取到。
  check(
    '解析 [package] name 与 [dependencies] 键',
    domain.name === 'quill-domain' && domain.deps.length === 1 && domain.deps[0].name === 'quill-adapters' && domain.deps[0].kind === 'runtime',
    JSON.stringify(domain),
  );

  // 2. 干净通过。
  const clean = evaluate([adapters, domain, wiki], []);
  check('干净：全向下 → 通过（0 违例）', clean.ok && clean.violations.length === 0 && clean.fresh.length === 0, JSON.stringify(clean.violations));

  // 3. 新违例（向上）报红：L1 依赖 L2。
  const upText = M('u/Cargo.toml', '[package]\nname = "quill-domain"\n\n[dependencies]\nquill-wiki = { path = "../quill-wiki" }\n');
  const up = evaluate([upText, wiki], []);
  check('新违例：L1 -> L2 向上 → 报红', !up.ok && up.fresh.length === 1 && up.fresh[0].kind === 'up', JSON.stringify(up));

  // 4. 同层也算违例（「严格小于」）。
  const sameText = M('s/Cargo.toml', '[package]\nname = "quill-agent"\n\n[dependencies]\nquill-wiki = { path = "../quill-wiki" }\n');
  const same = evaluate([sameText, wiki], []);
  check('新违例：L2 -> L2 同层 → 报红', !same.ok && same.fresh.length === 1 && same.fresh[0].kind === 'same', JSON.stringify(same));

  // 5. 基线内的违例：不报红，且如实列出。
  const baselined = evaluate([upText, wiki], [{ from: 'quill-domain', to: 'quill-wiki', section: 'runtime', reason: '合成理由' }]);
  check('基线内：新违例命中基线 → 通过且列为已知', baselined.ok && baselined.baselined.length === 1 && baselined.fresh.length === 0, JSON.stringify(baselined));

  // 6. 陈旧条目：违例已修好（清单变干净）但基线还留着 → 报红。
  const stale = evaluate([adapters, domain, wiki], [{ from: 'quill-domain', to: 'quill-wiki', section: 'runtime', reason: '合成理由' }]);
  check('陈旧条目：基线有、代码里已不复现 → 报红', !stale.ok && stale.stale.length === 1, JSON.stringify(stale));

  // 7. dev-dependencies 放宽：L2 的 dev 依赖 L3，不算违例，但要列出来。
  const devText = M('v/Cargo.toml', '[package]\nname = "quill-agent"\n\n[dev-dependencies]\nquill-core = { path = "../quill-core" }\n');
  const dev = evaluate([devText], []);
  check('dev 放宽：向上只管列出不判红', dev.ok && dev.violations.length === 0 && dev.dev.length === 1 && dev.dev[0].relation === 'up', JSON.stringify(dev.dev));

  // 8. 未登记分层的 quill-* 依赖 → 报红（不许借「表里没有」绕过）。
  const unknownText = M('n/Cargo.toml', '[package]\nname = "quill-agent"\n\n[dependencies]\nquill-nope = { path = "../quill-nope" }\n');
  const unknown = evaluate([unknownText], []);
  check('未登记 crate → 报红', !unknown.ok && unknown.unregistered.length === 1, JSON.stringify(unknown.unregistered));

  // 9. `[dependencies.quill-store]` 子表写法也要被解析到。
  const subtable = M('t/Cargo.toml', '[package]\nname = "quill-control"\n\n[dependencies.quill-store]\npath = "../quill-store"\n');
  check('解析 [dependencies.<name>] 子表写法', subtable.deps.length === 1 && subtable.deps[0].name === 'quill-store', JSON.stringify(subtable.deps));

  // 10. build-dependencies 等同 runtime 严格判（它是真实依赖图的一部分）。
  const buildDep = M('b/Cargo.toml', '[package]\nname = "quill-domain"\n\n[build-dependencies]\nquill-wiki = { path = "../quill-wiki" }\n');
  const build = evaluate([buildDep, wiki], []);
  check('build-dependencies 按 runtime 判（向上 → 报红）', !build.ok && build.fresh.length === 1, JSON.stringify(build));

  console.log(`\n自测结果：${pass} 通过 / ${fail} 失败`);
  return fail === 0;
}

function main() {
  if (process.argv.includes('--self-test')) {
    process.exit(selfTest() ? 0 : 1);
  }

  const manifests = discoverManifests();
  const result = evaluate(manifests);

  if (process.argv.includes('--json')) {
    const out = {
      ok: result.ok,
      scanned: manifests.length,
      fresh: result.fresh,
      baselined: result.baselined,
      stale: result.stale,
      dev: result.dev,
      unregistered: result.unregistered,
    };
    console.log(JSON.stringify(out, null, 2));
  } else {
    console.log(report(result, manifests.length));
  }
  process.exit(result.ok ? 0 : 1);
}

// 只有直接执行本文件时才跑 main（被 import 时——比如被别的自测工具引用——不跑）。
// 用 pathToFileURL 归一化：Windows 与 WSL 的 argv[1] 写法不同（相对路径、盘符、/mnt/d）。
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main();
}
