import { apiJson } from '../api/client'

/** 准入策略。四值与后端 `store::DM_POLICIES` 一一对应。 */
export type DmPolicy = 'open' | 'allowlist' | 'pairing' | 'disabled'

export interface ChannelAccountView {
  account_id: string
  account_name: string
  configured: boolean
}

/**
 * 对外的通道视图。
 *
 * **这里刻意没有 token 字段** —— 后端 `store::to_public` 根本不发它。
 * 别在这里「顺手补上」：凭据只进不出是这个接口唯一的安全边界，
 * 而前端拿到它之后无论存不存都会被 devtools 看见。
 */
export interface ChannelView {
  channel_id: string
  kind: string
  name: string
  enabled: boolean
  configured: boolean
  accounts: ChannelAccountView[]
  dm_policy: DmPolicy
}

export interface ChannelsResponse {
  channels: ChannelView[]
  supported: string[]
}

export interface ChannelWrite {
  kind: string
  name?: string
  enabled?: boolean
  channel_id?: string
  config?: {
    dm_policy?: DmPolicy
    allow_from?: string[]
    accounts?: Array<{ account_id: string; account_name: string; token?: string }>
  }
}

export interface QrTicket {
  qrcode_token: string
  /** 二维码图片内容：data URL 或一个 https 链接。 */
  qrcode_url: string
  expires_at: number
}

/**
 * 扫码轮询的结果。
 *
 * `success` 分支也带 `detail?: never`：只有等待/过期分支会带回服务端给的说明，
 * 但把它写成可选而不是只挂在部分分支上，调用处就不必先窄化判空再读 ——
 * 「读一个可能不存在的字段」在这里是常态（扫到一半服务端会给提示）。
 */
export type QrPollResult =
  | { status: 'wait' | 'scaned' | 'expired'; detail?: string }
  | { status: 'success'; channel: ChannelView; detail?: never }

export function loadChannels(): Promise<ChannelsResponse> {
  return apiJson<ChannelsResponse>('/api/channels')
}

export function saveChannel(write: ChannelWrite): Promise<ChannelView> {
  return apiJson<ChannelView>('/api/channels', {
    method: 'POST',
    body: JSON.stringify(write),
  })
}

export function deleteChannel(channelId: string): Promise<{ deleted: string }> {
  return apiJson<{ deleted: string }>(`/api/channels/${encodeURIComponent(channelId)}`, {
    method: 'DELETE',
  })
}

/** 第一步：取二维码。不带凭据，所以不需要带 token。 */
export function startWeixinQr(): Promise<QrTicket> {
  return apiJson<QrTicket>('/api/channels/weixin/qrcode/generate', { method: 'POST' })
}

/** 第二步：轮询扫码状态。确认成功后凭据已经写进通道。 */
export function pollWeixinQr(
  qrcodeToken: string,
  channelId?: string,
): Promise<QrPollResult> {
  return apiJson<QrPollResult>('/api/channels/weixin/qrcode/poll', {
    method: 'POST',
    body: JSON.stringify(
      channelId ? { qrcode_token: qrcodeToken, channel_id: channelId } : { qrcode_token: qrcodeToken },
    ),
  })
}

/**
 * 轮询节奏。
 *
 * 「正在扫」时 2 秒一次，「已经扫到等确认」时也是 2 秒 —— 用户正举着手机，
 * 慢一点会让人以为卡住了。其余情况停掉：没在扫码的时候轮询没有意义。
 */
export function qrRefetchInterval(result: QrPollResult | undefined): number | false {
  if (!result) return false
  if (result.status === 'success' || result.status === 'expired') return false
  return 2_000
}