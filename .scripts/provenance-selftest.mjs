// 出处判定的自测：拿合成的引用与合成的索引喂 `.provenance-check.mjs`，
// 确认四种结论都对。
//
// **为什么需要它**：这个门禁的全部价值是「引用烂了它会红」。判据写错成
// 「永远通过」时，它在真实仓库上跑出来的输出和「一切正常」一模一样 ——
// 而真实仓库此刻恰好是全绿的（`UPSTREAM-USAGE.md` 里 130 条引用都对），
// 所以一道恒真的检查在这里**不可能靠真实运行暴露**。只能靠合成输入钉死判定。
// 这与 `.upstream-check.mjs` 的自测、`gate-selftest.sh` 是同一个道理
// （见 `MILESTONES.md` 的 M0：判定逻辑写错成恒真，不会自己暴露）。
//
// **不碰真实索引，也不依赖仓库外的文件**：所有输入都在这里现写。行数也现造
// （ctx.lineCount 返回写死的数），端到端那几个场景另有一个临时根、被引用的
// 上游文件在那个根里现造。所以这个自测在「本机跑没跑过 fetch-vendor.sh」
// 「sparse 检出在不在手边」两种情况下都该是同一个结果 ——
// 2026-10-08 第一次真跑 CI 就红在这儿：它原来引的是 gitignore 的
// `vendor/goose/...`，本机有、CI 没有，于是本机一直绿、CI 一直红。
//
// 跑法：node .scripts/provenance-selftest.mjs
import {
  looksLikeRef,
  parseRef,
  classifyRef,
  decideProvenance,
  isUpstreamPath,
  isOctopPath,
  inSparseSet,
  extractRefs,
} from '../.provenance-check.mjs';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

let fail = 0;

function expect(desc, cond, detail = '') {
  if (cond) {
    console.log(`  ✓ ${desc}`);
  } else {
    console.log(`  ✗ ${desc}${detail ? `\n      ${detail}` : ''}`);
    fail = 1;
  }
}

const REPO = fileURLToPath(new URL('..', import.meta.url));

// —— 合成世界 ——
// 存在的文件与它们各自的行数。**故意造得很少**，这样「行号越界」能被稳定触发，
// 不依赖任何真实文件将来会不会变长。
const LINE_COUNTS = {
  '.octop-ref/octop/src/octop/infra/agents/experts/catalog.py': 961,
  '.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCard.tsx': 460,
  '.octop-ref/octop/dashboard/src/pages/Experts/index.tsx': 917,
  'vendor/goose/crates/goose/src/agents/subagent_handler.rs': 900,
  'crates/quill-server/src/api_chat.rs': 1649,
  'WORKING.md': 98,
  '.library-check.mjs': 238,
};
// 仓库根上真实存在的裸文件名。bare name 只有落在这里才允许通过。
const ROOT_FILES = new Set(['WORKING.md', '.library-check.mjs']);
const SPARSE = ['dashboard/src/pages/Experts', 'src/octop/infra/agents/experts'];

const ctx = {
  lineCount: (p) => (p in LINE_COUNTS ? LINE_COUNTS[p] : null),
  inSparse: (p) => inSparseSet(p, SPARSE),
  rootHit: (p) => ROOT_FILES.has(p),
  // 默认「vendor 树都在位」，所以下面场景6 那条「树在、文件没了 → broken」保持成立。
  treePresent: () => true,
};

const st = (raw) => classifyRef(raw, ctx).status;

console.log('场景1：引用长什么样才算引用');
{
  expect('带行号的完整路径算引用', looksLikeRef('vendor/goose/crates/goose/src/agents/subagent_handler.rs:46'));
  expect('行号区间也算引用', looksLikeRef('.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:852-853'));
  expect('没行号的路径算引用（我们的落点允许这么写）', looksLikeRef('ui/web/src/charts/echarts.ts'));
  expect('裸文件名带行号也算引用 —— 它正是要被抓的那一类', looksLikeRef('skillhub_market.py:610'));
  expect('状态词不算引用', !looksLikeRef('已接入'));
  expect('null 不算引用', !looksLikeRef('null'));
  expect('散文碎片 m..s 不算引用', !looksLikeRef('m..s'));
  expect('裸扩展名 .md 不算引用', !looksLikeRef('.md'));
  expect('带空格与命令前缀的不算引用', !looksLikeRef('node .provenance-check.mjs'));
  expect('带花括号路径模板的不算引用', !looksLikeRef('skills/{slug}/SKILL.md'));
  expect('glob 不算引用', !looksLikeRef('dashboard/src/pages/Control/**'));
}

