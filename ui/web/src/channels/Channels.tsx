import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader } from '../components/Page'
import {
  deleteChannel,
  loadChannels,
  pollWeixinQr,
  qrRefetchInterval,
  saveChannel,
  startWeixinQr,
  testChannel,
  type ChannelTestResult,
  type ChannelView,
  type DmPolicy,
  type QrPollResult,
  type QrTicket,
} from './api'
import './channels.css'

const CHANNEL_QUERY_KEY = ['channels'] as const

export function ChannelsPage(): ReactNode {
  const { t } = useTranslation()
  const channels = useQuery({ queryKey: CHANNEL_QUERY_KEY, queryFn: loadChannels })

  const supported = channels.data?.supported ?? []
  const rows = channels.data?.channels ?? []

  return (
    <div className="page-scroll channels-page">
      <PageHeader
        title={t('channels.title', { defaultValue: '通道' })}
        description={t('channels.description', {
          defaultValue: '把智能体接到浏览器之外。在微信上给智能体发消息，它在那边回你。',
        })}
      />

      {channels.isPending ? (
        <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p>
      ) : null}
      {channels.isError ? (
        <ErrorNotice error={channels.error} />
      ) : null}

      {channels.isSuccess ? (
        <div className="settings-stack">
          {!supported.length ? (
            <p className="form-notice">
              {t('channels.noneSupported', {
                defaultValue: '本实例没有启用任何通道。',
              })}
            </p>
          ) : null}
          {supported.map((kind) => (
            <ChannelCard
              key={kind}
              kind={kind}
              channel={rows.find((r) => r.kind === kind)}
            />
          ))}
        </div>
      ) : null}
    </div>
  )
}

