export interface ContentBlock {
  type: string
  text?: string
  thinking?: string
  name?: string
  input?: unknown
  content?: unknown
  is_error?: boolean
  [key: string]: unknown
}

export interface ChatMessage {
  id: string
  seq: number
  role: 'user' | 'assistant'
  status: string
  content: string
  reasoning: string
  input_tokens: number
  output_tokens: number
  turn_ms: number
  error_code: string
  created_at: number
}

export interface MessageHistory {
  messages: ChatMessage[]
}

export const emptyHistory = (): MessageHistory => ({ messages: [] })

export function upsertMessage(messages: ChatMessage[], incoming: ChatMessage): ChatMessage[] {
  const byId = new Map(messages.map((message) => [message.id, message]))
  byId.set(incoming.id, incoming)
  return [...byId.values()].sort((left, right) => left.seq - right.seq || left.id.localeCompare(right.id))
}

export function messageBlocks(message: ChatMessage): ContentBlock[] {
  const blocks: ContentBlock[] = []
  if (message.reasoning.trim()) blocks.push({ type: 'thinking', thinking: message.reasoning })
  if (message.content.trim()) blocks.push({ type: 'text', text: message.content })
  return blocks
}
