import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import i18n from '../i18n'
import { Transcript } from './Transcript'
import type { ChatMessage } from './model'

/**
 * 每条回复末尾那一行的判据。
 *
 * 这里的断言全部围绕一件事：**这一行说的是「这一条」，不是整个会话。**
 * 之前耗时在聊天页底部另有一份会话级求和，同一件事两个地方各说一次，
 * 且两个数不一样，用户无从判断眼前这个是哪来的。
 */

function assistant(over: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: 'm1',
    seq: 1,
    role: 'assistant',
    status: 'complete',
    content: '1+1 等于 2。',
    reasoning: '',
    input_tokens: 598,
    output_tokens: 9,
    turn_ms: 2084,
    error_code: '',
    created_at: 1_700_000_000_000,
    ...over,
  }
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

describe('Transcript 每条末尾的用量行', () => {
  it('这一条花了多久、出入参多少，都在这一行里', () => {
    render(<Transcript messages={[assistant()]} running={false} />)
    const line = screen.getByTestId('chat-turn-usage')
    expect(line.textContent).toContain('2084 毫秒')
    expect(line.textContent).toContain('入 598')
    expect(line.textContent).toContain('出 9')
  })

  it('速度由这一条自己的出参与耗时算出，不是会话级求和', () => {
    render(<Transcript messages={[assistant()]} running={false} />)
    // 9 token / 2.084 秒 = 4.3
    expect(screen.getByTestId('chat-turn-usage').textContent).toContain('4.3 tok/s')
  })

  it('出参为 0 时不显示速度，而不是显示 0.0 tok/s', () => {
    // 出参 0 说明这条一个字都没生成。写「0.0 tok/s」是在报告一个没意义的数，
    // 而那一行已经有耗时和出入参了，少一个数读起来不像坏掉。
    render(<Transcript messages={[assistant({ output_tokens: 0 })]} running={false} />)
    const line = screen.getByTestId('chat-turn-usage')
    expect(line.textContent).not.toContain('tok/s')
    expect(line.textContent).toContain('2084 毫秒')
  })

  it('用户自己那条不显示用量行', () => {
    render(
      <Transcript
        messages={[assistant({ id: 'u1', role: 'user', content: '1+1 等于几？', turn_ms: 0 })]}
        running={false}
      />,
    )
    expect(screen.queryByTestId('chat-turn-usage')).toBeNull()
  })
})