function ChannelCard({
  kind,
  channel,
}: {
  kind: string
  channel: ChannelView | undefined
}): ReactNode {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [busy, setBusy] = useState<false | 'save' | 'delete' | 'qr'>(false)
  const [error, setError] = useState<unknown>(null)
  // 探测结果（null = 还没测过）。与 save/delete 的 error 分开：探测「失败」
  // 是一条正常的结论（ok:false），不是操作出错 —— 混在一起会让用户以为点坏了。
  const [probe, setProbe] = useState<ChannelTestResult | null>(null)

  const enabled = channel?.enabled ?? false
  const configured = channel?.configured ?? false
  const policy: DmPolicy = channel?.dm_policy ?? 'allowlist'

  const save = useMutation({
    mutationFn: saveChannel,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: CHANNEL_QUERY_KEY })
    },
    onError: setError,
  })

  const remove = useMutation({
    mutationFn: deleteChannel,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: CHANNEL_QUERY_KEY })
    },
    onError: setError,
  })

  const toggle = async (next: boolean): Promise<void> => {
    if (busy || !channel) return
    setBusy('save')
    setError(null)
    try {
      await save.mutateAsync({ kind, channel_id: channel.channel_id, enabled: next })
    } finally {
      setBusy(false)
    }
  }

  const drop = async (): Promise<void> => {
    if (busy || !channel) return
    setBusy('delete')
    setError(null)
    try {
      await remove.mutateAsync(channel.channel_id)
    } finally {
      setBusy(false)
    }
  }

  const probeNow = async (): Promise<void> => {
    if (busy || !channel) return
    setBusy('save')
    setError(null)
    setProbe(null)
    try {
      setProbe(await testChannel(channel.channel_id))
    } catch (e) {
      // 真的操作失败（网络/鉴权）才走这里；`ok:false` 是上面那条正常返回。
      setError(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card
      title={t(`channels.kind.${kind}`, { defaultValue: '微信' })}
      description={t(`channels.help.${kind}`, {
        defaultValue: '用手机扫一次码，之后就能在微信上跟智能体对话。',
      })}
    >
      <div className="channel-body">
        {channel && channel.accounts.length ? (
          <dl className="channel-accounts">
            {channel.accounts.map((a) => (
              <div key={a.account_id}>
                <dt>{t('channels.accountId', { defaultValue: '账号' })}</dt>
                <dd><code>{a.account_id}</code></dd>
              </div>
            ))}
          </dl>
        ) : null}

        {kind === 'weixin' ? <WeixinQr channel={channel} onError={setError} /> : null}

        <div className="form-actions">
          <button
            type="button"
            className="primary-button"
            disabled={busy !== false}
            onClick={() => void toggle(!enabled)}
          >
            {enabled
              ? t('channels.stop', { defaultValue: '停用' })
              : t('channels.start', { defaultValue: '启用' })}
          </button>
          {channel ? (
            <button
              type="button"
              className="secondary-button"
              disabled={busy !== false}
              onClick={() => void probeNow()}
            >
              {t('channels.test', { defaultValue: '测试连接' })}
            </button>
          ) : null}
          {channel ? (
            <button
              type="button"
              className="danger-button"
              disabled={busy !== false}
              onClick={() => void drop()}
            >
              {t('channels.remove', { defaultValue: '删除通道' })}
            </button>
          ) : null}
        </div>

        {probe ? (
          <p className="form-notice">
            {probe.ok
              ? t('channels.testOk', { defaultValue: '连接正常：端点可达、已配置凭据。' })
              : (probe.error ?? t('channels.testFail', { defaultValue: '连接测试没通过。' }))}
            {/* 服务端如实声明「没验证登录有效性」—— 把它照原样显示，
                而不是让用户以为点一下就验证了整条链路。 */}
            <small>{probe.note}</small>
          </p>
        ) : null}

        <label className="field">
          <span>{t('channels.dmPolicy', { defaultValue: '谁能私聊' })}</span>
          <select
            value={policy}
            disabled={busy !== false || !channel}
            onChange={(e) => {
              if (!channel) return
              setBusy('save')
              setError(null)
              void save
                .mutateAsync({
                  kind,
                  channel_id: channel.channel_id,
                  config: { dm_policy: e.target.value as DmPolicy },
                })
                .finally(() => setBusy(false))
            }}
          >
            <option value="allowlist">
              {t('channels.policy.allowlist', { defaultValue: '只有名单里的人（默认）' })}
            </option>
            <option value="open">
              {t('channels.policy.open', { defaultValue: '任何人' })}
            </option>
            <option value="pairing">
              {t('channels.policy.pairing', { defaultValue: '配对后（暂未实现）' })}
            </option>
            <option value="disabled">
              {t('channels.policy.disabled', { defaultValue: '谁都不行' })}
            </option>
          </select>
          <small>
            {t('channels.policyHelp', {
              defaultValue:
                '新连上的通道默认是「只有名单里的人」，且名单为空 —— 也就是谁都不能私聊它。要放开得先选「任何人」。',
            })}
          </small>
        </label>

        {!enabled && configured ? (
          <p className="form-notice">
            {t('channels.enabledHint', {
              defaultValue: '已经连上但没启用：消息不会被接收。',
            })}
          </p>
        ) : null}

        {error ? <ErrorNotice error={error} /> : null}
      </div>
    </Card>
  )
}

/**
 * 微信扫码。
 *
 * 流程三步（协议要求，见 `channels/weixin.rs`）：
 * 取码 → 用户拿手机扫并确认 → 服务端拿到凭据。凭据**只写服务端**，
 * 浏览器这边从头到尾没见过 token —— 这正是「接口里没有 token 字段」的原因。
 */
function WeixinQr({
  channel,
  onError,
}: {
  channel: ChannelView | undefined
  onError: (e: unknown) => void
}): ReactNode {
  const { t } = useTranslation()
  const [ticket, setTicket] = useState<QrTicket | null>(null)

  const begin = async (): Promise<void> => {
    onError(null)
    try {
      setTicket(await startWeixinQr())
    } catch (e) {
      onError(e)
    }
  }

  // 拿到码就开始轮询；没码就不轮。
  const poll = useQuery({
    queryKey: ['channels', 'qr', ticket?.qrcode_token],
    queryFn: () => pollWeixinQr(ticket!.qrcode_token),
    enabled: Boolean(ticket),
    refetchInterval: (q) => qrRefetchInterval(q.state.data as QrPollResult | undefined),
    refetchIntervalInBackground: false,
    retry: false,
  })

  const result = poll.data as QrPollResult | undefined
  // 「扫完就别再显示二维码」用**派生**判断而不是 useEffect + setState：
  // 成功后渲染的就是那个按钮，二维码自然消失。写 effect 会在渲染后多跑一遍
  // 并触发第二次渲染，而这里并不需要任何副作用。
  const done = result?.status === 'success'
  const expired = result?.status === 'expired'

  return (
    <section className="channel-qr">
      {!ticket || expired || done ? (
        <button type="button" className="secondary-button" onClick={() => void begin()}>
          {channel?.configured
            ? t('channels.reconnect', { defaultValue: '重新扫码' })
            : t('channels.connect', { defaultValue: '微信扫码连接' })}
        </button>
      ) : (
        <div className="channel-qr-panel">
          {ticket.qrcode_url ? (
            ticket.qrcode_url.startsWith('data:') ? (
              <img src={ticket.qrcode_url} alt={t('channels.qrAlt', { defaultValue: '微信登录二维码' })} />
            ) : (
              <p>
                <a href={ticket.qrcode_url} target="_blank" rel="noreferrer">
                  {t('channels.qrLink', { defaultValue: '打开微信登录页' })}
                </a>
              </p>
            )
          ) : null}
          <p className="page-status">
            {result?.status === 'scaned'
              ? t('channels.qrScanned', { defaultValue: '已扫码，请在手机上确认…' })
              : t('channels.qrWaiting', { defaultValue: '请用微信扫码，并在手机上确认。' })}
          </p>
          {result?.detail ? <p className="form-notice">{result.detail}</p> : null}
          <button type="button" className="secondary-button" onClick={() => setTicket(null)}>
            {t('common.cancel', { defaultValue: '取消' })}
          </button>
        </div>
      )}
      {expired ? (
        <p className="form-notice">
          {t('channels.qrExpired', { defaultValue: '二维码过期了（约 5 分钟），重新取一张。' })}
        </p>
      ) : null}
    </section>
  )
}