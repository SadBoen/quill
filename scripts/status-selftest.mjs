#!/usr/bin/env node
/**
 * status.mjs 的自测：**判定逻辑必须被证明会红**。
 *
 * 为什么非有不可：这个仓库有过两次「检查从来没真正生效过」——
 * 一道门禁把 failed 恒算成 0，一道门禁压根没接线。status.mjs 是要取代那些
 * 手工标记的，如果它自己判断写错了，它就成了第三个 —— 而且更隐蔽，
 * 因为它输出的是一张看起来很权威的表。
 *
 * 所以规矩是：**新写的判据必须能证明它会失败**。做法见 `.scripts/gate-selftest.sh`
 * 的文件头 —— 把被测的东西改回坏样子，看检查是否真的红。
 *
 * 这里测的是纯函数 `decide` 与 `selfCheck`（它们刻意不碰文件、不跑命令、
 * 不看时间），所以可以喂合成数据把它们测穷尽 —— 这比变异测试更强，
 * 因为能枚举到真实运行碰不到的组合。
 *
 * 跑法：node scripts/status-selftest.mjs
 */

import { decide, selfCheck, feFullName, VERDICT } from './status.mjs';

let failed = 0;
let checks = 0;

function check(label, actual, expected) {
  checks++;
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  if (ok) {
    console.log(`  ✓ ${label}`);
  } else {
    console.log(`  ✗ ${label}`);
    console.log(`      期望：${JSON.stringify(expected)}`);
    console.log(`      实际：${JSON.stringify(actual)}`);
    failed++;
  }
}

/** 一份「什么都通过」的证据，改哪块就单独改哪块。 */
const baseEvidence = {
  quick: false,
  testsKnown: true,
  knownTests: new Set(['t::passing', 't::failing', 'fe > 用例甲']),
  passing: new Set(['t::passing', 'fe > 用例甲']),
  existingPaths: new Set(),
  cmdResults: { 'true': { verdict: VERDICT.PASS, detail: 'ok' } },
};

const v = (over = {}) => ({ verify: { kind: 'test', name: 't::passing' }, ...over });

console.log('=== status 判定逻辑自测 ===');

console.log('\n场景 1：判据存在且通过 → 已验证');
check(
  '通过的测试判为已验证',
  decide(v(), baseEvidence).verdict,
  VERDICT.PASS,
);

console.log('\n场景 2：测试存在但没通过 → 未通过（不是「没找到」）');
{
  const item = v({ verify: { kind: 'test', name: 't::failing' } });
  const r = decide(item, baseEvidence);
  check('判为未通过', r.verdict, VERDICT.FAIL);
  check(
    '理由要点名是「存在但没过」——改名与回归要分得开',
    /存在但没通过/.test(r.detail),
    true,
  );
}

console.log('\n场景 3：**测试不存在 → 判据本身坏了**（最要命的一类）');
{
  const item = v({ verify: { kind: 'test', name: 't::someone_renamed_this' } });
  const r = decide(item, baseEvidence);
  check('判为判据坏了', r.verdict, VERDICT.BROKEN);
  check(
    '不能判成「未通过」——那会让人以为只是测试红了，而不是判据绑错了',
    r.verdict === VERDICT.FAIL,
    false,
  );
}

console.log('\n场景 4：前端用例名同样要能匹配');
check(
  '前端用例名（文件 > 用例）能判为已验证',
  decide(v({ verify: { kind: 'test', name: 'fe > 用例甲' } }), baseEvidence).verdict,
  VERDICT.PASS,
);

console.log('\n场景 5：quick 模式下 test 类判据必须标「未复查」，不能假装通过');
{
  const ev = { ...baseEvidence, quick: true, testsKnown: false };
  check(
    '判为未复查',
    decide(v(), ev).verdict,
    VERDICT.UNCHECKED,
  );
  check(
    '绝不能判成已验证',
    decide(v(), ev).verdict === VERDICT.PASS,
    false,
  );
}

