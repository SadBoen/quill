// 一次性核查脚本：比对 ui/web 各 t() 调用点传入的占位符 与 语言包字符串里的占位符。
// 用法：node .i18n-check.mjs
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

// 路径**按脚本自身位置**推，不写死盘符。
// 写死 `D:/96_CoderWorld/quill/...` 的代价：在 WSL 里跑会 ENOENT，
// 别人 clone 到别的目录也跑不了 —— 一道门禁只能在一台机器的一个目录上跑，
// 那它守的不是仓库，是那台机器。
const ROOT = dirname(fileURLToPath(import.meta.url))
const SRC = join(ROOT, 'ui', 'web', 'src')
const RES = join(SRC, 'i18n', 'resources.ts')

const raw = readFileSync(RES, 'utf8')
const zhStart = raw.indexOf('export const zhCN')
const zhSrc = raw.slice(zhStart)

// ---- 解析出 zh 语言包：ns.key(可任意深) -> 字符串 ----
// 早先这里是逐行正则，只认两层（ns.key），于是 chat.sidebar.label 这类嵌套 key
// 全被误报成「语言包缺 key」。改成平衡括号抠出对象字面量后真的求值，深度不再受限。
const zh = {}
{
  const start = zhSrc.indexOf('translation: {')
  if (start < 0) throw new Error('resources.ts 里找不到 translation: {')
  const open = zhSrc.indexOf('{', start)
  let depth = 0
  let end = -1
  for (let i = open; i < zhSrc.length; i += 1) {
    const ch = zhSrc[i]
    if (ch === '{') depth += 1
    else if (ch === '}') {
      depth -= 1
      if (depth === 0) { end = i; break }
    }
  }
  if (end < 0) throw new Error('translation 对象括号不配对')
  const tree = new Function(`"use strict";return (${zhSrc.slice(open, end + 1)});`)()
  const flatten = (node, prefix) => {
    for (const [k, v] of Object.entries(node)) {
      const key = prefix ? `${prefix}.${k}` : k
      if (typeof v === 'string') zh[key] = v
      else if (v && typeof v === 'object') flatten(v, key)
    }
  }
  flatten(tree, '')
}

// ---- 收集所有源码文件里的 t('ns.key', { ... }) 调用 ----
function walk(dir, out = []) {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (/\.tsx?$/.test(p)) out.push(p)
  }
  return out
}

const problems = []
const seen = new Set()
for (const file of walk(SRC)) {
  if (file.endsWith(join('i18n', 'resources.ts'))) continue
  const text = readFileSync(file, 'utf8')
  // 平衡括号地抓 t('key', { ...对象... })
  const re = /t\(\s*'([A-Za-z0-9_.]+)'\s*,\s*\{/g
  let m
  while ((m = re.exec(text))) {
    const key = m[1]
    let i = m.index + m[0].length
    let depth = 1
    while (i < text.length && depth > 0) {
      if (text[i] === '{') depth++
      else if (text[i] === '}') depth--
      i++
    }
    const opts = text.slice(m.index + m[0].length, i - 1)
    const line = text.slice(0, m.index).split('\n').length
    // 先把字符串字面量（含 defaultValue）整体抹掉：里面的 {id}、`${...}` 都不是传参。
    // 不抹的话 `defaultValue: '...PATCH /api/sessions/{id}。'` 会被当成传了 id，
    // 报出一堆「死参数」假阳性，门禁一旦红久了就没人再看。
    const stripped = opts.replace(/'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`/g, "''")
    // `key: value` 形式
    const kv = [...stripped.matchAll(/([A-Za-z0-9_]+)\s*:/g)].map((x) => x[1])
    // 再把 `key: value` 整段抠掉，剩下的才是 ES6 简写 `{ field, route }`，
    // 否则 `route: MCP_ROUTE` 里的 MCP_ROUTE 会被误当成参数名。
    const shorthandSrc = stripped.replace(/[A-Za-z0-9_]+\s*:\s*[^,{}]+/g, ',')
    const shorthand = [...shorthandSrc.matchAll(/(?:^|[{,\s])([A-Za-z0-9_]+)\s*(?=[,}]|$)/g)].map((x) => x[1])
    const passed = new Set([...kv, ...shorthand])
    passed.delete('defaultValue')

    const str = zh[key]
    if (str === undefined) {
      problems.push({ kind: '语言包缺 key', file, line, key, detail: '' })
      continue
    }
    const need = new Set([...str.matchAll(/\{\{(\w+)\}\}/g)].map((x) => x[1]))
    const missing = [...need].filter((n) => !passed.has(n))
    const extra = [...passed].filter((n) => !need.has(n))
    if (missing.length) {
      problems.push({
        kind: '语言包要占位符但调用点没传',
        file, line, key,
        detail: `缺 ${missing.map((n) => '{{' + n + '}}').join(' ')}；实传 [${[...passed].join(', ')}]`,
      })
    }
    if (extra.length) {
      problems.push({
        kind: '调用点传了但语言包用不到（死参数）',
        file, line, key,
        detail: `多余 ${extra.join(', ')}；语言包占位符 [${[...need].join(', ')}]`,
      })
    }
    seen.add(key)
  }
}

console.log(`zh 语言包 key 总数: ${Object.keys(zh).length}`)
console.log(`扫到的 t() 调用点涉及 key 数: ${seen.size}`)
console.log(`问题数: ${problems.length}\n`)
const byKind = {}
for (const p of problems) (byKind[p.kind] ??= []).push(p)
for (const [k, list] of Object.entries(byKind)) {
  console.log(`### ${k} (${list.length})`)
  for (const p of list) {
    console.log(`  ${p.file.replace(SRC, 'src')}:${p.line}  ${p.key}  ${p.detail}`)
  }
  console.log('')
}
if (problems.length === 0) console.log('OK: 所有 t() 调用的占位符与 zh 语言包完全一致')

// ---- 退出码：**有问题就必须非 0**（2026-10-08 修，queue Q074）------------------
// 此前这个脚本里一个 `process.exit` 都没有：它把问题逐条打印得很清楚，然后**照样
// 退出 0**。CI 里跑的是 `node .i18n-check.mjs`，于是「漏翻译会报红」是句空头承诺 ——
// 打印得再详细，流水线也永远是绿的（最高指示第 5 条要的是「判据机器可跑」，
// 一个永远退 0 的判据不是判据）。
//
// 实测（本次修的当天）：把一条 zh 文案的 `{{count}}` 改成 `{count}`，
// 脚本如实报出「死参数 (1)」而退出码是 0；加上这两行之后同一次注入返回 1。
if (problems.length > 0) {
  console.error(
    `\nFAIL: i18n 有 ${problems.length} 个问题（见上）。\n` +
      '下一步：按每一行的 `文件:行 key 说明` 逐个改；' +
      '若问题在语言包侧，改 `ui/web/src/i18n/resources.ts`。',
  )
  process.exit(1)
}
