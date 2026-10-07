/**
 * 「这台实例到底有没有某个能力」的判定，以及「该把哪一条错误给用户看」。
 *
 * **为什么要单独一个文件**：2026-10-08 在浏览器里逐页实点时抓到一个很难看的问题 ——
 * `/api/cron` 未登记时，自动化页顶部如实报了红字，底下却照样给出一个能填、
 * 能点「创建」的完整表单。填完提交，POST 直接 404，而界面**一个字提示都没有**。
 * 用户会以为任务建好了，然后等它永远不来。
 *
 * 假按钮比没有按钮坏得多：没有按钮用户知道没这功能，假按钮用户以为有。
 * 所以这里只做两件事 —— 把「没这个能力」判出来（好让页面别画那个表单），
 * 以及别让一条后台的常驻错误把你刚点的那个操作的结果挡掉。
 *
 * 纯函数：不碰 React、不发请求、不看时间，所以能被测穷尽。
 */
import { ApiError } from '../api/client';

/**
 * 这条路由在服务端是不是没登记。
 *
 * **只认服务端明说「没这条路由」**：404 且错误码是 `not_found`。
 * 别的 404（比如资源不存在）不算 —— 那是个案，不是能力缺失。
 * 拿不准就返回 false，页面照常画 —— 多画一个坏掉的表单，
 * 比把还在正常工作的功能藏起来要好。
 */
export function routeMissing(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404 && error.code === 'not_found'
}

/**
 * 该把哪一条错误显示给用户。
 *
 * **为什么不能一串 `??`**（这就是那个静默失败的机制）：
 * `actionError ?? cron.error ?? saveCron.error` 里，`cron.error` 在
 * `/api/cron` 没登记时**永远非空**，于是它会把 `saveCron.error` 挡在后面 ——
 * 用户刚点的「创建」失败了，屏幕上却还是那条跟这次点击无关的旧提示。
 *
 * 规则：**你刚做的那个动作的错误，优先于一直在报的后台错误。**
 * 后台错误仍然会显示，只是排在后面、且在没有动作错误时才占位。
 */
export function pickVisibleError(...candidates: unknown[]): unknown {
  // 判「有没有值」而不是判真值：`if (c)` 会把 0 与 '' 这类合法值一起吞掉。
  for (const c of candidates) {
    if (c !== null && c !== undefined) return c
  }
  return null
}