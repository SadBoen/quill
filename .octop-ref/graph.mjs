// Octop 前端结构索引（codegraph 的本地替代）。
// Octop 是私有代码，外部 code graph 服务没有它的索引；而 dashboard/src/pages/Experts/index.tsx
// 单文件就 60KB+，每次要抄一个功能就整读进上下文既慢又贵。所以把结构抽成可查询的表。
//
// 用法：
//   node .octop-ref/graph.mjs files            列出索引到的文件与体积
//   node .octop-ref/graph.mjs comps <substr>   组件清单（按名字过滤）
//   node .octop-ref/graph.mjs props <CompName> 某个组件的 props 与签名
//   node .octop-ref/graph.mjs i18n <substr>    i18n key（含调用点文件:行号）
//   node .octop-ref/graph.mjs api <substr>     api 层导出与端点
//   node .octop-ref/graph.mjs find <re>        在 Experts 页里按正则找行
//   node .octop-ref/graph.mjs read <file:line> 打开某处上下文（以该行为中心的 ±20 行，目标行标 `>`）
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs'
import { join, relative } from 'node:path'

const ROOT = new URL('./octop/', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')
const PAGES = join(ROOT, 'dashboard', 'src', 'pages', 'Experts')
const LOCALES = join(ROOT, 'dashboard', 'src', 'locales')
const API = join(ROOT, 'dashboard', 'src', 'api')

const walk = (dir, out = []) => {
  if (!existsSync(dir)) return out
  for (const e of readdirSync(dir)) {
    const p = join(dir, e)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (/\.(tsx?|less|css)$/.test(p)) out.push(p)
  }
  return out
}

const files = walk(PAGES)
const rel = (f) => relative(ROOT, f).replace(/\\/g, '/')

// ---- 组件：export function X / const X = (…) =>  + 同名 props interface ----
function components() {
  const out = []
  for (const f of files) {
    if (!/\.tsx$/.test(f)) continue
    const src = readFileSync(f, 'utf8')
    const lines = src.split(/\r?\n/)
    lines.forEach((ln, i) => {
      const m = ln.match(/^export (?:default )?function ([A-Z]\w*)/)
        || ln.match(/^export const ([A-Z]\w*)\s*[:=]/)
        || ln.match(/^(?:const|function) ([A-Z]\w*)\s*[(:=]/)
      if (!m) return
      const name = m[1]
      if (/^(React|Fragment)/.test(name)) return
      const propsName = `${name}Props`
      const propsLine = lines.findIndex((l) => new RegExp(`(interface|type) ${propsName}\\b`).test(l))
      out.push({
        name,
        file: rel(f),
        line: i + 1,
        kb: (Buffer.byteLength(src) / 1024).toFixed(1),
        hasProps: propsLine >= 0,
        less: existsSync(f.replace(/\.tsx$/, '.module.less')) ? rel(f.replace(/\.tsx$/, '.module.less')) : null,
      })
    })
  }
  return out
}

function propsOf(name) {
  const out = []
  for (const f of files) {
    if (!/\.tsx$/.test(f)) continue
    const src = readFileSync(f, 'utf8')
    const lines = src.split(/\r?\n/)
    const re = new RegExp(`^export (?:interface|type) ${name}Props\\b|^interface ${name}Props\\b|^type ${name}Props\\b`)
    const idx = lines.findIndex((l) => re.test(l))
    if (idx < 0) continue
    let depth = 0
    const body = []
    for (let i = idx; i < lines.length; i += 1) {
      body.push(`${i + 1}| ${lines[i]}`)
      for (const ch of lines[i]) { if (ch === '{') depth += 1; else if (ch === '}') depth -= 1 }
      if (depth <= 0 && i > idx) break
    }
    // 组件签名
    const sig = lines.findIndex((l) => new RegExp(`export function ${name}\\b|export const ${name}\\b`).test(l))
    out.push(`--- ${rel(f)}:${idx + 1} ---`)
    if (sig >= 0) out.push(`${sig + 1}| ${lines[sig]}`)
    out.push(...body)
    out.push('')
  }
  return out.join('\n')
}

function i18nKeys() {
  const rows = []
  const walkAll = (dir, out = []) => {
    for (const f of walk(dir)) if (f.includes(`${'pages'}`)) out.push(f)
    return out
  }
  for (const f of [...walkAll(PAGES), ...walk(join(ROOT, 'dashboard', 'src', 'components'))]) {
    const lines = readFileSync(f, 'utf8').split(/\r?\n/)
    lines.forEach((ln, i) => {
      const re = /\bt\(\s*['"`]([a-zA-Z0-9_.]+)['"`]/g
      let m
      while ((m = re.exec(ln))) rows.push({ key: m[1], file: rel(f), line: i + 1, text: ln.trim().slice(0, 110) })
    })
  }
  return rows
}

// locale 文件里 key -> 中文原文。
// locale 是嵌套 JSON（zh.json 333KB），必须真解析后按路径取值；
// 早先这里用叶子 key 做正则匹配，嵌套层级一深就全查不到（曾整片返回「locale 未找到」）。
function localeText(keys) {
  const maps = new Map()
  for (const name of ['zh', 'en']) {
    const f = join(LOCALES, `${name}.json`)
    if (!existsSync(f)) continue
    const tree = JSON.parse(readFileSync(f, 'utf8'))
    const flat = new Map()
    const walkNode = (node, prefix) => {
      for (const [k, v] of Object.entries(node)) {
        const key = prefix ? `${prefix}.${k}` : k
        if (typeof v === 'string') flat.set(key, v)
        else if (v && typeof v === 'object') walkNode(v, key)
      }
    }
    walkNode(tree, '')
    maps.set(name, flat)
  }
  const out = []
  for (const k of keys) {
    for (const [lang, flat] of maps) {
      const v = flat.get(k)
      if (v !== undefined) out.push({ key: k, lang, text: v, file: `dashboard/src/locales/${lang}.json` })
    }
  }
  return out
}

function apiExports() {
  const out = []
  for (const f of walk(API)) {
    if (!/\.tsx?$/.test(f)) continue
    const src = readFileSync(f, 'utf8')
    const lines = src.split(/\r?\n/)
    lines.forEach((ln, i) => {
      const m = ln.match(/^export (?:async )?function (\w+)/) || ln.match(/^export const (\w+)/)
      if (m) out.push(`${m[1].padEnd(34)} ${rel(f)}:${i + 1}`)
    })
  }
  return out
}

const [, , cmd, arg] = process.argv
if (cmd === 'files') {
  const bySize = files.map((f) => ({ f: rel(f), kb: Buffer.byteLength(readFileSync(f)) / 1024 }))
    .sort((a, b) => b.kb - a.kb)
  for (const r of bySize) console.log(`${r.kb.toFixed(1).padStart(7)} kB  ${r.f}`)
  console.log(`\n合计 ${bySize.length} 个文件`)
} else if (cmd === 'comps') {
  const list = components()
    .filter((c) => !arg || c.name.toLowerCase().includes(arg.toLowerCase()))
    .sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line)
  for (const c of list) {
    console.log(`${c.name.padEnd(28)} ${c.file}:${c.line}  ${c.kb}kB${c.hasProps ? '  [props]' : ''}${c.less ? '  +less' : ''}`)
  }
  console.log(`\n共 ${list.length} 个组件`)
} else if (cmd === 'props') {
  console.log(propsOf(arg || ''))
} else if (cmd === 'i18n') {
  const rows = i18nKeys().filter((r) => !arg || r.key.includes(arg))
  const texts = localeText(rows.map((r) => r.key))
  const byKey = new Map()
  for (const t of texts) {
    if (!byKey.has(t.key)) byKey.set(t.key, t)
    if (t.lang === 'zh' && byKey.get(t.key).lang !== 'zh') byKey.set(t.key, t)
  }
  for (const r of rows) {
    const t = byKey.get(r.key)
    console.log(`${r.key}\n    调用点 ${r.file}:${r.line}\n    ${t ? t.lang : '--'}   ${t ? t.text : '(locale 未找到)'}`)
  }
  console.log(`\n共 ${rows.length} 个调用点 / ${new Set(rows.map((r) => r.key)).size} 个 key`)
} else if (cmd === 'copy') {
  // 只看文案：key + 中文原文 + 调用点，用于直接抄界面文字。
  const rows = i18nKeys().filter((r) => !arg || r.key.includes(arg))
  const byKey = new Map()
  for (const t of localeText(rows.map((r) => r.key))) {
    if (t.lang === 'zh' || !byKey.has(t.key)) byKey.set(t.key, t)
  }
  for (const k of [...new Set(rows.map((r) => r.key))].sort()) {
    const t = byKey.get(k)
    if (t) console.log(`${k}\n    ${t.text}`)
  }
} else if (cmd === 'api') {
  const list = apiExports().filter((l) => !arg || l.toLowerCase().includes(arg.toLowerCase()))
  console.log(list.join('\n'))
  console.log(`\n共 ${list.length} 条`)
} else if (cmd === 'find') {
  if (!arg) { console.error('用法: node .octop-ref/graph.mjs find <regex>'); process.exit(1) }
  const re = new RegExp(arg)
  for (const f of files) {
    readFileSync(f, 'utf8').split(/\r?\n/).forEach((ln, i) => {
      if (re.test(ln)) console.log(`${rel(f)}:${i + 1}| ${ln.trim().slice(0, 150)}`)
    })
  }
} else if (cmd === 'read') {
  const [file, line] = (arg || '').split(':')
  const full = join(ROOT, file)
  if (!existsSync(full)) { console.error(`找不到 ${file}`); process.exit(1) }
  const lines = readFileSync(full, 'utf8').split(/\r?\n/)
  const n = Number(line)
  // 不给行号就从头读；给了就以它为中心对称取 ±20 行。
  // 旧写法是 `from = n-21, to = n+40`，请求行落在窗口第 22 位且**没有任何标记** ——
  // 于是人会照着窗口第一行去引用，正好差 20 行。M6 要求每条上游引用都能被复查，
  // 一个会让人引错行的工具比没有工具更糟，所以改成对称并把目标行标出来。
  const SPAN = 20
  const from = Number.isFinite(n) && n > 0 ? Math.max(0, n - 1 - SPAN) : 0
  const to = Number.isFinite(n) && n > 0 ? Math.min(lines.length, n + SPAN) : Math.min(lines.length, 41)
  for (let i = from; i < to; i += 1) {
    const marker = i + 1 === n ? '>' : ' '
    console.log(`${marker}${String(i + 1).padStart(5)}| ${lines[i]}`)
  }
} else {
  console.log('用法: files | comps [substr] | props <Name> | i18n [substr] | copy [substr] | api [substr] | find <regex> | read <file:line>')
}
