#!/usr/bin/env node
/**
 * 公开 API 兼容门禁（Q010）：盯住 `quill-adapters` 的**公开 API 表面**，
 * 删除或签名变化 → 退出码 1；新增 → 允许；`--update` 显式接受变更并重写基线。
 *
 * ## 这道门禁回答什么问题
 *
 * 「这次改动有没有把别人正在依赖的公开 API 改坏 / 删掉？」
 * `quill-adapters` 是「唯一允许跨层的 crate」（描述原文），全仓库都依赖它的
 * id 类型与适配接口。删掉一个 `pub fn`、把 `&str` 改成 `String`、从
 * `pub use` 的再导出里拿掉一个名字 —— `cargo build --workspace` 只有在**碰巧
 * 有调用点**时才会红；没有调用点的公开 API 被删，编译全绿而使用者下次升级才炸。
 * 这类破坏必须由专门的门禁拦：把「当前公开表面」和**提交在案**的基线逐条比对。
 *
 * ## 为什么不用 cargo-semver-checks（如实记录，2026-10-08 本机实测）
 *
 * 本机（WSL）**装过并试过** `cargo-semver-checks v0.51.0`，结论是它在本仓库
 * 的默认模式**不可用**：本仓库所有 crate 都是 `publish = false`（最高指示第 4 条
 * 的打包模型 + deny.toml 的供应链门禁），而 `check-release` 必须到 crates.io
 * 找一个已发布版本当基线，实测直接报错：
 *
 *     error: failed to retrieve index of crate versions from registry
 *     Caused by: quill-adapters not found in registry (crates.io)
 *
 * 它自带的绕过手段 `--baseline-rev <git-rev>` 实测**能跑**（会拿 git 里的旧版本
 * 编译 rustdoc 再比对），但那是「与某个 git 提交比」，**不是**一份可评审、可提交、
 * 每轮 CI 含义相同的契约基线：它要求 CI 有完整 git 历史（fetch-depth: 0）、
 * 每次事件现算基线 ref（PR 取 base sha / push 取 HEAD^），且工具本身要
 * `cargo install` 编译数分钟。本门禁选择了「提交在案的文本基线」这条路线：
 * 基线随代码一起被 review、随 PR 一起被 diff，这正是「契约变更必须显式」的形状。
 * 等 quill 真正发版（或在 CI 里固定一个基线 tag）后，建议把 semver-checks 升级
 * 为主门禁 —— 它做的是**类型级语义比对**，比本脚本强。
 *
 * ## 判据（怎么算、覆盖什么）
 *
 * 扫 `crates/quill-adapters/src/` 下所有 `.rs`（递归子目录），抽取这些公开条目（含签名行）：
 *   `pub fn` / `pub struct` / `pub enum` / `pub trait` / `pub const` / `pub type`
 *   / `pub mod`，外加 `pub use` 再导出（**本脚本的补充**：对 quill-adapters 而言
 *   lib.rs 的再导出就是公开表面本身，漏掉它这道门禁等于睁眼瞎；单层
 *   `pub use path::{A, B as C}` 会逐名展开）。
 * 抽取后按「文件 + 形态 + 名字」归组、签名归一化（多行签名并成一行、空白折叠）
 * 后排序，与 `docs/api-baseline/quill-adapters.txt` 比对：
 *   - 基线有、现在没了 → **删除** → 红；
 *   - 键还在、签名多重集变了 → **签名变化** → 红；
 *   - 现在多出来的 → **新增** → 允许（只在输出里列出，不重写基线也能过）。
 *
 * ## 这个判据**不能**覆盖什么（别把它当 cargo-semver-checks 的等价物）
 *
 * - **宏生成项**：`macro_rules!` 展开、`#[derive]` 生成的方法、`include!` 进来的
 *   代码，文本扫描一律看不见；
 * - **泛型约束的语义等价**：`fn f(x: impl AsRef<str>)` 改成 `fn f<S: AsRef<str>>(x: S)`
 *   是兼容的，本脚本会当成签名变化报红（**宁枉勿纵**：保守报红，用 --update 接受）；
 * - **trait 实现的兼容性**：给 trait 加必需方法、改默认实现、加 supertrait、
 *   sealed trait 的密封性，都只看 trait 声明行，看不懂语义；
 * - **`#[cfg]` / feature 门控**：按文本比对，不按激活的 feature 求值；
 * - **可见性语义**：私有模块里的 `pub` 实际上不可达，本脚本照样收集（偏保守）；
 * - **`pub use` 的复杂形态**：glob（`::*`）、嵌套分组只按整条语句比对；
 * - **字段级变化**：`pub struct` 只记结构体声明行，字段增删/改类型不单独报
 *   （若字段行以 `pub` 开头也不会被当成条目 —— 只认条目关键字）。
 *
 * 一句话：**基线是「已发布契约的近似」，不是类型级真值**。它拦的是最常见、最致命的
 * 那类破坏（删除、改签名），不是全部 semver 规则。
 *
 * ## 用法
 *
 *   node .api-compat-check.mjs            # 检查（CI 只跑这个；红 = 删除/签名变化）
 *   node .api-compat-check.mjs --update   # 显式接受当前表面为新基线（写完要提交）
 *   node .api-compat-check.mjs --self-test  # 判据自测（合成输入，三态：无变化过 /
 *                                           # 删除红 / 签名变化红，另验新增与解析边界）
 */
