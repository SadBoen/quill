/**
 * 校验 ui/web/src/experts/library.ts 与 .octop-ref 源数据的一致性。
 *
 * 独立的「源 vs 产物」双向比对：不读 library.ts 的运行时结果，只解析其源码文本，
 * 避免「生成器自证」。跑法：node .library-check.mjs（仓库根）。
 */
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import vm from 'node:vm';

const ROOT = path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
const LIB = path.join(ROOT, '.octop-ref/octop/src/octop/infra/agents/experts/library');
const OUT_DIR = path.join(ROOT, 'ui/web/src/experts/library');
const TS = path.join(ROOT, 'ui/web/src/experts/library.ts');

const MAX_INSTRUCTIONS_CHARS = 20000; // crates/quill-agent/src/expert.rs:13，按 chars().count()

let failures = 0;
let checks = 0;
const ok = (cond, label, detail = '') => {
  checks += 1;
  if (cond) {
    console.log(`  ok   ${label}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${label}${detail ? ` — ${detail}` : ''}`);
  }
};

const ts = fs.readFileSync(TS, 'utf8');
const sha = (p) => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

console.log('== 1. 专家数量与 id 集合 ==');
const dirIds = fs
  .readdirSync(LIB, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name)
  .sort();
const expectedIds = dirIds.filter((id) => id !== 'default');
console.log(`  源 library/ 子目录: ${dirIds.length} 个（含 default），排除 default 后 ${expectedIds.length} 个`);

const exportedIds = [...ts.matchAll(/^ {4}id: "([^"]+)",$/gm)].map((m) => m[1]);
ok(exportedIds.length === 17, '导出条数为 17', `实际 ${exportedIds.length}`);
ok(!exportedIds.includes('default'), '不含 default');
ok(
  exportedIds.slice().sort().join(',') === expectedIds.slice().sort().join(','),
  'id 与源子目录名一一对应',
  `产物=[${exportedIds.slice().sort().join(',')}] 期望=[${expectedIds.slice().sort().join(',')}]`,
);
const sortedExpected = expectedIds
  .slice()
  .sort((a, b) => (a === 'general-assistant' ? 0 : 1) - (b === 'general-assistant' ? 0 : 1) || (a < b ? -1 : 1));
ok(exportedIds.join(',') === sortedExpected.join(','), '排序 = general-assistant 置顶 + 其余 id 升序', exportedIds.join(','));
ok(
  ts.includes('catalog.py') && ts.includes('852-853'),
  '代码里写明了排除 default 的 Octop 出处',
);
ok(ts.includes('eb28011249c02cafd389b2d424294c6c1b9cf422') && /MIT/.test(ts), '文件头保留 pin 与 MIT 许可');

console.log('\n== 2. ?raw 导入目标存在且与源逐字节一致（sha256） ==');
const importRe = /^import (\w+) from '\.\/library\/([^/]+)\/([^']+)\?raw'$/gm;
const imports = [...ts.matchAll(importRe)].map((m) => ({ var: m[1], id: m[2], file: m[3] }));
ok(imports.length > 0, `解析到 ${imports.length} 条 ?raw 导入`);
let fileMismatch = [];
for (const im of imports) {
  const dst = path.join(OUT_DIR, im.id, im.file);
  const src = path.join(LIB, im.id, im.file);
  if (!fs.existsSync(dst)) {
    ok(false, `产物文件存在 ${im.id}/${im.file}`);
    continue;
  }
  if (!fs.existsSync(src)) {
    ok(false, `源文件存在 ${im.id}/${im.file}`);
    continue;
  }
  if (sha(dst) === sha(src)) {
    ok(true, `sha256 一致 ${im.id}/${im.file} (${fs.statSync(dst).size}B)`);
  } else {
    fileMismatch.push(`${im.id}/${im.file}`);
    ok(false, `sha256 一致 ${im.id}/${im.file}`);
  }
}
// 产物目录不得有源里没有的多余 md
const outMd = [];
for (const id of fs.readdirSync(OUT_DIR, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name)) {
  for (const f of fs.readdirSync(path.join(OUT_DIR, id))) {
    if (f.endsWith('.md')) outMd.push(`${id}/${f}`);
  }
}
ok(
  outMd.length === imports.length,
  `产物 md 文件数 (${outMd.length}) = 导入数 (${imports.length})，无多余文件`,
  outMd.filter((p) => !imports.some((i) => `${i.id}/${i.file}` === p)).join(','),
);

