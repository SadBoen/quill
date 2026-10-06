#!/usr/bin/env node
// 扫仓库里有没有 U+FFFD 替换字符（乱码残留）。
//
// 为什么要有这道门禁：Windows PowerShell 5.1 改含中文的文件时会按 ANSI 写，
// 把 UTF-8 中文打成 U+FFFD。之前已经中招过两轮，其中一处是
// **面向用户显示的错误文案**（error.rs 的 ADVICE_TOOL_LOOP_EXHAUSTED），
// 用户看到的是「只是它一直[方块][方块]调工具」—— 这种错跑测试发现不了，
// 因为它照样能编译、照样能通过所有断言。
//
// 范围：只扫入库文件。跳过 node_modules、_raw（原始下载件）与 .octop-ref。

import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

const ROOT = process.cwd()
const SKIP_DIRS = new Set([
  'node_modules',
  '.git',
  'target',
  'dist',
  '.octop-ref',
  // vendor 是上游源码，逐字节保持原样，不归我们改
  'vendor',
])
const SKIP_PATH_PARTS = ['TESTSETS/_raw']
const TEXT_EXT = /\.(rs|tsx?|mjs|js|json|md|sh|py|toml|css|html|yml|yaml|txt)$/

const findings = []
let scanned = 0

function walk(dir) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue
    const full = join(dir, name)
    const rel = relative(ROOT, full).replace(/\\/g, '/')
    if (SKIP_PATH_PARTS.some((p) => rel.startsWith(p))) continue
    let st
    try {
      st = statSync(full)
    } catch {
      continue
    }
    if (st.isDirectory()) {
      walk(full)
      continue
    }
    if (!TEXT_EXT.test(name)) continue
    let text
    try {
      text = readFileSync(full, 'utf8')
    } catch {
      continue
    }
    scanned += 1
    const lines = text.split('\n')
    lines.forEach((line, i) => {
      const at = line.indexOf('\uFFFD')
      if (at === -1) return
      findings.push({ file: rel, line: i + 1, col: at + 1, text: line.trim().slice(0, 110) })
    })
  }
}

walk(ROOT)

console.log(`扫了 ${scanned} 个文本文件`)
if (findings.length === 0) {
  console.log('OK: 没有 U+FFFD 替换字符')
  process.exit(0)
}
console.log(`\n发现 ${findings.length} 处乱码（U+FFFD）—— 面向用户的文案里出现这个，用户会直接看到方块：\n`)
for (const f of findings) {
  console.log(`  ${f.file}:${f.line}:${f.col}`)
  console.log(`      ${f.text}`)
}
console.log('\n用 edit / write 工具改（PowerShell 的 Set-Content 会把中文打成乱码）。')
process.exit(1)