import { readFileSync, readdirSync, existsSync, writeFileSync, mkdirSync, statSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { join, resolve, relative, sep, dirname } from 'node:path';

const HERE = fileURLToPath(new URL('.', import.meta.url));

/// 本门禁目前只盯 quill-adapters（Q010 要求「至少在这个 crate 上跑起来」）。
/// 推广到别的 crate = 把 CRATE 参数化 + 每 crate 一份基线文件，判据本身不变。
const CRATE = 'quill-adapters';
const BASELINE_PATH = join(HERE, 'docs', 'api-baseline', `${CRATE}.txt`);

/// 认得的条目关键字。`use` 单独处理（要展开再导出）。
const ITEM_KINDS = new Set(['fn', 'struct', 'enum', 'trait', 'type', 'const', 'mod']);

/// 名字归一化：Rust 标识符（允许 raw `r#name`），去掉后面的 `(`/`:`/`,`/`{` 等。
function cleanName(tok) {
  if (!tok) return '';
  const m = /^(r#[A-Za-z_][A-Za-z0-9_]*|[A-Za-z_][A-Za-z0-9_]*)/.exec(tok);
  return m ? m[1] : tok.replace(/[^A-Za-z0-9_#]/g, '');
}

/// 签名归一化：多行并成一行、空白折叠、标点两侧空白去掉。
/// 保证「换行 / 缩进 / rustfmt 重排」不算 API 变化；字符串字面量原样保留
/// （标点折叠不能动字符串里的值，否则 `"a, b"` 与 `"a,b"` 会被判成同一条）。
function collapseSpace(text) {
  return text
    .replace(/\s+/g, ' ')
    .replace(/\s*([()[\]{},;:<>])\s*/g, '$1')
    // 收尾的尾逗号（`,)` / `,}` / `,>`）在 Rust 里与省略等价：
    // rustfmt 在参数换行时会加它，不能因此把「重排」判成签名变化。
    .replace(/,(?=[)\]}>])/g, '')
    .trim();
}

function normalize(sig) {
  const parts = sig.split(/("(?:[^"\\]|\\.)*")/);
  return parts.map((p, i) => (i % 2 === 1 ? p : collapseSpace(p))).join('').trim();
}

function lastSeg(path) {
  const segs = path.split('::');
  return segs[segs.length - 1];
}

/// 一行（可能带修饰符）是否是一个公开条目的开头；是则返回 `{ kind, name }`。
/// `pub(crate)` / `pub(in …)` / `pub(super)` 不算公开 API：`^pub\s` 直接把它们挡掉。
function itemHead(line) {
  if (!/^pub\s/.test(line)) return null;
  const toks = line.split(/\s+/);
  for (let i = 1; i < toks.length; i += 1) {
    const w = toks[i];
    // `pub const fn f` —— const 是修饰符，条目是 fn。
    if (w === 'const' && toks[i + 1] === 'fn') return { kind: 'fn', name: cleanName(toks[i + 2]) };
    if (ITEM_KINDS.has(w)) return { kind: w, name: cleanName(toks[i + 1]) };
    if (w === 'use') return { kind: 'use', name: '' }; // 交给 expandUse 展开
  }
  return null;
}

/// 展开一条归一化后的 `pub use` 语句为若干条目：
/// - `pub use ids::UserId;` → 1 条（name=UserId）；
/// - `pub use ids::{A, B as C};` → 2 条（name=A / C），逐名展开是为了
///   「往再导出组里加名字」算新增（允许）而不是整组签名变化（报红）；
/// - glob / 嵌套分组 / `self` → 整条语句算 1 条（保守，注释里注明局限）。
export function expandUse(normSig) {
  const bodyFull = normSig.replace(/^pub\s+use\s+/, '').replace(/;\s*$/, '').trim();
  const brace = bodyFull.indexOf('{');
  if (brace < 0) {
    const m = /^(.*?)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)$/.exec(bodyFull);
    const name = m ? m[2] : lastSeg(bodyFull);
    return [{ kind: 'use', name, sig: normSig }];
  }
  const prefix = bodyFull.slice(0, brace).replace(/:+$/, '').trim();
  const inner = bodyFull.slice(brace + 1, bodyFull.lastIndexOf('}')).trim();
  if (prefix === '' || inner.includes('{') || inner.includes('*')) {
    return [{ kind: 'use', name: '<整组>', sig: normSig }];
  }
  return inner
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .map((part) => {
      const m = /^(.*?)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)$/.exec(part);
      const name = m ? m[2] : lastSeg(part === 'self' ? prefix : part);
      return { kind: 'use', name, sig: `pub use ${prefix}::${part};` };
    });
}

