/**
 * 预设提供商目录：只收录各厂商公开的 base_url，字段与 `/api/admin/providers` 的
 * `preset_id` / `protocol` / `base_url` 一一对应。这里是目录，不是数据源 ——
 * 页面上「已配置」与否一律以 `GET /api/admin/providers` 的真实返回为准。
 */
export type PresetProtocol = 'openai' | 'anthropic'

export type PresetTab = 'cloud' | 'local'

export interface PresetProvider {
  preset_id: string
  display_name: string
  vendor: string
  tab: PresetTab
  protocol: PresetProtocol
  /** 目录里已知的站点地址；一个 preset 可以对应多个站点。 */
  sites: string[]
}

export interface PresetVendor {
  vendor: string
  tab: PresetTab
}

export const CUSTOM_PRESET_ID = 'custom'

export const PRESET_VENDORS: PresetVendor[] = [
  { vendor: 'OpenAI 系', tab: 'cloud' },
  { vendor: '阿里云', tab: 'cloud' },
  { vendor: 'DeepSeek', tab: 'cloud' },
  { vendor: 'Kimi', tab: 'cloud' },
  { vendor: '智谱 GLM', tab: 'cloud' },
  { vendor: '硅基流动', tab: 'cloud' },
  { vendor: 'Groq', tab: 'cloud' },
  { vendor: 'Google Gemini', tab: 'cloud' },
  { vendor: '本地', tab: 'local' },
]

export const PRESET_PROVIDERS: PresetProvider[] = [
  {
    preset_id: 'openai',
    display_name: 'OpenAI',
    vendor: 'OpenAI 系',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://api.openai.com/v1'],
  },
  {
    preset_id: 'anthropic',
    display_name: 'Anthropic',
    vendor: 'OpenAI 系',
    tab: 'cloud',
    protocol: 'anthropic',
    sites: ['https://api.anthropic.com/v1'],
  },
  {
    preset_id: 'openrouter',
    display_name: 'OpenRouter',
    vendor: 'OpenAI 系',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://openrouter.ai/api/v1'],
  },
  {
    preset_id: 'aliyun',
    display_name: '阿里云百炼',
    vendor: '阿里云',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://dashscope.aliyuncs.com/compatible-mode/v1'],
  },
  {
    preset_id: 'deepseek',
    display_name: 'DeepSeek',
    vendor: 'DeepSeek',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://api.deepseek.com/v1'],
  },
  {
    preset_id: 'moonshot',
    display_name: 'Kimi',
    vendor: 'Kimi',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://api.moonshot.cn/v1'],
  },
  {
    preset_id: 'zhipu',
    display_name: '智谱 GLM',
    vendor: '智谱 GLM',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://open.bigmodel.cn/api/paas/v4'],
  },
  {
    preset_id: 'siliconflow',
    display_name: '硅基流动',
    vendor: '硅基流动',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://api.siliconflow.cn/v1'],
  },
  {
    preset_id: 'groq',
    display_name: 'Groq',
    vendor: 'Groq',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://api.groq.com/openai/v1'],
  },
  {
    preset_id: 'gemini',
    display_name: 'Google Gemini',
    vendor: 'Google Gemini',
    tab: 'cloud',
    protocol: 'openai',
    sites: ['https://generativelanguage.googleapis.com/v1beta/openai'],
  },
  {
    preset_id: 'ollama',
    display_name: 'Ollama（本地）',
    vendor: '本地',
    tab: 'local',
    protocol: 'openai',
    sites: ['http://localhost:11434/v1'],
  },
]

export function presetVendorsOf(tab: PresetTab): PresetVendor[] {
  return PRESET_VENDORS.filter((entry) => entry.tab === tab)
}

export function presetProvidersOf(vendor: string): PresetProvider[] {
  return PRESET_PROVIDERS.filter((entry) => entry.vendor === vendor)
}

export function findPresetProvider(presetId: string): PresetProvider | undefined {
  return PRESET_PROVIDERS.find((entry) => entry.preset_id === presetId)
}

export function presetSiteCount(preset: PresetProvider): number {
  return preset.sites.length
}

/** 目录里已知站点对应的默认地址；用户可以在表单里改。 */
export function presetDefaultBaseUrl(preset: PresetProvider): string {
  return preset.sites[0] ?? ''
}
