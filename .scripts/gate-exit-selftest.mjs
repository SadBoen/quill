#!/usr/bin/env node
/**
 * 门禁退出码自检（queue Q106）：**每一个门禁脚本都必须有非 0 退出路径**。
 *
 * ## 这道门禁回答什么问题
 *
 * 「有没有哪个门禁脚本，发现了问题却照样退 0？」—— 2026-10-08 真的抓到过一个：
 * `.i18n-check.mjs` 里**一个 `process.exit` 都没有**，它把问题逐条打印得很好看，
 * 然后退出 0；CI 里跑的就是它，于是「漏翻译会报红」是句空头承诺：
 * 打印得再详细，流水线永远是绿的。**一个永远退 0 的判据不是判据**
 * （最高指示第 5 条：判据必须机器可跑）。
 *
 * 这类缺陷靠人偶然发现太不可靠 —— 所以这道门禁把「有没有非 0 退出路径」变成
 * 可跑的检查：任何门禁脚本里找不到下列任一形态，就直接红。
 *
 * ## 判定形态（三者任一即算合格）
 *
 * 1. `process.exit(<非零>)` / `process.exit(cond ? 0 : 1)`（条件式里出现非零字面量）；
 * 2. `process.exitCode = <非零>`
 * 3. `throw`（未捕获的异常会让 Node 退非 0）。
 *
 * **已知局限（如实写）**：这是**源码级**检查，只能证明「存在这样一条路径」，
 * 不能证明「它在正确的时候会走到」—— 真正的证明是逐脚本注入一次故障看它红
 * （`.layer-guard` / `.api-compat-check` / `.i18n-check` 都是这么验过的）。
 * 这道门禁的价值在于**拦住「压根没写」**这种最常见的形态。
 *
 * 用法：node .scripts/gate-exit-selftest.mjs [--self-test]
 */

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..')

/** 有没有「非 0 退出路径」。纯函数，便于自测。 */
export function hasNonZeroExitPath(src) {
  for (const line of src.split('\n')) {
    // 注释行不算（例如本文件自己的说明里就写着 process.exit）
    const code = line.replace(/^\s*(\/\/|\*|\/\*).*$/, '')
    if (!code.trim()) continue
    if (/process\.exitCode\s*=\s*[^0\s]/.test(code)) return true
    const m = code.match(/process\.exit\(([^)]*)\)/)
    if (m) {
      // 参数是常量：只要不是字面量 0 就算有非 0 路径
      const arg = m[1]
      if (!/^\s*0\s*$/.test(arg)) return true
    }
  }
  return /\bthrow\b/.test(src.replace(/^\s*\*.*$/gm, ''))
}

/** 全部门禁脚本（仓库根的点号脚本 + `.scripts/` 下的，排除本文件与纯自测脚本）。 */
export function gateScripts(root) {
  const out = []
  for (const dir of [root, join(root, '.scripts')]) {
    let entries
    try {
      entries = readdirSync(dir)
    } catch {
      continue
    }
    for (const name of entries) {
      if (!name.endsWith('.mjs')) continue
      const full = join(dir, name)
      if (!statSync(full).isFile()) continue
      if (name === 'gate-exit-selftest.mjs') continue // 自己
      out.push(full)
    }
  }
  return out.sort()
}

function check(root) {
  const files = gateScripts(root)
  if (files.length < 8) {
    console.error(
      `FAIL: 只扫到 ${files.length} 个门禁脚本 —— 路径很可能不对（${root}）。` +
        '「扫了个空还判绿」是这类检查最丢人的坏法。',
    )
    return 1
  }
  const bad = []
  for (const f of files) {
    const src = readFileSync(f, 'utf8')
    if (!hasNonZeroExitPath(src)) bad.push(f)
  }
  if (bad.length > 0) {
    console.error('FAIL: 这些门禁脚本没有任何非 0 退出路径（发现问题也会退 0）：')
    for (const f of bad) console.error(`  ${f.replace(ROOT, '.')}`)
    console.error(
      '\n下一步：在「发现问题」的分支上加 `process.exit(1)`（或置 `process.exitCode = 1`），' +
        '并按仓库口径给一段带「下一步」的 FAIL 文案；然后注入一次故障实测它真的退 1。',
    )
    return 1
  }
  console.log(`OK: ${files.length} 个门禁脚本都有非 0 退出路径（源码级检查，见文件头的局限说明）。`)
  return 0
}

function selfTest() {
  const cases = [
    ['process.exit(1)\n', true, '直接 exit(1)'],
    ['process.exit(failures === 0 ? 0 : 1)\n', true, '条件式'],
    ['process.exitCode = 1\n', true, 'exitCode = 1'],
    ['throw new Error("boom")\n', true, '抛错'],
    ['// process.exit(1) 只出现在注释里\nconsole.log("ok")\n', false, '注释不算'],
    ['console.log("问题数:", n)\n', false, '只打印不退出（i18n 当初的样子）'],
    ['process.exit(0)\n', false, '永远退 0 等于没有失败路径'],
  ]
  let pass = 0
  for (const [src, want, label] of cases) {
    const got = hasNonZeroExitPath(src)
    if (got === want) {
      pass++
      console.log(`  ok   ${label}`)
    } else {
      console.error(`  FAIL ${label}：期望 ${want}，实际 ${got}`)
    }
  }
  const files = gateScripts(ROOT)
  if (files.length >= 8) {
    pass++
    console.log(`  ok   扫到 ${files.length} 个门禁脚本（不是空扫）`)
  } else {
    console.error(`  FAIL 只扫到 ${files.length} 个脚本`)
  }
  console.log(`\n自测结果：${pass} 通过 / ${cases.length + 1 - pass} 失败`)
  return pass === cases.length + 1 ? 0 : 1
}

const argv = process.argv.slice(2)
const rc = argv.includes('--self-test') ? selfTest() : check(ROOT)
process.exit(rc)