/// 去掉行尾注释（`//` 在字符串/字符字面量里不算注释）。
function cleanLine(line) {
  let inStr = false;
  let inChar = false;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (inStr) {
      if (ch === '\\') i += 1;
      else if (ch === '"') inStr = false;
      continue;
    }
    if (inChar) {
      if (ch === '\\') i += 1;
      else if (ch === "'") inChar = false;
      continue;
    }
    if (ch === '"') inStr = true;
    else if (ch === "'") {
      // 字符字面量 `'x'` 才进 inChar；生命周期 `'static` 直接跳过引号。
      const rest = line.slice(i + 1);
      if (/^(\\.|[^\\'])'/.test(rest)) inChar = true;
    } else if (ch === '/' && line[i + 1] === '/') {
      return line.slice(0, i);
    }
  }
  return line;
}

/// 括号计数的安全视图：把字符串/字符字面量替换成空格，保留生命周期（它们不含括号）。
function sanitize(s) {
  let out = '';
  for (let i = 0; i < s.length; i += 1) {
    const ch = s[i];
    if (ch === '"') {
      i += 1;
      while (i < s.length && s[i] !== '"') {
        if (s[i] === '\\') i += 1;
        i += 1;
      }
      out += ' ';
    } else if (ch === "'" && /^(\\.|[^\\'])'/.test(s.slice(i + 1))) {
      i += /^(\\.|[^\\'])'/.exec(s.slice(i + 1))[0].length;
      out += ' ';
    } else {
      out += ch;
    }
  }
  return out;
}

/// 累积的签名文本是否已经完整。
/// - 非 `use` 条目：遇到第一个 `{`（函数体/结构体开始）或行尾 `;`（声明结束）即完整；
/// - `use`：等 `;` 且花括号已配平（再导出组跨行）。
/// 用括号深度判断，避免把 `[u8; 16]` 跟着的多行签名、行尾注释里的括号算错。
function isComplete(sanitized, kind) {
  let paren = 0;
  let bracket = 0;
  let brace = 0;
  let seenBrace = false;
  for (const ch of sanitized) {
    if (ch === '(') paren += 1;
    else if (ch === ')') paren -= 1;
    else if (ch === '[') bracket += 1;
    else if (ch === ']') bracket -= 1;
    else if (ch === '{') {
      brace += 1;
      seenBrace = true;
    } else if (ch === '}') brace -= 1;
  }
  const flat = paren <= 0 && bracket <= 0;
  const endsSemi = /;\s*$/.test(sanitized);
  if (kind === 'use') return flat && brace <= 0 && endsSemi;
  return flat && (seenBrace || endsSemi);
}

/// 抽取一份 Rust 源码文本里的公开条目。纯函数，自测直接喂合成文本。
export function extractItems(text) {
  const items = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const t = lines[i].trim();
    if (!t || t.startsWith('//')) continue;
    const head = itemHead(t);
    if (!head) continue;

    // 把可能跨行的签名并成一行：吃到第一个 `{`（函数/结构体开始）或 `;` 为止。
    // 逐行先去掉行尾注释（注释里的括号/斜杠不能影响深度判断）。
    let sig = cleanLine(t);
    let sanitized = sanitize(sig);
    let j = i;
    while (!isComplete(sanitized, head.kind) && j + 1 < lines.length) {
      j += 1;
      const chunk = cleanLine(lines[j].trim());
      sig = `${sig} ${chunk}`;
      sanitized = `${sanitized} ${sanitize(chunk)}`;
    }
    i = j;
    sig = normalize(sig);

    if (head.kind === 'use') {
      items.push(...expandUse(sig));
    } else if (head.name) {
      items.push({ kind: head.kind, name: head.name, sig });
    }
  }
  return items;
}

function walkRs(dir, acc) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walkRs(p, acc);
    else if (name.endsWith('.rs')) acc.push(p);
  }
  return acc;
}

/// 扫真实 crate，返回 `{ file, kind, name, sig }[]`（file 为仓库相对路径，'/' 分隔）。
export function collectCrateItems() {
  const srcDir = join(HERE, 'crates', CRATE, 'src');
  const out = [];
  for (const p of walkRs(srcDir, [])) {
    const file = relative(HERE, p).split(sep).join('/');
    for (const it of extractItems(readFileSync(p, 'utf8'))) out.push({ file, ...it });
  }
  return out;
}

const keyOf = (e) => `${e.file}|${e.kind}|${e.name}`;
const sortByKey = (a, b) => {
  const ka = keyOf(a);
  const kb = keyOf(b);
  return ka < kb ? -1 : ka > kb ? 1 : a.sig < b.sig ? -1 : a.sig > b.sig ? 1 : 0;
};

/// 基线文件行格式：`<文件>\t<形态> <名字>\t<归一化签名>`；`#` 开头是注释。
export function serializeBaseline(entries) {
  return [...entries].sort(sortByKey).map((e) => `${e.file}\t${e.kind} ${e.name}\t${e.sig}`).join('\n');
}

export function parseBaseline(text) {
  const entries = [];
  for (const [i, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const cols = raw.split('\t');
    if (cols.length !== 3 || !cols[0] || !cols[1] || !cols[2]) {
      throw new Error(`基线文件第 ${i + 1} 行格式坏了（应为 3 个 TAB 分隔列）：${JSON.stringify(raw)}`);
    }
    const m = /^([a-z]+) (.+)$/.exec(cols[1]);
    if (!m) throw new Error(`基线文件第 ${i + 1} 行的形态列坏了：${JSON.stringify(cols[1])}`);
    entries.push({ file: cols[0], kind: m[1], name: m[2], sig: cols[2] });
  }
  return entries;
}

/// 判据本体（纯函数）：按「文件+形态+名字」归组比对签名多重集。
/// 返回 `{ removed, changed, added, baselineCount, currentCount }`。
export function diffApi(baselineEntries, currentEntries) {
  const group = (entries) => {
    const m = new Map();
    for (const e of entries) {
      const k = keyOf(e);
      if (!m.has(k)) m.set(k, { key: k, sample: e, sigs: [] });
      m.get(k).sigs.push(e.sig);
    }
    for (const g of m.values()) g.sigs.sort();
    return m;
  };
  const b = group(baselineEntries);
  const c = group(currentEntries);
  const removed = [];
  const changed = [];
  const added = [];
  for (const [k, g] of b) {
    const now = c.get(k);
    if (!now) removed.push(g);
    else if (now.sigs.join('\n') !== g.sigs.join('\n')) changed.push({ key: k, sample: g.sample, before: g.sigs, after: now.sigs });
  }
  for (const [k, g] of c) {
    if (!b.has(k)) added.push(g);
  }
  return { removed, changed, added, baselineCount: baselineEntries.length, currentCount: currentEntries.length };
}

const fmtEntry = (e) => `${e.file}: ${e.kind} ${e.name} ${e.sig}`;

function describe(diff) {
  const lines = [];
  if (diff.removed.length > 0) {
    lines.push(`删除（不兼容）：${diff.removed.length} 条`);
    for (const g of diff.removed) lines.push(`  - ${fmtEntry(g.sample)}`);
  }
  if (diff.changed.length > 0) {
    lines.push(`签名变化/条目数变化（不兼容）：${diff.changed.length} 条`);
    for (const g of diff.changed) {
      lines.push(`  - ${fmtEntry(g.sample)}`);
      lines.push(`      基线：${g.before.join('  ||  ')}`);
      lines.push(`      现在：${g.after.join('  ||  ')}`);
    }
  }
  if (diff.added.length > 0) {
    lines.push(`新增（允许，不判红）：${diff.added.length} 条`);
    for (const g of diff.added) lines.push(`  + ${fmtEntry(g.sample)}`);
  }
  return lines.join('\n');
}

function baselineHeader() {
  return [
    `# ${CRATE} 公开 API 基线`,
    '#',
    '# 这是**已发布契约的近似**，不是类型级真值：由 .api-compat-check.mjs 的文本扫描生成，',
    '# 覆盖边界（宏生成项、泛型约束语义、trait 实现兼容性、cfg 门控、复杂 pub use 等）',
    '# 见脚本头注释「这个判据不能覆盖什么」。',
    '#',
    '# 生成/接受变更：node .api-compat-check.mjs --update（CI 里只跑检查，不跑 --update）',
    '# 判据：删除条目 / 签名变化 → 退出码 1；新增 → 允许。',
    '# 格式：<源文件>\\t<形态> <名字>\\t<归一化签名>',
    '#',
  ].join('\n');
}

function runUpdate() {
  const current = collectCrateItems();
  const old = existsSync(BASELINE_PATH) ? parseBaseline(readFileSync(BASELINE_PATH, 'utf8')) : [];
  const diff = diffApi(old, current);
  mkdirSync(dirname(BASELINE_PATH), { recursive: true });
  writeFileSync(BASELINE_PATH, `${baselineHeader()}\n${serializeBaseline(current)}\n`, 'utf8');
  console.log(`基线已重写：${relative(HERE, BASELINE_PATH).split(sep).join('/')}`);
  console.log(`  条目：${old.length} -> ${current.length}（新增 ${diff.added.length} / 删除 ${diff.removed.length} / 签名变化 ${diff.changed.length}）`);
  if (diff.removed.length + diff.changed.length > 0) {
    console.log('  本次 --update 明确接受了以下不兼容变更：');
    console.log(describe({ ...diff, added: [] }));
  }
  console.log('  记得把基线一起提交 —— 契约变更必须在 review 里可见。');
}

function runCheck() {
  if (!existsSync(BASELINE_PATH)) {
    console.error(`FAIL: 基线不存在：${relative(HERE, BASELINE_PATH).split(sep).join('/')}`);
    console.error('  首次生成：node .api-compat-check.mjs --update（并把基线一起提交）。');
    process.exit(1);
  }
  let baselineEntries;
  try {
    baselineEntries = parseBaseline(readFileSync(BASELINE_PATH, 'utf8'));
  } catch (err) {
    console.error(`FAIL: 基线文件读不了/坏了：${err.message}`);
    console.error('  修法：node .api-compat-check.mjs --update 重新生成并提交。');
    process.exit(1);
  }
  const current = collectCrateItems();
  const diff = diffApi(baselineEntries, current);
  const breaking = diff.removed.length + diff.changed.length;
  if (breaking > 0) {
    console.error(`FAIL: ${CRATE} 的公开 API 有 ${breaking} 处不兼容变更（删除 ${diff.removed.length} / 签名变化 ${diff.changed.length}）：`);
    console.error(describe({ ...diff, added: [] }));
    if (diff.added.length > 0) console.error(describe({ removed: [], changed: [], added: diff.added }));
    console.error('');
    console.error('若这是有意的契约变更：node .api-compat-check.mjs --update，并在 review 里说清为什么。');
    console.error('注意：本门禁是「已发布契约的近似」，不是 cargo-semver-checks 的等价物（局限见脚本头注释）。');
    process.exit(1);
  }
  console.log(`OK: ${CRATE} 公开 API 与基线一致（基线 ${diff.baselineCount} 条 / 当前 ${diff.currentCount} 条，新增 ${diff.added.length} 条）。`);
  if (diff.added.length > 0) console.log(describe({ removed: [], changed: [], added: diff.added }));
  console.log('本门禁是「已发布契约的近似」，不是类型级真值：宏生成项、泛型约束语义、trait 实现兼容性等不在覆盖内。');
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
  const grab = (text) => extractItems(text).map((e) => ({ file: 'src/lib.rs', ...e }));
  const find = (items, kind, name) => items.find((e) => e.kind === kind && e.name === name);

  console.log('自测：合成 Rust 文本必须被判对（无变化过 / 删除红 / 签名变化红 / 新增允许）');

  const base = `pub mod ids;\n\npub use ids::{A, B as Bee, MAX_LEN};\n\npub struct S {\n    pub field: u8,\n}\n\npub enum E { X }\n\npub trait T: Send {\n    fn run(&self);\n}\n\npub type Alias = S;\n\npub const MAX_LEN: usize = 64;\n\npub(crate) fn hidden() {}\n\npub fn go(x: u8) -> u8 {\n    x\n}\n`;
  const items = grab(base);

  // 1. 六类条目 + pub use 展开都要抽到；pub(crate) 与字段不能混进来。
  check(
    '抽取 pub fn/struct/enum/trait/type/const/mod 与 pub use 展开',
    !!find(items, 'mod', 'ids') && !!find(items, 'struct', 'S') && !!find(items, 'enum', 'E') &&
      !!find(items, 'trait', 'T') && !!find(items, 'type', 'Alias') && !!find(items, 'const', 'MAX_LEN') &&
      !!find(items, 'fn', 'go') && !!find(items, 'use', 'A') && !!find(items, 'use', 'Bee'),
    JSON.stringify(items),
  );
  check('pub(crate) 不算公开、pub 字段不算条目', !find(items, 'fn', 'hidden') && !find(items, 'const', 'field'), JSON.stringify(items));

  // 2. 无变化 → 通过（且多行/缩进差异被归一化掉）。
  const reflowed = base.replace('pub fn go(x: u8) -> u8 {\n    x\n}', 'pub fn go(\n    x: u8,\n) -> u8 {\n    x\n}');
  const same = diffApi(items, grab(reflowed));
  check('无变化：换行/缩进重排不算变化 → 通过', same.removed.length === 0 && same.changed.length === 0 && same.added.length === 0, JSON.stringify(same));

  // 3. 删除公开条目 → 红。
  const deleted = diffApi(items, grab(base.replace('pub type Alias = S;\n\n', '')));
  check('删除：pub type 没了 → 判为删除（红）', deleted.removed.length === 1 && deleted.removed[0].sample.name === 'Alias', JSON.stringify(deleted.removed));

  // 4. 签名变化 → 红。
  const changedSig = diffApi(items, grab(base.replace('pub fn go(x: u8) -> u8 {', 'pub fn go(x: u16) -> u8 {')));
  check('签名变化：u8 -> u16 → 判为变化（红）', changedSig.changed.length === 1 && changedSig.changed[0].sample.name === 'go', JSON.stringify(changedSig.changed));

  // 5. 新增 → 允许。
  const addedOnly = diffApi(items, grab(`${base}\npub fn brand_new() -> bool {\n    true\n}\n`));
  check('新增：brand_new → 允许（不在红名单里）', addedOnly.added.length === 1 && addedOnly.removed.length === 0 && addedOnly.changed.length === 0, JSON.stringify(addedOnly));

  // 6. pub use 组里拿掉一个名字 → 判删除（而不是整组失配）。
  const dedupedUse = diffApi(items, grab(base.replace('pub use ids::{A, B as Bee, MAX_LEN};', 'pub use ids::{A, MAX_LEN};')));
  check('pub use 逐名展开：Bee 被拿掉 → 删除 1 条', dedupedUse.removed.length === 1 && dedupedUse.removed[0].sample.name === 'Bee', JSON.stringify(dedupedUse.removed));

  // 7. 同名重复条目（impl 里的同名方法）：删掉其中一个 → 判变化，不误报整体删除。
  const dupA = 'pub struct A;\npub struct B;\n\nimpl A {\n    pub fn parse(s: &str) -> A {}\n}\n\nimpl B {\n    pub fn parse(s: &str) -> B {}\n}\n';
  const dupB = 'pub struct A;\npub struct B;\n\nimpl A {\n    pub fn parse(s: &str) -> A {}\n}\n';
  const dupDiff = diffApi(grab(dupA), grab(dupB));
  check('同名重复方法：少一个 → 签名多重集变化（红），不误报删除', dupDiff.changed.length === 1 && dupDiff.removed.length === 0, JSON.stringify(dupDiff));

  // 8. 基线序列化 / 解析往返一致（含 TAB 格式）。
  const round = parseBaseline(`# 注释\n${serializeBaseline(items)}\n`);
  check('基线序列化往返：分条、签名都一致', serializeBaseline(round) === serializeBaseline(items), JSON.stringify(round));

  // 9. 坏基线要显式报错，不许静默当空基线（否则门禁会假绿）。
  let threw = false;
  try {
    parseBaseline('crates/x/src/lib.rs pub fn go\tpub fn go();\n');
  } catch {
    threw = true;
  }
  check('坏基线 → 抛错（不静默假绿）', threw);

  // 10. 注释与字符串诱饵：注释里的假条目不能进来。
  const decoy = grab('// pub fn fake() {}\n/// pub struct Doc;\npub fn real() {}\n');
  check('注释里的 pub 条目不算（只有 real）', decoy.length === 1 && decoy[0].name === 'real', JSON.stringify(decoy));

  console.log(`\n自测结果：${pass} 通过 / ${fail} 失败`);
  return fail === 0;
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes('--self-test')) {
    process.exit(selfTest() ? 0 : 1);
  }
  if (args.includes('--update')) {
    runUpdate();
    return;
  }
  runCheck();
}

// 只有直接执行本文件时才跑 main（被 import 时不跑）。
if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main();
}