console.log('\n场景2：引用的拆分');
{
  const r = parseRef('.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:852-853');
  expect('路径与区间拆对', r.path === '.octop-ref/octop/src/octop/infra/agents/experts/catalog.py' && r.from === 852 && r.to === 853, JSON.stringify(r));
  const single = parseRef('WORKING.md:98');
  expect('单行号的 to 等于 from', single.from === 98 && single.to === 98);
  const noLine = parseRef('ui/web/src/charts/echarts.ts');
  expect('没行号不是格式错（from 为 null、malformed 为 false）', noLine.from === null && noLine.malformed === false, JSON.stringify(noLine));
  const flipped = parseRef('WORKING.md:98-20');
  expect('区间反了判格式错', flipped.malformed === true, JSON.stringify(flipped));
}

console.log('\n场景3：路径归属与 sparse 判定');
{
  expect('vendor/ 前缀算上游', isUpstreamPath('vendor/goose/crates/goose/src/agents/subagent_handler.rs'));
  expect('.octop-ref/ 前缀算上游', isUpstreamPath('.octop-ref/octop/dashboard/src/pages/Experts/index.tsx'));
  expect('octop 相对路径也算上游', isUpstreamPath('dashboard/src/pages/Experts/index.tsx'));
  expect('本仓库 crates/ 不算上游', !isUpstreamPath('crates/quill-server/src/api_chat.rs'));
  expect('octop 相对路径是 octop 路径', isOctopPath('src/octop/infra/skills/skillhub_market.py'));
  expect('goose 路径不是 octop 路径（所以 sparse 管不到它）', !isOctopPath('vendor/goose/crates/goose/src/agents/subagent_handler.rs'));
  expect('sparse 目录本身在内', inSparseSet('.octop-ref/octop/dashboard/src/pages/Experts/index.tsx', SPARSE));
  expect('sparse 目录的子目录在内', inSparseSet('src/octop/infra/agents/experts/catalog.py', SPARSE));
  expect('sparse 之外不在内', !inSparseSet('src/octop/infra/skills/skillhub_market.py', SPARSE));
  expect('sparse 之外不在内（dashboard 侧）', !inSparseSet('dashboard/src/pages/Agent/Skills/components/SkillHubTab.tsx', SPARSE));
  expect('cone 模式带出来的祖先紧邻文件算在内', inSparseSet('dashboard/package.json', ['dashboard/src/pages/Experts']));
}

console.log('\n场景4：好引用必须判通过');
{
  expect('行号在范围内 → ok', st('.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:857') === 'ok');
  expect('区间的末尾正好等于行数 → ok', st('.octop-ref/octop/dashboard/src/pages/Experts/index.tsx:917') === 'ok');
  expect('goose 引用 → ok', st('vendor/goose/crates/goose/src/agents/subagent_handler.rs:46') === 'ok');
  expect('本仓库引用 → ok', st('crates/quill-server/src/api_chat.rs:830-838') === 'ok');
  expect('没行号但文件在 → ok', st('WORKING.md') === 'ok');
  expect('仓库根上的文件带行号也放行（位置唯一，钉得住）', st('WORKING.md:98') === 'ok');
  expect('仓库根上的点文件带行号也放行', st('.library-check.mjs:61-79') === 'ok');
}

console.log('\n场景5：行号越界必须判失败（这是最容易被写成「不判」的一类）');
{
  const r = classifyRef('.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:5000', ctx);
  expect('越界 → broken', r.status === 'broken', JSON.stringify(r));
  expect('理由里说出了真实行数', /只有 961 行/.test(r.reason), r.reason);
  const edge = classifyRef('WORKING.md:99', ctx);
  expect('超出 1 行也算越界（不是 >= 号）', edge.status === 'broken', JSON.stringify(edge));
  const last = classifyRef('WORKING.md:98', ctx);
  expect('正好等于行数不算越界', last.status === 'ok', JSON.stringify(last));
}