console.log('\n场景 6：manual 永远只能是「需人工」，不因任何证据变成通过');
{
  const item = { verify: { kind: 'manual', how: '在浏览器里点一遍' } };
  check('有全部通过的证据时仍是需人工', decide(item, baseEvidence).verdict, VERDICT.MANUAL);
  check(
    'quick 模式下也是需人工',
    decide(item, { ...baseEvidence, quick: true, testsKnown: false }).verdict,
    VERDICT.MANUAL,
  );
}

console.log('\n场景 7：absent —— 「我们删掉了它」这类判据');
{
  const item = { verify: { kind: 'absent', path: 'crates/quill-bridge' } };
  check('不存在 → 已验证', decide(item, baseEvidence).verdict, VERDICT.PASS);
  const ev = { ...baseEvidence, existingPaths: new Set(['crates/quill-bridge']) };
  check('又出现了 → 未通过', decide(item, ev).verdict, VERDICT.FAIL);
}

console.log('\n场景 8：cmd —— 命令判据只看退出结果');
{
  const item = { verify: { kind: 'cmd', cmd: 'true' } };
  check('退出 0 → 已验证', decide(item, baseEvidence).verdict, VERDICT.PASS);
  const ev = { ...baseEvidence, cmdResults: { true: { verdict: VERDICT.FAIL, detail: 'exit 1' } } };
  check('退出非 0 → 未通过', decide(item, ev).verdict, VERDICT.FAIL);
}

console.log('\n场景 9：未知的判据种类必须显式报坏，不能默默当通过');
{
  const r = decide({ verify: { kind: 'some_new_kind' } }, baseEvidence);
  check('判为判据坏了', r.verdict, VERDICT.BROKEN);
}

console.log('\n场景 10：self-check 抓判据声明自身的毛病');
{
  // 注意 X1 的测试名必须是 knownTests 里**真有的**那个。这里第一次写成
  // 「真的存在」——一个不在已知集合里的名字，于是 selfCheck 把它报了，
  // 而我的断言「没误报 X1」就红了。是断言写错了，不是 selfCheck 写错了：
  // 一个绑了不存在名字的判据本来就该被报出来。
  const items = [
    { id: 'X1', verify: { kind: 'test', name: 't::passing' } },
    { id: 'X2', verify: { kind: 'test', name: '绑错了' } },
    { id: 'X1', verify: { kind: 'cmd', cmd: 'ls' } }, // id 重复
    { id: 'X3', verify: { kind: 'manual' } }, // manual 没写怎么验
    { id: 'X4' }, // 整个 verify 都没有
  ];
  const problems = selfCheck(items, baseEvidence.knownTests);
  const joined = problems.join('\n');
  check('抓到绑错名字', /X2/.test(joined), true);
  check('抓到 id 重复', /id 重复/.test(joined), true);
  check('抓到 manual 缺 how', /X3/.test(joined), true);
  check('抓到缺 verify', /X4/.test(joined), true);
  check('没误报合法的那条 X1', /X1:/.test(joined), false);
}

console.log('\n场景 11：全部合法的声明必须零问题（别让 self-check 变成永远红的噪音）');
{
  const items = [
    { id: 'Y1', verify: { kind: 'test', name: 't::passing' } },
    { id: 'Y2', verify: { kind: 'cmd', cmd: 'ls' } },
    { id: 'Y3', verify: { kind: 'absent', path: 'nope' } },
    { id: 'Y4', verify: { kind: 'manual', how: '人工点' } },
  ];
  check('零问题', selfCheck(items, baseEvidence.knownTests), []);
}

