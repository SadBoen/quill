// 上游判定的自测：拿合成观测喂 `.upstream-check.mjs` 的 decideUpstream，
// 确认「落后判失败」「一致判通过」「查不到也判失败」三种结论都对。
//
// **为什么需要它**：这个脚本原来打印完「上游最新 = vX」就退出 0 ——
// 上游走了我们没跟，它照样绿。那是「一个不可能失败的检查」，和 gate-selftest.sh
// 里记的那道 awk 门禁同一类错（见 MILESTONES.md 的 M0）。这种错只有用合成
// 输入钉住判定才抓得住：真实网络时好时坏，不能拿来当判据。
//
// **不联网**：全部输入都是这里写死的。跑得起来与否只看判定本身对不对。
//
// 跑法：node .scripts/upstream-check-selftest.mjs
import { compareSemver, decideUpstream, parseUpstreamDoc, samePath, parseInventory } from '../.upstream-check.mjs';

let fail = 0;

function expect(desc, cond, detail = '') {
  if (cond) {
    console.log(`  ✓ ${desc}`);
  } else {
    console.log(`  ✗ ${desc}${detail ? `\n      ${detail}` : ''}`);
    fail = 1;
  }
}

// 合成观测。形状必须与 observeGoose / observeOctop 的返回值一致，
// 否则测的是「读不到记录」那条分支，不是要测的那条。
const GOOSE_REC = '1.53.0';
const OCTOP_PIN = 'eb28011249c02cafd389b2d424294c6c1b9cf422';
const OCTOP_HEAD = '1111111111111111111111111111111111111111';
const INV = {
  source: 'UPSTREAM-USAGE.md',
  goose: ['crates/goose/src/agents/subagent_handler.rs'],
  octop: ['dashboard/src/pages/Experts/index.tsx'],
};
const gOk = () => ({ recorded: GOOSE_REC, upstream: { tag: GOOSE_REC, publishedAt: '2026-10-02' } });
const oOk = () => ({ pinned: OCTOP_PIN, upstream: { sha: OCTOP_PIN, date: '2026-10-05', aheadBy: 0, files: [] } });

console.log('场景1：上游与记录一致 —— 应当判通过');
{
  const r = decideUpstream({ goose: gOk(), octop: oOk(), inventory: INV });
  expect('没有 problems', r.problems.length === 0, r.problems.join(' | '));
  expect('没有一项被标成 behind', !r.behind.goose && !r.behind.octop);
  expect('没有任何一项被标成查不到', !r.unknown.goose && !r.unknown.octop);
  expect('两行都在说「一致」', r.lines.length === 2 && r.lines.every((l) => l.includes('一致')), r.lines.join(' | '));
}

console.log('场景2：goose 上游出了新 release —— 应当判失败，并说清落后几个');
{
  const goose = {
    recorded: GOOSE_REC,
    upstream: {
      tag: '1.56.0',
      publishedAt: '2026-10-20',
      newer: [
        { tag: '1.56.0', publishedAt: '2026-10-20' },
        { tag: '1.55.0', publishedAt: '2026-10-15' },
        { tag: '1.54.0', publishedAt: '2026-10-10' },
      ],
    },
    diff: { files: [{ filename: 'crates/goose/src/agents/subagent_handler.rs' }, { filename: 'README.md' }] },
  };
  const r = decideUpstream({ goose, octop: oOk(), inventory: INV });
  expect('有一条 problem（退出码 1 的依据）', r.problems.length === 1, JSON.stringify(r.problems));
  expect('problem 说了落后几个 release', r.problems[0].includes('3 个 release'), r.problems[0]);
  expect('problem 同时给了记录值与上游值', r.problems[0].includes(GOOSE_REC) && r.problems[0].includes('1.56.0'), r.problems[0]);
  expect('behind.goose 为真', r.behind.goose === true);
  expect('octop 没被牵连', r.behind.octop === false);
  expect('给出了 release notes 位置', r.lines.some((l) => l.includes('github.com/aaif-goose/goose/releases')));
  expect('点出了被改动的依赖文件', r.lines.some((l) => l.includes('subagent_handler.rs')), r.lines.join(' | '));
}