console.log('\n场景6：文件读不到 —— sparse 内外要分清');
{
  const off = classifyRef('src/octop/infra/skills/skillhub_market.py:610', ctx);
  expect('sparse 之外的 octop 路径 → offline-unverifiable', off.status === 'offline-unverifiable', JSON.stringify(off));
  expect('unverifiable 的理由说清了「不是引用腐烂」', /不是引用腐烂/.test(off.reason), off.reason);
  const inside = classifyRef('.octop-ref/octop/dashboard/src/pages/Experts/components/TeamCardExtra.tsx:10', ctx);
  expect('sparse 之内却读不到 → broken（这才是真的烂了）', inside.status === 'broken', JSON.stringify(inside));
  const ours = classifyRef('crates/quill-server/src/does_not_exist.rs:1', ctx);
  expect('本仓库文件读不到 → ours-missing', ours.status === 'ours-missing', JSON.stringify(ours));
  const goose = classifyRef('vendor/goose/crates/goose/src/gone.rs:1', ctx);
  expect('goose 侧读不到 → broken（vendor/goose 是全量拷贝，没有 sparse 这回事）', goose.status === 'broken', JSON.stringify(goose));
}

console.log('\n场景6b：vendor 整棵树没取回来 —— 是「没核」，不是「引用坏了」');
{
  // 2026-10-08：CI 里 `vendor/openoctopus-frontend` 整个不存在（取码脚本压根不取它），
  // 而判定把它报成「坏掉的引用 —— 文件读不到」。那是把「没跑」说成「测了而且坏了」。
  const noTree = { ...ctx, treePresent: () => false };
  const r = classifyRef('vendor/openoctopus-frontend/src/index.css:70', noTree);
  expect('整棵树没取回来 → offline-unverifiable', r.status === 'offline-unverifiable', JSON.stringify(r));
  expect('理由说清了怎么取', /fetch-vendor\.sh/.test(r.reason), r.reason);
  // 反面：树在、就这个文件没了 → 仍然是 broken，不能被这条规则放过
  const stillBroken = classifyRef('vendor/goose/crates/goose/src/gone.rs:1', ctx);
  expect('树在位时仍然是 broken（没被放宽）', stillBroken.status === 'broken', JSON.stringify(stillBroken));
  // 没有 treePresent 的旧调用方也不能因此崩掉或乱判
  const noFn = classifyRef('vendor/goose/crates/goose/src/gone.rs:1', {
    lineCount: ctx.lineCount,
    inSparse: ctx.inSparse,
    rootHit: ctx.rootHit,
  });
  expect('老调用方（没有 treePresent）仍然判 broken', noFn.status === 'broken', JSON.stringify(noFn));
}

console.log('\n场景7：格式错必须判失败（2026-10-06 那次引用腐烂的形状）');
{
  const bare = classifyRef('skillhub_market.py:610', ctx);
  expect('裸文件名带行号 → broken', bare.status === 'broken', JSON.stringify(bare));
  expect('理由点明「定位不到是哪一份」', /定位不到是哪一份/.test(bare.reason), bare.reason);
  const bareNoLine = classifyRef('skillhub_market.py', ctx);
  expect('裸文件名连行号都没有也 → broken', bareNoLine.status === 'broken', JSON.stringify(bareNoLine));
  expect('区间反了 → broken', classifyRef('WORKING.md:98-20', ctx).status === 'broken');
  expect('行号不是数字 → broken', classifyRef('WORKING.md:abc', ctx).status === 'broken');
  expect('行号带字母 → broken', classifyRef('WORKING.md:9a', ctx).status === 'broken');
}