console.log('\n== 3. 逐专家：manifest 字段原样性 / 条数 / instructions 长度 ==');
/** 从 library.ts 源码里抠出某个专家的对象文本（平衡括号扫描）。 */
function entryText(id) {
  const at = ts.indexOf(`\n    id: "${id}",`);
  ok(at >= 0, `library.ts 含 ${id} 条目`);
  if (at < 0) return null;
  const start = ts.lastIndexOf('{', at);
  let depth = 0;
  let i = start;
  for (; i < ts.length; i += 1) {
    if (ts[i] === '{') depth += 1;
    else if (ts[i] === '}') {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  return ts.slice(start, i + 1);
}
/** 解析对象文本里的字面量字段（用 vm 跑，键与值都是 JSON 安全字面量）。 */
function field(objText, key) {
  const m = objText.match(new RegExp(`\\b${key}: (\\[.*?\\]|\\{.*?\\}|"(?:[^"\\\\]|\\\\.)*"|true|false|\\d+|null)(?=[,\\n])`, 's'));
  if (!m) return undefined;
  return vm.runInNewContext(m[1]);
}

const charTable = [];
const oversized = [];
const personaMissing = [];
for (const id of expectedIds) {
  console.log(`\n-- ${id}`);
  const m = JSON.parse(fs.readFileSync(path.join(LIB, id, 'manifest.json'), 'utf8'));
  const obj = entryText(id);
  if (!obj) continue;

  for (const [key, srcObj] of [
    ['labelZh', m.label?.zh], ['labelEn', m.label?.en],
    ['descriptionZh', m.description?.zh], ['descriptionEn', m.description?.en],
    ['welcomeZh', m.welcome_message?.zh], ['welcomeEn', m.welcome_message?.en],
    ['iconName', m.icon_name], ['color', m.color],
  ]) {
    const got = field(obj, key);
    ok(got === (srcObj ?? ''), `${key} 与 manifest 原样一致`, got === (srcObj ?? '') ? '' : `产物=${JSON.stringify(got)} 源=${JSON.stringify(srcObj)}`);
  }

  const qps = field(obj, 'quickPrompts');
  const srcQps = m.quick_prompts ?? [];
  ok(Array.isArray(qps) && qps.length === srcQps.length, `quick_prompts 条数 = ${srcQps.length}`);
  if (Array.isArray(qps)) {
    let fieldsOk = true;
    let firstBad = '';
    srcQps.forEach((sp, i) => {
      const gp = qps[i];
      for (const [key, sv] of [
        ['titleZh', sp.title?.zh], ['titleEn', sp.title?.en],
        ['descriptionZh', sp.description?.zh], ['descriptionEn', sp.description?.en],
        ['promptZh', sp.prompt?.zh], ['promptEn', sp.prompt?.en],
        ['color', sp.color], ['iconName', sp.icon_name],
      ]) {
        if (gp[key] !== (sv ?? '')) {
          fieldsOk = false;
          firstBad = firstBad || `qp#${i}.${key}: 产物=${JSON.stringify(gp[key])} 源=${JSON.stringify(sv)}`;
        }
      }
    });
    ok(fieldsOk, `quick_prompts 全部字段原样一致（${srcQps.length} 条）`, firstBad);
  }

  const teZh = field(obj, 'taskExamplesZh');
  const teEn = field(obj, 'taskExamplesEn');
  ok(
    JSON.stringify(teZh) === JSON.stringify(m.task_examples?.zh ?? []),
    `taskExamplesZh 与源原样一致（${(m.task_examples?.zh ?? []).length} 条）`,
  );
  ok(
    JSON.stringify(teEn) === JSON.stringify(m.task_examples?.en ?? []),
    `taskExamplesEn 与源原样一致（${(m.task_examples?.en ?? []).length} 条）`,
  );

  const files = field(obj, 'instructionsFiles') ?? [];
  const present = (m.prompt_files ?? []).filter((f) => fs.existsSync(path.join(LIB, id, f)));
  const absent = (m.prompt_files ?? []).filter((f) => !fs.existsSync(path.join(LIB, id, f)));
  ok(
    JSON.stringify(files) === JSON.stringify(present),
    `instructionsFiles = prompt_files 声明顺序中磁盘存在的部分（${present.join(' > ') || '无'}）`,
    `产物=${JSON.stringify(files)} 期望=${JSON.stringify(present)}`,
  );
  const pm = field(obj, 'personaMissing');
  ok(pm === (absent.length > 0), `personaMissing = ${absent.length > 0}`, absent.length ? `缺失: ${absent.join(',')}` : '');
  if (absent.length) personaMissing.push(`${id}: ${absent.join(',')}`);

  // instructions 实测字符数：从源 md 独立重拼，再和 library.ts 里记录的 instructionsChars 对齐
  const expectedInstr = present.map((f) => `<!-- ${f} -->\n` + fs.readFileSync(path.join(LIB, id, f), 'utf8')).join('\n\n');
  const chars = [...expectedInstr].length;
  const recorded = field(obj, 'instructionsChars');
  ok(recorded === chars, `instructionsChars 实测 ${chars}（脚本独立重拼）`, `记录=${recorded}`);
  const ov = field(obj, 'oversized');
  ok(ov === chars > MAX_INSTRUCTIONS_CHARS, `oversized = ${chars > MAX_INSTRUCTIONS_CHARS}`);
  if (chars > MAX_INSTRUCTIONS_CHARS) oversized.push(`${id}=${chars}字符`);
  charTable.push({ id, files: present.length, chars, bytes: Buffer.byteLength(expectedInstr, 'utf8') });
}

console.log('\n== 4. instructions 字符数（上限 ' + MAX_INSTRUCTIONS_CHARS + '） ==');
console.log('  id'.padEnd(32) + 'files  chars    bytes');
for (const r of charTable) {
  console.log('  ' + r.id.padEnd(30) + String(r.files).padEnd(7) + String(r.chars).padEnd(8) + String(r.bytes).padEnd(7) + (r.chars > MAX_INSTRUCTIONS_CHARS ? '  <= 超限' : ''));
}
const maxRow = charTable.reduce((a, b) => (b.chars > a.chars ? b : a));
console.log(`  最大: ${maxRow.id} = ${maxRow.chars} 字符（上限 ${MAX_INSTRUCTIONS_CHARS}，余量 ${MAX_INSTRUCTIONS_CHARS - maxRow.chars}）`);
ok(oversized.length === 0, `无超限专家`, oversized.join(', '));

console.log('\n== 5. sha256 汇总 ==');
console.log(`  校验文件数: ${imports.length}`);
console.log(`  全部一致: ${fileMismatch.length === 0 ? '是' : '否 ' + fileMismatch.join(', ')}`);

console.log('\n== 6. 异常项汇总 ==');
console.log(`  人格文件缺失 (personaMissing): ${personaMissing.length ? personaMissing.join(' | ') : '无'}`);
console.log(`  超过 20000 字符 (oversized): ${oversized.length ? oversized.join(' | ') : '无'}`);
console.log(`  sha256 不一致: ${fileMismatch.length ? fileMismatch.join(', ') : '无'}`);

console.log(`\n== 结果: ${checks - failures}/${checks} 通过, ${failures} 失败 ==`);
process.exit(failures === 0 ? 0 : 1);