console.log('\n场景 12：**前端用例名必须由真实代码拼出来**（这里曾经测错了层）');
{
  // 这三条是 2026-10-08 从真实 vitest json 输出里抄出来的形状，
  // 不是编的：ChatPage 的两条 ancestorTitles 是**空数组**，
  // ModelsPage 那条外面套了一层 describe。空数组这形状最容易把拼接写坏，
  // 而旧自测手写 `'fe > 用例甲'` 恰好绕开了它 —— 于是真跑时三条判据全绑不上。
  const real = [
    {
      label: '无 describe 的用例',
      args: [
        '/mnt/d/96_CoderWorld/quill/ui/web/src/chat/ChatPage.test.tsx',
        [],
        '切到另一个会话时，上一个会话的话不会留在屏幕上',
      ],
      want: 'src/chat/ChatPage.test.tsx > 切到另一个会话时，上一个会话的话不会留在屏幕上',
    },
    {
      label: '套了一层 describe 的用例',
      args: [
        '/mnt/d/96_CoderWorld/quill/ui/web/src/models/ModelsPage.test.tsx',
        ['压缩阈值输入框'],
        '明写「暂未生效」：这个数存得下来，但没有任何代码读它',
      ],
      want:
        'src/models/ModelsPage.test.tsx > 压缩阈值输入框 > 明写「暂未生效」：这个数存得下来，但没有任何代码读它',
    },
    {
      label: 'Windows 反斜杠路径',
      args: [
        'D:\\96_CoderWorld\\quill\\ui\\web\\src\\chat\\ChatPage.test.tsx',
        [],
        '用例甲',
      ],
      want: 'src/chat/ChatPage.test.tsx > 用例甲',
    },
  ];
  for (const c of real) {
    check(`拼出全名（${c.label}）`, feFullName(...c.args), c.want);
  }

  // 关键一环：把**函数的真实输出**原样当作判据名喂给 decide。
  // 如果拼接逻辑退化了（比如退回只取文件名），这里必然红。
  const name = feFullName(
    '/mnt/d/96_CoderWorld/quill/ui/web/src/chat/ChatPage.test.tsx',
    [],
    '切到另一个会话时，上一个会话的话不会留在屏幕上',
  );
  const ev = {
    ...baseEvidence,
    knownTests: new Set([name]),
    passing: new Set([name]),
  };
  check(
    '拼出来的名字能被自己的判据匹配上',
    decide({ verify: { kind: 'test', name } }, ev).verdict,
    VERDICT.PASS,
  );
  check(
    '退回 basename 的老写法则绑不上（这正是线上发生过的故障）',
    decide({ verify: { kind: 'test', name: 'ChatPage.test.tsx > 切到另一个会话时，上一个会话的话不会留在屏幕上' } }, ev)
      .verdict,
    VERDICT.BROKEN,
  );
}

console.log('\n场景 13：selfCheck 必须抓到**未知的判据种类**（这是个真实发生过的洞）');
{
  // 2026-10-08：`project/items.mjs` 里 B6-4 写了 `kind: 'script'`、
  // B6-6 写了 `kind: 'auto'`。`decide()` 会把它判成「判据坏了」，
  // 但 `selfCheck()` 里当时**没有「未知 kind」这一支** —— 于是 `--self-check`
  // （门禁里跑的那道）报「全部成立」，两条真实状态未知的判据一路绿灯。
  // 判据声明坏掉比判据未通过更严重，而自检的首要职责正是抓它。
  const items = [
    { id: 'Z1', verify: { kind: 'script', how: '跑个脚本' } },
    { id: 'Z2', verify: { kind: 'auto', how: '跑测试' } },
  ];
  const problems = selfCheck(items, baseEvidence.knownTests);
  const joined = problems.join('\n');
  check('抓到 kind=script', /Z1/.test(joined) && /未知的判据种类/.test(joined), true);
  check('抓到 kind=auto', /Z2/.test(joined), true);
  // 反向：把 kind 改回合法的一种，就不该再报 kind 的问题。
  const ok = selfCheck([{ id: 'Z3', verify: { kind: 'cmd', cmd: 'ls' } }], baseEvidence.knownTests);
  check('合法 kind 不误报', ok, []);
}

console.log('\n' + '='.repeat(60));
if (failed === 0) {
  console.log(`status 自测：${checks} 项断言全对 —— 判定逻辑可信`);
  process.exit(0);
}
console.log(`status 自测：${failed}/${checks} 项判错 —— 判定逻辑不可信，先修它`);
process.exit(1);