console.log('场景3：octop 的 main 走了 —— 应当判失败，并说出落后几个 commit');
{
  const octop = {
    pinned: OCTOP_PIN,
    upstream: {
      sha: OCTOP_HEAD,
      date: '2026-10-25',
      aheadBy: 4,
      files: [{ filename: 'dashboard/src/pages/Experts/index.tsx' }, { filename: 'docs/readme.md' }],
    },
  };
  const r = decideUpstream({ goose: gOk(), octop, inventory: INV });
  expect('有一条 problem', r.problems.length === 1, JSON.stringify(r.problems));
  expect('problem 说了落后 4 个 commit', r.problems[0].includes('4 个 commit'), r.problems[0]);
  expect('problem 给了 compare 链接', r.lines.some((l) => l.includes(`/compare/${OCTOP_PIN}...main`)));
  expect('goose 没被牵连', r.behind.goose === false);
  expect('behind.octop 为真', r.behind.octop === true);
  expect('点出了被改动的依赖文件', r.lines.some((l) => l.includes('Experts/index.tsx')), r.lines.join(' | '));
}

console.log('场景4：查不到上游 —— 同样判失败，且不能出现「已是最新」这种话');
{
  // key = 查不到的那一方；只钉它不能自称一致，另一方正常报一致是对的。
  const cases = [
    ['goose 联网失败', 'goose', { goose: { recorded: GOOSE_REC, upstream: { error: 'ETIMEDOUT' } }, octop: oOk(), inventory: INV }],
    ['octop 限流', 'octop', { goose: gOk(), octop: { pinned: OCTOP_PIN, upstream: { error: 'GitHub 限流（HTTP 403）' } }, inventory: INV }],
    ['octop compare 挂了但知道 main 已走', 'octop', {
      goose: gOk(),
      octop: { pinned: OCTOP_PIN, upstream: { sha: OCTOP_HEAD, date: '2026-10-25', compareError: 'HTTP 502' } },
      inventory: INV,
    }],
  ];
  for (const [name, key, input] of cases) {
    const r = decideUpstream(input);
    expect(`${name}：判失败`, r.problems.length >= 1, JSON.stringify(r.problems));
    expect(`${name}：明说查不到`, r.problems.some((p) => /查不到|算不出/.test(p)), JSON.stringify(r.problems));
    expect(`${name}：${key} 没有自称一致`, !r.lines.some((l) => l.trimStart().startsWith(key) && l.includes('一致')), JSON.stringify(r.lines));
  }
}

console.log('场景5：octop 与记录分叉（diverged）—— 不是「落后」，也不能当没事');
{
  const octop = { pinned: OCTOP_PIN, upstream: { sha: OCTOP_HEAD, date: '2026-10-25', diverged: true } };
  const r = decideUpstream({ goose: gOk(), octop, inventory: INV });
  expect('判失败', r.problems.length === 1, JSON.stringify(r.problems));
  expect('说的是分叉，且没给出「落后 N 个」这种编出来的数', r.problems[0].includes('diverged') && !/落后 \d+ 个 commit/.test(r.problems[0]), r.problems[0]);
  expect('标成「查不到」而不是「落后」', r.unknown.octop === true && r.behind.octop === false);
}

console.log('场景6：缺依赖清单 —— 不能因此崩，也不能假装查过');
{
  const goose = {
    recorded: GOOSE_REC,
    upstream: { tag: '1.54.0', publishedAt: '2026-10-10', newer: [{ tag: '1.54.0', publishedAt: '2026-10-10' }] },
    diff: null,
  };
  const r = decideUpstream({ goose, octop: oOk(), inventory: { goose: [], octop: [], source: null } });
  expect('仍然判失败', r.problems.length === 1, JSON.stringify(r.problems));
  expect('提示清单在哪', r.lines.some((l) => l.includes('UPSTREAM-USAGE.md')), JSON.stringify(r.lines));
  expect('没拿到改动文件时不谎称「没碰依赖」', !r.lines.some((l) => l.includes('没有碰到')), JSON.stringify(r.lines));
}

