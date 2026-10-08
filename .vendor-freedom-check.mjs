#!/usr/bin/env node
/**
 * 最高指示第 4 条的门禁：**最终产物只打包 quill**。
 *
 * ## 这条指示的原话
 *
 * > 最终产物只打包 quill 一个项目。goose 与 octop 是**参考源**，允许把它们的代码
 * > 抄进来，但不许把整个项目当依赖加进 `Cargo.toml`。……删掉 `vendor/goose` 与
 * > `.octop-ref/octop` 后 `cargo build` 必须照常成功。
 *
 * ## 为什么要有这道门禁
 *
 * 「抄不依赖」只有靠门禁才守得住。一旦有人图省事写下一行
 * `goose = { path = "../vendor/goose/crates/goose" }`，`cargo build` 会照常通过、
 * 测试会照常变绿 —— 而**打包产物里就多了一整个上游项目**，直到某天发布才发现。
 * 那种错误在 review 里几乎看不出来（一行 TOML），必须靠机器拦。
 *
 * ## 判据（扫哪些、怎么判）
 *
 * 扫仓库里所有 `Cargo.toml`（跳过 `vendor/`、`.octop-ref/`、`target/`、
 * `node_modules/`、`.git/` —— 参考源自己的清单不归我们管），任一条命中即报红：
 *
 * 1. **依赖键**以 `goose` / `octop` 开头（如 `goose = …`、`goose-providers = …`）。
 * 2. 任何 `path = "…"` 指向 `vendor/` 或 `.octop-ref/`。
 * 3. 任何 `git = "…"` 指向 goose / octop 的上游仓库。
 *
 * 复现：`node .vendor-freedom-check.mjs`；自测：`--self-test`。
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join, relative, sep } from 'node:path';

const HERE = fileURLToPath(new URL('.', import.meta.url));

/// 这些目录里的 Cargo.toml 不是我们的：
/// - `vendor/`、`.octop-ref/` 是参考源自己的清单；`target/`、`node_modules/`、`.git/` 是产物；
/// - `fixtures/` 是**合成的测试数据**（喂给检测器的假清单），不是真依赖声明。
const SKIP_DIRS = new Set(['vendor', '.octop-ref', 'target', 'node_modules', '.git', 'fixtures']);

/// 依赖键开头命中这些即违规。
const FORBIDDEN_KEYS = [/^goose(-|$)/i, /^octop(-|$)/i];

/// 路径 / git 地址里命中这些即违规（指向参考源或上游仓库）。
const FORBIDDEN_TARGETS = [
  /(^|\/)vendor\//,
  /(^|\/)\.octop-ref\//,
  /github\.com\/[^"']*\/goose/i,
  /github\.com\/[^"']*\/octop/i,
];

/// 扫一份 Cargo.toml 文本，返回违规描述（空数组 = 干净）。
/// 导出成纯函数，是为了 `--self-test` 能拿合成输入直接验它 —— 不跑真文件系统。
export function scanToml(relPath, text) {
  const out = [];
  for (const [i, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;

    // 形如 `name = …` 或 `name = { … }` 的依赖键。
    const m = /^([A-Za-z0-9_-]+)\s*=/.exec(line);
    if (m && FORBIDDEN_KEYS.some((re) => re.test(m[1]))) {
      out.push(`${relPath}:${i + 1} 依赖键「${m[1]}」不许用（最高指示第 4 条：只打包 quill）`);
      continue;
    }

    // `path = "…"` / `git = "…"` 指向参考源。
    for (const t of FORBIDDEN_TARGETS) {
      if (t.test(line)) {
        out.push(`${relPath}:${i + 1} 指向参考源/上游：${line}`);
        break;
      }
    }
  }
  return out;
}

function walk(dir, acc) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) walk(p, acc);
    else if (name === 'Cargo.toml') acc.push(p);
  }
  return acc;
}

function selfTest() {
  let pass = 0;
  let fail = 0;
  const check = (label, got, wantHit) => {
    const hit = got.length > 0;
    if (hit === wantHit) {
      pass += 1;
      console.log(`  ok   ${label}`);
    } else {
      fail += 1;
      console.log(`  FAIL ${label}：期望${wantHit ? '命中' : '不命中'}，实际 ${JSON.stringify(got)}`);
    }
  };

  console.log('自测：合成输入必须被判对（含反向验证）');
  check('干净清单', scanToml('x/Cargo.toml', '[dependencies]\nserde = "1"\naxum = "0.7"\n'), false);
  check('直接依赖 goose', scanToml('x/Cargo.toml', 'goose = { path = "../v" }\n'), true);
  check('依赖键带后缀', scanToml('x/Cargo.toml', 'goose-providers = "1"\n'), true);
  check('path 指进 vendor', scanToml('x/Cargo.toml', 'foo = { path = "../vendor/goose/crates/goose" }\n'), true);
  check('path 指进 .octop-ref', scanToml('x/Cargo.toml', 'foo = { path = "../.octop-ref/octop" }\n'), true);
  check('git 指向上游', scanToml('x/Cargo.toml', 'foo = { git = "https://github.com/aaif-goose/goose" }\n'), true);
  check('注释里的 goose 不算', scanToml('x/Cargo.toml', '# goose 是参考源，不是依赖\nserde = "1"\n'), false);
  check('goose 作为普通值不算', scanToml('x/Cargo.toml', 'description = "抄 goose 的口径"\n'), false);

  console.log(`\n自测结果：${pass} 通过 / ${fail} 失败`);
  return fail === 0;
}

function main() {
  if (process.argv.includes('--self-test')) {
    process.exit(selfTest() ? 0 : 1);
  }
  const files = walk(HERE, []);
  const violations = [];
  for (const f of files) {
    violations.push(...scanToml(relative(HERE, f).split(sep).join('/'), readFileSync(f, 'utf8')));
  }
  if (violations.length > 0) {
    console.error(`FAIL: 有 ${violations.length} 处违反「只打包 quill」（最高指示第 4 条）：`);
    for (const v of violations) console.error(`  - ${v}`);
    console.error('\n参考源只能**抄代码**，不能加成依赖。');
    process.exit(1);
  }
  console.log(`OK: 扫了 ${files.length} 个 Cargo.toml，没有把 goose / octop 当依赖（最高指示第 4 条）`);
}

main();