console.log('\n场景8：汇总口径 —— 「核过」与「没核」不许混成一句话');
{
  const r = decideProvenance([
    { status: 'ok' },
    { status: 'ok' },
    { status: 'offline-unverifiable' },
    { status: 'broken' },
    { status: 'ours-missing' },
  ]);
  expect('ok 记 2', r.counts.ok === 2, JSON.stringify(r.counts));
  expect('unverifiable 记 1', r.counts['offline-unverifiable'] === 1);
  expect('broken + ours-missing 一起算「坏了」', r.broken.length === 2, JSON.stringify(r.broken.length));
  expect('unverifiable **不进** broken', !r.broken.some((x) => x.status === 'offline-unverifiable'));
  const clean = decideProvenance([{ status: 'ok' }, { status: 'offline-unverifiable' }]);
  expect('只有 unverifiable 时 broken 为空（门禁不该因本地检出而红）', clean.broken.length === 0);
}

console.log('\n场景9：从 markdown 里真的把引用抠出来');
{
  const md = [
    '| 机制 | 我们的落点 | 上游出处 | 状态 | 差别 |',
    '|---|---|---|---|---|',
    '| A | `crates/quill-server/src/api_chat.rs:830` | `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` | 已接入 | 无差别 |',
    '| B | `ui/web/src/experts/library.ts:9-11` | `src/octop/infra/skills/skillhub_market.py:610` | 我们的选择 | 见下 |',
    '',
    '散文里也有：`WORKING.md:44` 写着规矩；`已接入` 不是引用；`m..s` 也不是。',
  ].join('\n');
  const refs = extractRefs(md);
  expect('抠出 5 条（4 条表格 + 1 条散文）', refs.length === 5, JSON.stringify(refs));
  expect('包含本仓库引用', refs.includes('crates/quill-server/src/api_chat.rs:830'));
  expect('包含 goose 引用', refs.includes('vendor/goose/crates/goose/src/agents/subagent_handler.rs:46'));
  expect('包含 sparse 外的 octop 引用', refs.includes('src/octop/infra/skills/skillhub_market.py:610'));
  expect('散文里的引用也算', refs.includes('WORKING.md:44'));
  expect('状态词没被当成引用', !refs.includes('已接入'));
  expect('散文碎片没被当成引用', !refs.includes('m..s'));
}