console.log('场景7：版本号归一 —— v1.53 对 1.53.0 不能判成落后');
{
  expect('compareSemver("v1.53", "1.53.0") === 0', compareSemver('v1.53', '1.53.0') === 0);
  expect('compareSemver("1.54.0", "1.53.9") === 1', compareSemver('1.54.0', '1.53.9') === 1);
  expect('compareSemver("1.53.0", "1.54.0") === -1', compareSemver('1.53.0', '1.54.0') === -1);
  const r = decideUpstream({ goose: gOk(), octop: oOk(), inventory: INV });
  expect('同版本判通过', r.problems.length === 0, JSON.stringify(r.problems));
}

console.log('场景8：清单解析与路径匹配 —— 前缀不同也要认得出是同一个文件');
{
  const inv = parseInventory(
    [
      '# 依赖清单',
      '## goose',
      '| 文件 | 用途 |',
      '| `crates/goose/src/agents/subagent_handler.rs` | 子 agent 调度 |',
      '',
      '## Octop',
      '- `dashboard/src/pages/Experts/index.tsx`',
    ].join('\n'),
  );
  expect('goose 路径收进来了', inv.goose.includes('crates/goose/src/agents/subagent_handler.rs'), JSON.stringify(inv.goose));
  expect('octop 路径归到 octop 段', inv.octop.includes('dashboard/src/pages/Experts/index.tsx'), JSON.stringify(inv.octop));
  expect('带 vendor/ 前缀也算同一个文件', samePath('vendor/goose/crates/goose/src/agents/subagent_handler.rs', 'crates/goose/src/agents/subagent_handler.rs'));
  expect('不同文件不算', !samePath('crates/goose/src/agents/subagent_handler.rs', 'crates/goose/src/main.rs'));
}

console.log('场景9：读 UPSTREAM.md 的那两行 —— 读不到就不算通过（Q076 的真缺陷）');
{
  // 真实表格形态：goose 那行的值加粗，octop 那行的值**不加粗**。
  // 原正则要求 octop 的值两边都有 `**`，于是永远读到 undefined、脚本却照样报 OK。
  const realShaped = [
    '| **我们跟的版本** | **v1.53.0** |',
    '| **我们跟的 commit** | `eb28011249c02cafd389b2d424294c6c1b9cf422` |',
  ].join('\n');
  const parsed = parseUpstreamDoc(realShaped);
  expect('goose 版本读得到（带 v 也归一）', parsed.gooseVersion === '1.53.0', JSON.stringify(parsed));
  expect('octop commit 读得到（不加粗也要读到）', parsed.octopCommit === OCTOP_PIN, JSON.stringify(parsed));

  // 加粗的写法同样要认（Markdown 加粗与否不是契约）。
  const bolded = '| **我们跟的 commit** | **`eb28011249c02cafd389b2d424294c6c1b9cf422`** |';
  expect('加粗写法的 commit 也读得到', parseUpstreamDoc(bolded).octopCommit === OCTOP_PIN);

  // 读不到时必须能看出来 —— 这是本场景要守的东西：不是「解析出来了」，
  // 而是「解析不出来的时候不会被当成核对通过」。
  const broken = parseUpstreamDoc('| 我们跟的 commit | 忘了写 |');
  expect('格式坏掉时 octop commit 为 undefined（调用方据此判失败）', broken.octopCommit === undefined, JSON.stringify(broken));
  expect('格式坏掉时 goose 版本也为 undefined', broken.gooseVersion === undefined, JSON.stringify(broken));
}

console.log('');
if (fail === 0) {
  console.log('上游判定自测：九个场景全对');
  process.exit(0);
}
console.log('上游判定自测：有场景判错了 —— 判定逻辑不可信，先修它再谈上游对齐');
process.exit(1);