console.log('\n场景10：端到端 —— 拿真脚本跑合成索引，证明退出码真的会变');
{
  const dir = mkdtempSync(join(tmpdir(), 'quill-prov-selftest-'));
  const head = ['| 机制 | 我们的落点 | 上游出处 | 状态 | 差别 |', '|---|---|---|---|---|'];

  // **在一个临时根里自己造出被引用的那些文件**，而不是用仓库里的真文件。
  //
  // 原因：这个场景的价值在于「真脚本读到真文件时判 ok」。可它原来引用的
  // `vendor/goose/...` 是 gitignore 的、只有跑过 fetch-vendor.sh 才存在 ——
  // 于是本机一直绿、CI 一直红（2026-10-08 第一次真跑 CI 就红在这儿）。
  // 一个自测依赖仓库外的文件，它测的其实是「那台机器跑没跑过取码脚本」。
  const root = join(dir, 'root');
  const seed = (rel, lines) => {
    const abs = join(root, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, Array.from({ length: lines }, (_, i) => `第 ${i + 1} 行`).join('\n'), 'utf8');
  };
  seed('crates/quill-server/src/api_chat.rs', 1649);
  seed('ui/web/src/experts/library.ts', 40);
  seed('vendor/goose/crates/goose/src/agents/subagent_handler.rs', 900);
  seed('.octop-ref/octop/dashboard/src/pages/Experts/index.tsx', 917);

  const run = (name, rows) => {
    const p = join(dir, name);
    writeFileSync(p, [...head, ...rows].join('\n'), 'utf8');
    try {
      const out = execFileSync(
        process.execPath,
        ['.provenance-check.mjs', '--root', root, '--index', p],
        { cwd: REPO, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
      );
      return { code: 0, out: String(out) };
    } catch (e) {
      return { code: e.status ?? -1, out: `${e.stdout ?? ''}${e.stderr ?? ''}` };
    }
  };

  const good = run(
    'good.md',
    [
      '| 好的一条 | `crates/quill-server/src/api_chat.rs:830-838` | `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` | 已接入 | 无差别 |',
      '| sparse 外的一条 | `ui/web/src/experts/library.ts:9-11` | `src/octop/infra/skills/skillhub_market.py:610` | 我们的选择 | 核不到 |',
    ],
  );
  expect('全好 + 一条 sparse 外 → 退出 0', good.code === 0, `code=${good.code}\n${good.out}`);
  expect('输出里明说 sparse 外那条**没能核**', /没能核/.test(good.out), `code=${good.code}\n${good.out}`);
  expect('输出里没有把它算成「核过」', !/没有一条坏的.*核过 sparse/.test(good.out));

  const oor = run(
    'oor.md',
    ['| 行号越界 | `crates/quill-server/src/api_chat.rs:99999` | `vendor/goose/crates/goose/src/agents/subagent_handler.rs:46` | 已接入 | 无差别 |'],
  );
  expect('行号越界 → 退出非 0', oor.code !== 0, `code=${oor.code}\n${oor.out}`);
  expect('输出点名了越界这件事', /行号越界/.test(oor.out), oor.out);

  const bare = run(
    'bare.md',
    ['| 裸文件名 | `crates/quill-server/src/api_chat.rs:830` | `skillhub_market.py:610` | 我们的选择 | 腐烂的那一类 |'],
  );
  expect('裸文件名带行号 → 退出非 0', bare.code !== 0, `code=${bare.code}\n${bare.out}`);
  expect('输出说清是定位不到', /定位不到/.test(bare.out), bare.out);

  const gone = run(
    'gone.md',
    ['| 上游文件不在了 | `crates/quill-server/src/api_chat.rs:830` | `.octop-ref/octop/dashboard/src/pages/Experts/index.tsx:99999` | 已接入 | 行号对不上 |'],
  );
  expect('sparse 内的上游文件行号越界 → 退出非 0（不是 unverifiable）', gone.code !== 0, `code=${gone.code}\n${gone.out}`);

  const noHeader = join(dir, 'noheader.md');
  writeFileSync(noHeader, '| 机制 | 落点 |\n|---|---|\n| A | `WORKING.md:1` |\n', 'utf8');
  let noHeaderCode = 0;
  let noHeaderOut = '';
  try {
    execFileSync(process.execPath, ['.provenance-check.mjs', '--root', root, '--index', noHeader], { cwd: REPO, encoding: 'utf8' });
  } catch (e) {
    noHeaderCode = e.status ?? -1;
    noHeaderOut = `${e.stdout ?? ''}${e.stderr ?? ''}`;
  }
  expect('表头被改坏 → 退出 2（不是 0）', noHeaderCode === 2, `code=${noHeaderCode}\n${noHeaderOut}`);

  rmSync(dir, { recursive: true, force: true });
}

console.log('\n场景11：反向钉死 —— 「永远通过」的实现必须被本自测抓住');
{
  // 一个恒真的判定：不管输入是什么都返回 ok。
  const alwaysOk = () => ({ status: 'ok' });
  const brokenInputs = [
    'skillhub_market.py:610',
    '.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:5000',
    'crates/quill-server/src/does_not_exist.rs:1',
  ];
  // 恒真实现在这三条上会说 ok，于是场景4-7 里「坏输入必须判非 ok」的三条断言全红 ——
  // 也就是说，自测**看得见**这个 bug。这就是要钉的那件事。
  const wouldBeCaught = brokenInputs.every((r) => alwaysOk(r).status === 'ok' && st(r) !== 'ok');
  expect('恒真实现会被场景4-7 的断言判为「不通过」', wouldBeCaught, JSON.stringify(brokenInputs.map(st)));
  expect('真实现对同三条输入全部判非 ok', brokenInputs.every((r) => st(r) !== 'ok'), brokenInputs.map(st).join(' | '));
  expect('真实现对好输入仍然判 ok（不是靠「全判坏」来通过自测）', st('WORKING.md:98') === 'ok' && st('vendor/goose/crates/goose/src/agents/subagent_handler.rs:46') === 'ok');
}

console.log('');
if (fail === 0) {
  console.log('出处判定自测：十一个场景全对');
  process.exit(0);
}
console.log('出处判定自测：有场景判错了 —— 判定逻辑不可信，它现在给的 OK 是假的，先修它再谈出处对齐');
process.exit(1);
