/**
 * Octop 预设专家库（数据抽取）。
 *
 * 出处：TencentCloud/Octop @ eb28011249c02cafd389b2d424294c6c1b9cf422
 * 许可：MIT, Copyright (c) 2026 Octop（原文见 .octop-ref/octop/LICENSE）
 * 本文件所有文案均逐字来自 Octop 源 manifest.json 与人格 markdown，未改写、未补全。
 * 只抽了 preset 元数据 + 人格正文；Octop 的 skills/ 子目录本轮不接（quill 还没有技能引擎）。
 *
 * 排除 default：Octop 自己的专家库列表不展示它——catalog.py::list_summaries() 显式
 * ``` if ex_id == "default": continue ```（.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:852-853），
 * default 是内部默认助手，不是可安装的预设专家。故此处为 17 条。
 */

/**
 * 人格正文长度上限，与服务端 crates/quill-agent/src/expert.rs 的
 * `pub const MAX_INSTRUCTIONS: usize = 20_000;` 同口径：按**字符**（chars().count()）算，不是字节。
 * 超过该值服务端会判 ExpertInstructionsInvalid。
 */
export const LIBRARY_MAX_INSTRUCTIONS_CHARS = 20_000

export interface LibrarySource {
  repo: string
  pin: string
  license: string
  extractedAt: string
  /** 被排除的预设 id 及原因。 */
  excluded: readonly { id: string; reason: string }[]
}

export const LIBRARY_SOURCE: LibrarySource = {
  repo: 'https://github.com/TencentCloud/Octop',
  pin: 'eb28011249c02cafd389b2d424294c6c1b9cf422',
  license: 'MIT, Copyright (c) 2026 Octop',
  extractedAt: '2026-10-05',
  excluded: [
    {
      id: 'default',
      reason:
        'Octop 的专家库列表不展示 default：catalog.py::list_summaries() 显式 if ex_id == "default": continue' +
        '（.octop-ref/octop/src/octop/infra/agents/experts/catalog.py:852-853），它是内部默认助手，不是可安装的预设专家。',
    },
  ],
}

export interface LibraryQuickPrompt {
  titleZh: string
  titleEn: string
  descriptionZh: string
  descriptionEn: string
  promptZh: string
  promptEn: string
  color: string
  iconName: string
}

export interface LibraryExpert {
  id: string
  labelZh: string
  labelEn: string
  descriptionZh: string
  descriptionEn: string
  welcomeZh: string
  welcomeEn: string
  iconName: string
  color: string
  quickPrompts: readonly LibraryQuickPrompt[]
  taskExamplesZh: readonly string[]
  taskExamplesEn: readonly string[]
  /** 拼好的系统提示正文：按 instructionsFiles 顺序拼接，每段前有 `<!-- 文件名 -->` 标记，原文保留。 */
  instructions: string
  /** 实际参与拼接的源文件名（prompt_files 中磁盘上存在的那些，按 manifest 声明顺序）。 */
  instructionsFiles: readonly string[]
  /** instructions 的实测字符数。 */
  instructionsChars: number
  /** true = manifest 声明的 prompt_files 在源盘上缺失，instructions 为空串。 */
  personaMissing: boolean
  /** personaMissing 时的中文说明。 */
  personaMissingNote?: string
  /** true = instructions 字符数超过 LIBRARY_MAX_INSTRUCTIONS_CHARS，服务端会拒绝。原文不截断。 */
  oversized: boolean
}

// 人格正文：Vite ?raw 逐个显式导入，3.x 的 .md 以字符串进入本模块。
import generalAssistantSOUL from './library/general-assistant/SOUL.md?raw'
import aiCodingCoachSOUL from './library/ai-coding-coach/SOUL.md?raw'
import aiSafetyGuardianSOUL from './library/ai-safety-guardian/SOUL.md?raw'
import clinicalLearningSubscriptionSOUL from './library/clinical-learning-subscription/SOUL.md?raw'
import clinicalLearningSubscriptionAGENTS from './library/clinical-learning-subscription/AGENTS.md?raw'
import clinicalLearningSubscriptionBOOTSTRAP from './library/clinical-learning-subscription/BOOTSTRAP.md?raw'
import cvmAiDoctorIDENTITY from './library/cvm-ai-doctor/IDENTITY.md?raw'
import cvmAiDoctorSOUL from './library/cvm-ai-doctor/SOUL.md?raw'
import cvmAiDoctorHEARTBEAT from './library/cvm-ai-doctor/HEARTBEAT.md?raw'
import cvmClusterDoctorIDENTITY from './library/cvm-cluster-doctor/IDENTITY.md?raw'
import cvmClusterDoctorSOUL from './library/cvm-cluster-doctor/SOUL.md?raw'
import cvmClusterDoctorHEARTBEAT from './library/cvm-cluster-doctor/HEARTBEAT.md?raw'
import karpathyKnowledgeBaseSOUL from './library/karpathy-knowledge-base/SOUL.md?raw'
import karpathyKnowledgeBaseAGENTS from './library/karpathy-knowledge-base/AGENTS.md?raw'
import karpathyKnowledgeBaseBOOTSTRAP from './library/karpathy-knowledge-base/BOOTSTRAP.md?raw'
import meituanLivingAssistantSOUL from './library/meituan-living-assistant/SOUL.md?raw'
import multiAgentOrchestratorSOUL from './library/multi-agent-orchestrator/SOUL.md?raw'
import multiAgentOrchestratorAGENTS from './library/multi-agent-orchestrator/AGENTS.md?raw'
import newsTrendSOUL from './library/news-trend/SOUL.md?raw'
import newsTrendIDENTITY from './library/news-trend/IDENTITY.md?raw'
import newsTrendHEARTBEAT from './library/news-trend/HEARTBEAT.md?raw'
import officeAutomationSOUL from './library/office-automation/SOUL.md?raw'
import officeAutomationIDENTITY from './library/office-automation/IDENTITY.md?raw'
import opsEngineerSOUL from './library/ops-engineer/SOUL.md?raw'
import opsEngineerIDENTITY from './library/ops-engineer/IDENTITY.md?raw'
import parentingCompanionSOUL from './library/parenting-companion/SOUL.md?raw'
import parentingCompanionIDENTITY from './library/parenting-companion/IDENTITY.md?raw'
import parentingCompanionBOOTSTRAP from './library/parenting-companion/BOOTSTRAP.md?raw'
import parentingCompanionHEARTBEAT from './library/parenting-companion/HEARTBEAT.md?raw'
import stockAssistantSOUL from './library/stock-assistant/SOUL.md?raw'
import stockAssistantIDENTITY from './library/stock-assistant/IDENTITY.md?raw'
import stockAssistantHEARTBEAT from './library/stock-assistant/HEARTBEAT.md?raw'
import superpowersMethodologySOUL from './library/superpowers-methodology/SOUL.md?raw'
import tencentcloudApiSOUL from './library/tencentcloud-api/SOUL.md?raw'
import wechatOpsSOUL from './library/wechat-ops/SOUL.md?raw'
import wechatOpsIDENTITY from './library/wechat-ops/IDENTITY.md?raw'

/**
 * 排序口径照抄 Octop：`summaries.sort(key=lambda s: (0 if s.id == "general-assistant" else 1, s.id))`
 * （catalog.py:857）——general-assistant 置顶，其余按 id 升序。
 */
export const LIBRARY_EXPERTS: readonly LibraryExpert[] = [
  {
    id: "general-assistant",
    labelZh: "小通 · 通用助手",
    labelEn: "Main · General Assistant",
    descriptionZh: "友好的通用助手，无特定技能，适合作为起点或日常对话。",
    descriptionEn: "A friendly, no-skills default assistant — the safe starting point.",
    welcomeZh: "无论工作还是生活，说出你的想法",
    welcomeEn: "Share what's on your mind — work or life",
    iconName: "zap",
    color: "#6366f1",
    quickPrompts: [
    {
      titleZh: "总结长文",
      titleEn: "Summarize a document",
      descriptionZh: "粘贴文章或报告，提炼核心要点和结论",
      descriptionEn: "Paste an article or report and extract key points",
      promptZh: "请帮我总结以下内容，提炼出核心要点和结论：\n\n",
      promptEn: "Please summarize the following content and extract the key points and conclusions:\n\n",
      color: "#e8f4ff",
      iconName: "file-text",
    },
    {
      titleZh: "写一封邮件",
      titleEn: "Draft an email",
      descriptionZh: "告诉我收件人和目的，我来起草正式邮件",
      descriptionEn: "Tell me the recipient and purpose, and I'll draft the email",
      promptZh: "请帮我写一封邮件。收件人：，目的：，语气要求：正式/友好。",
      promptEn: "Please draft an email. Recipient:, purpose:, tone: formal/friendly.",
      color: "#fef3c7",
      iconName: "mail",
    },
    {
      titleZh: "解释代码",
      titleEn: "Explain code",
      descriptionZh: "粘贴代码片段，逐行解读逻辑和功能",
      descriptionEn: "Paste a code snippet and I'll walk through the logic",
      promptZh: "请逐步解释以下代码的逻辑和功能：\n\n```\n\n```",
      promptEn: "Please explain the logic and behavior of the following code step by step:\n\n```\n\n```",
      color: "#f1f5f9",
      iconName: "cpu",
    },
    {
      titleZh: "制定计划",
      titleEn: "Make a plan",
      descriptionZh: "描述目标，我来拆分成可执行的任务清单",
      descriptionEn: "Describe a goal and I'll break it into actionable tasks",
      promptZh: "请帮我制定一个详细的执行计划，目标是：",
      promptEn: "Please help me create a detailed action plan. The goal is:",
      color: "#dcfce7",
      iconName: "list-todo",
    },
    {
      titleZh: "翻译润色",
      titleEn: "Translate & polish",
      descriptionZh: "中英互译，或改写成更地道、更流畅的表达",
      descriptionEn: "Translate or rewrite text into more natural language",
      promptZh: "请将以下内容翻译成英文，并确保表达自然流畅：\n\n",
      promptEn: "Please translate the following into natural, fluent English:\n\n",
      color: "#f3e8ff",
      iconName: "globe",
    },
    {
      titleZh: "头脑风暴",
      titleEn: "Brainstorm ideas",
      descriptionZh: "给我一个主题，发散出多个创意和解决思路",
      descriptionEn: "Give me a topic and I'll brainstorm creative directions",
      promptZh: "请围绕以下主题进行头脑风暴，给出 5～8 个有创意的方向或想法：\n\n",
      promptEn: "Please brainstorm 5–8 creative directions or ideas around this topic:\n\n",
      color: "#fff1f2",
      iconName: "sparkles",
    },
    {
      titleZh: "起草与编辑",
      titleEn: "Draft & edit",
      descriptionZh: "描述写作需求，或粘贴内容让我润色",
      descriptionEn: "Describe a writing task or paste text to improve",
      promptZh: "请帮我撰写或改进以下内容：\n\n",
      promptEn: "Please help me write or improve the following:\n\n",
      color: "#fdf4e7",
      iconName: "pen-tool",
    },
    {
      titleZh: "深度研究",
      titleEn: "Deep research",
      descriptionZh: "提出问题，我来给出全面深入的解答",
      descriptionEn: "Ask a question and get a thorough answer",
      promptZh: "请对以下主题进行深入研究，给出全面的概述：\n\n",
      promptEn: "Please research the following topic in depth and give a comprehensive overview:\n\n",
      color: "#e8fdf5",
      iconName: "book-open",
    },
    {
      titleZh: "随便问问",
      titleEn: "Ask anything",
      descriptionZh: "有任何问题都可以直接问我",
      descriptionEn: "Ask me anything",
      promptZh: "我有一个问题想请教你：",
      promptEn: "I have a question for you:",
      color: "#eef2ff",
      iconName: "message-square",
    },
    {
      titleZh: "分析数据",
      titleEn: "Analyze data",
      descriptionZh: "粘贴表格或数据，帮你解读规律和趋势",
      descriptionEn: "Paste data and I'll find patterns and trends",
      promptZh: "请帮我分析以下数据，找出关键规律、趋势或异常，并给出结论：\n\n",
      promptEn: "Please analyze the following data, identify patterns, trends, or anomalies, and summarize conclusions:\n\n",
      color: "#e0f7f4",
      iconName: "trending-up",
    },
    {
      titleZh: "制作 PPT 大纲",
      titleEn: "PPT outline",
      descriptionZh: "告诉我主题，帮你规划演示文稿结构",
      descriptionEn: "Give me a topic and I'll outline a presentation",
      promptZh: "请帮我制作一份 PPT 演示文稿的大纲，主题是：",
      promptEn: "Please create a presentation outline. The topic is:",
      color: "#fff7ed",
      iconName: "presentation",
    },
    {
      titleZh: "优化简历",
      titleEn: "Polish a résumé",
      descriptionZh: "粘贴简历内容，帮你润色和强化亮点",
      descriptionEn: "Paste your résumé and I'll strengthen the highlights",
      promptZh: "请帮我优化以下简历内容，使其更专业、更有吸引力，并突出核心优势：\n\n",
      promptEn: "Please improve the following résumé to be more professional and highlight key strengths:\n\n",
      color: "#fdf2f8",
      iconName: "user",
    },
    ],
    taskExamplesZh: ["每个工作日「09:00」列出今日待办并按优先级排序，任务创建后立即启用","每周日「20:00」复盘本周完成事项，并给出下周计划","每天「22:00」提醒我整理明天要处理的三件事，从今天开始持续生效"],
    taskExamplesEn: ["Every weekday at 09:00, list today's todos ranked by priority — enable immediately","Every Sunday at 20:00, recap what got done this week and outline next week's plan","Every day at 22:00, remind me to pick the three things to handle tomorrow — start today and keep running"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  generalAssistantSOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 939,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "ai-coding-coach",
    labelZh: "AI 编程实战导师",
    labelEn: "AI Coding Coach",
    descriptionZh: "源自 ai-coding-guide 的 66 个 Claude Code 技巧与 9+ 款工具最佳实践，教你如何把 AI 编程工具（Claude Code / Cursor / Codex / Copilot / Aider 等）用出生产力：上下文管理、任务分解、提示词、调试、测试、安全与代码审查。",
    descriptionEn: "From ai-coding-guide: 66 Claude Code techniques + 9+ tools best practices — context management, task decomposition, prompting, debugging, testing, security, code review.",
    welcomeZh: "我是 AI 编程实战导师，告诉我你用的工具和你卡住的地方",
    welcomeEn: "I'm your AI Coding Coach — tell me your tool and what's blocking you",
    iconName: "code",
    color: "#3B82F6",
    quickPrompts: [
    {
      titleZh: "上下文管理技巧",
      titleEn: "Context management",
      descriptionZh: "让 AI 在长任务里不丢失关键信息",
      descriptionEn: "Keep AI focused across long tasks",
      promptZh: "我在长任务里总感觉 AI 越聊越乱、遗漏上下文。请给我一套上下文管理的具体技巧（CLAUDE.md、子代理、文件引用、压缩策略等）。",
      promptEn: "Give me concrete context-management techniques for long AI coding sessions.",
      color: "#dbeafe",
      iconName: "layers",
    },
    {
      titleZh: "任务分解与提示词",
      titleEn: "Decompose & prompt",
      descriptionZh: "把大需求拆成 AI 好执行的小步",
      descriptionEn: "Break a big ask into AI-friendly steps",
      promptZh: "请教我如何把一个大功能需求分解成 AI 容易执行的小步骤，并写出高质量的任务提示词（目标清晰、约束明确、验收可测）。",
      promptEn: "Teach me to decompose a big feature into AI-friendly steps with high-quality prompts.",
      color: "#e8f4ff",
      iconName: "split",
    },
    {
      titleZh: "工具选型建议",
      titleEn: "Tool selection",
      descriptionZh: "我该用哪款 AI 编程工具",
      descriptionEn: "Which AI coding tool fits",
      promptZh: "我想选一款 AI 编程工具，请基于我的场景（团队/个人、语言、是否要 agent 自主执行）对比 Claude Code / Cursor / Codex / Copilot / Aider 等的取舍。",
      promptEn: "Compare Claude Code / Cursor / Codex / Copilot / Aider for my scenario.",
      color: "#fef3c7",
      iconName: "git-compare",
    },
    ],
    taskExamplesZh: ["每个工作日「09:30」推送一条可立即练习的 AI 编程技巧（结合我常用的 Cursor / Claude Code），任务创建后立即启用","每周五「17:30」复盘本周我在 AI 编程上卡住的问题，给出下周练习计划","每月最后一天「20:00」整理一份本月工具与提示词最佳实践清单"],
    taskExamplesEn: ["Every weekday at 09:30, send one AI coding technique I can practice immediately (Cursor / Claude Code) — enable immediately","Every Friday at 17:30, recap where I got stuck with AI coding this week and outline next week's drills","On the last day of each month at 20:00, compile this month's tool and prompting best-practices list"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  aiCodingCoachSOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 1283,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "ai-safety-guardian",
    labelZh: "AI 安全合规卫士",
    labelEn: "AI Safety Guardian",
    descriptionZh: "基于 shellward 8 层防御与中文合规（PIPL / 等保 2.0 / 数据出境 / 生成式 AI 标识 GB 45438）的 AI 安全专家。负责安全体检、危险命令拦截、敏感数据出境识别与合规审计。",
    descriptionEn: "AI safety expert built on shellward's 8-layer defense and China compliance (PIPL / MLPS 2.0 / data export / GENAI labeling GB 45438).",
    welcomeZh: "我是 AI 安全合规卫士，告诉我你想做的安全体检或合规检查",
    welcomeEn: "I'm the AI Safety Guardian — tell me what safety check or compliance review you need",
    iconName: "shield",
    color: "#EF4444",
    quickPrompts: [
    {
      titleZh: "全量安全体检",
      titleEn: "Full safety scan",
      descriptionZh: "按 8 层防御做一遍安全与合规体检",
      descriptionEn: "Run an 8-layer safety & compliance scan",
      promptZh: "请按 8 层防御（promptGuard / outputScanner / toolBlocker / inputAuditor / securityGate / outboundGuard / dataFlowGuard / sessionGuard）帮我做一次 AI 工具安全体检，并给出加固建议。",
      promptEn: "Please run an 8-layer safety scan and give hardening advice.",
      color: "#fee2e2",
      iconName: "shield",
    },
    {
      titleZh: "命令安全检查",
      titleEn: "Check a command",
      descriptionZh: "检查某条命令是否危险",
      descriptionEn: "Check if a command is dangerous",
      promptZh: "请检查这条命令是否触发 toolBlocker 危险规则（rm -rf / curl|sh / 反弹 shell / fork bomb 等），并说明风险与更安全的替代。",
      promptEn: "Check whether this command triggers dangerous-command rules.",
      color: "#fef3c7",
      iconName: "terminal",
    },
    {
      titleZh: "数据出境合规",
      titleEn: "Data export check",
      descriptionZh: "识别敏感数据是否会被发往境外大模型",
      descriptionEn: "Detect sensitive data sent to overseas LLMs",
      promptZh: "请帮我识别：当前任务中是否有敏感数据可能被发往境外大模型端点（数据出境风险），并给出境内路由 / 脱敏建议（依据 PIPL 与数据出境规定）。",
      promptEn: "Identify data-export risk to overseas LLM endpoints per PIPL.",
      color: "#dbeafe",
      iconName: "globe",
    },
    ],
    taskExamplesZh: ["每天「09:00」做一次轻量安全体检：危险命令、敏感文件和环境变量泄露风险，任务创建后立即启用","每周一「10:00」扫描工作区是否有密钥或个人信息落地，列出待处理项","每月 1 日「09:30」做一次合规检查摘要（PIPL / 数据出境 / 生成式 AI 标识）"],
    taskExamplesEn: ["Every day at 09:00, run a light safety check for dangerous commands, sensitive files, and leaked env vars — enable immediately","Every Monday at 10:00, scan the workspace for secrets or personal data on disk and list follow-ups","On the 1st of each month at 09:30, send a compliance summary (PIPL / data export / generative-AI labeling)"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  aiSafetyGuardianSOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 1621,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "clinical-learning-subscription",
    labelZh: "基层医生学习小助手",
    labelEn: "Primary Care Doctor Learning Assistant",
    descriptionZh: "专注医学指南学习，擅长查找权威信源的指南信息、搭建学习计划、分学科推送指南总结、备考选材、医保政策解读与信源核验等，并兼具写作、总结、翻译、办公等通用能力。",
    descriptionEn: "A general-purpose assistant with specialist support for primary-care guideline learning diagnostics, section expansion, study pathways, exam-oriented material selection, continuous learning, insurance-policy study, and source verification.",
    welcomeZh: "我是基层医生学习与通用助手。基于权威指南做医学指南学习、政策摘要，也可回答症状/疾病等医学问答（附来源链接），并处理写作、总结、翻译、编程、计划等日常任务——通用能力不会因为医学模块而受限。不提供个体诊疗、处方剂量；急诊处置与医保报销只提供指南/政策层面的学习性建议，不作为执行或报销依据。需要个性化医学学习或订阅时，再征得同意登记最小化的医生信息。",
    welcomeEn: "I am a grassroots doctor learning and general assistant. I provide medical guideline learning and policy summaries based on authoritative guidelines, and can also answer medical questions like symptoms/diseases (with source links), as well as handle writing, summarization, translation, programming, planning and other daily tasks—general capabilities are not limited by the medical module. I do not provide individual diagnosis or prescription dosages; emergency management and medical insurance reimbursement only provide guideline/policy-level learning suggestions, not as execution or reimbursement basis. When personalized medical learning or subscription is needed, I will seek consent to register minimal doctor information.",
    iconName: "book-open-check",
    color: "#0f766e",
    quickPrompts: [
    {
      titleZh: "登记学习订阅",
      titleEn: "Register Subscription",
      descriptionZh: "填写5项信息，生成学习订阅配置",
      descriptionEn: "Register 5 fields and create learning subscription settings",
      promptZh: "我同意处理以下信息用于指南学习订阅。请登记：\n称谓（可用姓氏/昵称）：\n所在地区：省 / 市 / 区县\n所在医院：\n所在科室：\n当前职称：",
      promptEn: "I consent to using the following information for learning subscriptions. Please register:\nPreferred name or title:\nRegion: province / city / district\nHospital:\nDepartment:\nTitle:",
      color: "#ccfbf1",
      iconName: "user-check",
    },
    {
      titleZh: "预览下一学习单元",
      titleEn: "Preview Next Lesson",
      descriptionZh: "查看固定版本轨道的下一单元，不计入进度",
      descriptionEn: "Preview the next fixed-version lesson without advancing progress",
      promptZh: "请读取我的学习轨道并预览下一学习单元。不要创建投递账本、不要推进进度；如有多条轨道，先让我选择。",
      promptEn: "Please read my learning tracks and preview the next lesson. Do not create a delivery ledger or advance progress; ask me to choose if multiple tracks are active.",
      color: "#dbeafe",
      iconName: "book-open",
    },
    {
      titleZh: "今日指南学习",
      titleEn: "Today's Guideline Lesson",
      descriptionZh: "生成今日正式指南学习单元（走弱投递防重）",
      descriptionEn: "Generate today formal guideline lesson (weak-delivery dedup)",
      promptZh: "请按当前边界生成今日指南学习内容：先 delivery-check 查重，再读取下一固定单元，校验后输出正文。",
      promptEn: "Please generate today guideline lesson within current boundaries: delivery-check for dedup first, then read next fixed lesson, validate, and output.",
      color: "#dbeafe",
      iconName: "book-open",
    },
    {
      titleZh: "继续当前指南",
      titleEn: "Continue Guideline",
      descriptionZh: "按当前进度继续同一份指南的下一天学习",
      descriptionEn: "Continue the same guideline for next day",
      promptZh: "请读取学习档案中的当前连续学习指南和进度，继续生成下一天的指南学习内容。若尚未选择指南，请先推荐一份适合我科室的权威指南，并在我确认后记录进度。",
      promptEn: "Please read current guideline and progress from learning profile, then generate next day learning content. If no guideline selected, recommend one authoritative guideline for my department and record progress after I confirm.",
      color: "#e0f2fe",
      iconName: "book-marked",
    },
    {
      titleZh: "指南学习诊断",
      titleEn: "Guideline Learning Diagnostic",
      descriptionZh: "根据学习目标、指南版本和掌握情况定位优先学习内容",
      descriptionEn: "Identify learning priorities from a goal, guideline version, and current knowledge.",
      promptZh: "我想做一次指南学习诊断。学习目标：；准备学习的指南/主题：；当前掌握情况：；每天可投入时间：。请先核验指南来源，再给出学习诊断、优先章节和下一步计划。",
      promptEn: "I want a guideline learning diagnostic. Learning goal:; guideline/topic:; current familiarity:; minutes available per day:. Please verify the source first, then provide the learning diagnostic, priority sections, and next-step plan.",
      color: "#e0f2fe",
      iconName: "clipboard-check",
    },
    {
      titleZh: "指南学习地图",
      titleEn: "Guideline Learning Map",
      descriptionZh: "将指南按章节、概念关系和学习顺序整理成可展开的学习路径",
      descriptionEn: "Turn a guideline into an expandable path by chapter, concept relationship, and study order.",
      promptZh: "请把以下指南或主题整理成学习地图：先核验版本和权威来源，再给出章节结构、概念流程关系、优先学习顺序和可继续展开的部分。只用于学习，不针对具体患者作诊断或处置：\n\n",
      promptEn: "Please turn the following guideline or topic into a learning map: verify its version and authoritative source first, then provide chapter structure, concept-flow relationships, priority order, and expandable sections. This is for learning only, not patient-specific diagnosis or care:\n\n",
      color: "#dcfce7",
      iconName: "map",
    },
    {
      titleZh: "展开指南章节",
      titleEn: "Expand Guideline Section",
      descriptionZh: "按原文结构逐节讲解并给出原文定位，帮你读懂原文",
      descriptionEn: "Walk through a section following the original structure with precise locators.",
      promptZh: "请展开以下指南的指定章节：先核验完整名称、版本和权威来源，再按原文结构讲解本节要点、关键定义、证据等级、与上下游章节的关系和易混淆点，并给出原文定位（章/节/条目号）。只做学习性解读，不替代原文，也不针对具体患者：\n\n",
      promptEn: "Please expand the specified section of this guideline: verify its full title, version, and authoritative source first, then explain the section's key points, definitions, evidence grades, relationships to adjacent chapters, and common confusions, with precise locators. Educational reading support only, not a replacement for the original text and not patient-specific:\n\n",
      color: "#e0f2fe",
      iconName: "book-open-text",
    },
    {
      titleZh: "指南学习路径图",
      titleEn: "Guideline Study Pathway",
      descriptionZh: "把指南整理成学习顺序：每步对应原文、用时和掌握标志",
      descriptionEn: "Turn a guideline into a study order with locators, time, and mastery checks.",
      promptZh: "请把以下指南整理成学习路径图：先核验版本和权威来源，再给出前置知识、分步学习顺序（每步写明对应原文章节、预计用时、掌握标志）和可选深入方向。我要的是学习顺序，不是床旁处置步骤：\n\n",
      promptEn: "Please turn this guideline into a study pathway: verify version and authoritative source first, then give prerequisites, step-by-step study order (each with the matching section, estimated time, and mastery check), and optional deep-dive directions. I want a study order, not bedside management steps:\n\n",
      color: "#dcfce7",
      iconName: "route",
    },
    {
      titleZh: "按考试目标推荐教材",
      titleEn: "Exam-Oriented Materials",
      descriptionZh: "说明备考目标，动态推荐教材、指南与学习顺序",
      descriptionEn: "Recommend materials and study order based on your exam or role goal.",
      promptZh: "我正在准备以下考试／岗位目标，请据此推荐学习材料和顺序：区分官方大纲要求与你的补充建议，标注每份材料的版本、发布机构和用途，并说明以当年官方考试大纲为准。\n\n考试／岗位目标：\n目标时间：\n每日可投入时间：\n专业方向：",
      promptEn: "I am preparing for the following exam or role goal. Please recommend study materials and an order: separate official syllabus requirements from your own suggestions, note each item's version, publisher, and purpose, and state that the current official syllabus prevails.\n\nExam or role goal:\nTarget date:\nDaily study time:\nSpecialty:",
      color: "#fef9c3",
      iconName: "graduation-cap",
    },
    {
      titleZh: "医保政策回顾学习",
      titleEn: "Insurance Policy Review",
      descriptionZh: "回顾近一年或指定区间的重点医保政策，标注现行状态",
      descriptionEn: "Review key insurance policies over the past year or a chosen window.",
      promptZh: "请回顾我所在地区近一年的重点医保政策：写明回顾区间和检索层级，按发布时间排序，标注每份文件的发布机构、生效时间和现行状态（现行有效／已废止／已被替代），并按主题归纳学习提示。只做原文摘要，不要给报销结论或支付比例。",
      promptEn: "Please review the key insurance policies for my region over the past year: state the review window and search levels, sort by publication date, note each document's publisher, effective date, and current status (in force / repealed / superseded), and group study notes by theme. Summarize official text only, without reimbursement conclusions or payment ratios.",
      color: "#ede9fe",
      iconName: "history",
    },
    {
      titleZh: "专业指南更新提醒",
      titleEn: "Guideline Update",
      descriptionZh: "提醒本科室相关指南/共识/规范更新",
      descriptionEn: "Summarize department-relevant professional updates",
      promptZh: "请按当前边界生成专业指南更新提醒：",
      promptEn: "Please prepare a professional guideline update reminder within the current boundaries:",
      color: "#ede9fe",
      iconName: "bell-ring",
    },
    {
      titleZh: "医保政策变化学习",
      titleEn: "Insurance Summary",
      descriptionZh: "学习地区医保正式文件变化，不给报销结论",
      descriptionEn: "Summarize regional insurance policy text",
      promptZh: "请按当前边界生成地区医保政策变化学习内容：",
      promptEn: "Please prepare regional insurance policy learning content within the current boundaries:",
      color: "#fef3c7",
      iconName: "file-text",
    },
    {
      titleZh: "创建学习轨道",
      titleEn: "Create Learning Track",
      descriptionZh: "核验版本后规划固定章节单元，确认后再保存和启用",
      descriptionEn: "Verify a version, plan fixed lesson units, then save and activate only after confirmation",
      promptZh: "请为我设计一个候选学习轨道：先核验指南版本和权威来源，再给出固定章节单元、每次时长和关联学习目标。先不要保存、启用或推送，等我确认。",
      promptEn: "Design a candidate learning track: verify the guideline version and authoritative source, then propose fixed lessons, pacing, and learning goals. Do not save, activate, or send anything until I confirm.",
      color: "#dcfce7",
      iconName: "list-checks",
    },
    {
      titleZh: "查看学习轨道进度",
      titleEn: "View Learning Track Progress",
      descriptionZh: "查看固定版本、已确认送达单元和下一单元",
      descriptionEn: "View fixed version, accepted lessons, and the next lesson",
      promptZh: "请读取我的学习轨道，展示每条轨道的固定版本、已确认送达单元、我标记的学习状态和下一单元。不要把已送达说成已掌握，也不要自动推送。",
      promptEn: "Read my learning tracks and show each fixed version, accepted lessons, my marked learning status, and the next lesson. Do not equate delivery with mastery or send automatically.",
      color: "#e0f2fe",
      iconName: "book-marked",
    },
    {
      titleZh: "检查指南新版迁移",
      titleEn: "Review Guideline Migration",
      descriptionZh: "比较旧轨道与候选新版，确认前不改写任何进度",
      descriptionEn: "Compare an existing track with a candidate version without changing progress before confirmation",
      promptZh: "请检查我的学习轨道是否有可核验的新版本指南；若有，先给出旧/新版本、受影响章节和迁移方案。未经我确认，不要替换轨道、改写进度或发送内容。",
      promptEn: "Check whether any learning track has a verifiable newer guideline version. If so, show old/new versions, affected lessons, and a migration plan. Do not replace tracks, alter progress, or send content without my confirmation.",
      color: "#fef3c7",
      iconName: "git-compare",
    },
    {
      titleZh: "核验来源链接",
      titleEn: "Verify Source",
      descriptionZh: "判断指南、政策或通告来源能否作为依据",
      descriptionEn: "Check whether a guideline or policy source is reliable",
      promptZh: "请按信源核验规则检查以下来源是否可作为最终依据，并说明适用层级、发布时间、发布机构和不能确认的地方：\n\n",
      promptEn: "Please verify whether the following source can be used as final evidence, and explain its applicable level, publication date, issuing body, and any uncertainties:\n\n",
      color: "#f0fdf4",
      iconName: "shield-check",
    },
    {
      titleZh: "更新登记信息",
      titleEn: "Update Profile",
      descriptionZh: "变更地区、医院、科室或职称",
      descriptionEn: "Update region, hospital, department, or title",
      promptZh: "我需要更新学习订阅登记信息。请一次只问一个需要变更的字段，并继续遵守最小化隐私原则。",
      promptEn: "I need to update my learning subscription profile. Please ask for one field at a time and keep following the data minimization principle.",
      color: "#f5f3ff",
      iconName: "settings",
    },
    {
      titleZh: "停用学习订阅",
      titleEn: "Pause Subscription",
      descriptionZh: "停用推送任务或清除订阅档案",
      descriptionEn: "Pause scheduled pushes or clear the subscription profile",
      promptZh: "请帮我停用学习订阅。先列出当前可停用的订阅任务和学习档案中会保留/清除的信息，等我确认后再执行。",
      promptEn: "Please help me pause the learning subscription. First list the scheduled tasks that can be disabled and what information in my learning profile will be kept or cleared, then wait for my confirmation before taking action.",
      color: "#fee2e2",
      iconName: "pause-circle",
    },
    {
      titleZh: "处理其他任务",
      titleEn: "Handle Another Task",
      descriptionZh: "直接处理写作、总结、翻译、计划、数据整理等日常事务",
      descriptionEn: "Handle everyday work such as writing, summaries, translation, planning, or data organization.",
      promptZh: "请作为通用助手帮我处理这件事：",
      promptEn: "Please help me with this as a general assistant:",
      color: "#f3e8ff",
      iconName: "sparkles",
    },
    ],
    taskExamplesZh: ["每个工作日「07:30」按我的学习轨道推送当天指南学习单元，任务创建后立即启用","每周日「20:00」复盘本周学习进度，给出下周章节建议","每月 1 日「08:00」检查当前指南是否有新版；若有则预览迁移影响，先不改轨道","每个工作日「21:00」根据今日学习单元出 3 道自测题，不给诊疗建议","每周三「07:30」推送一条医保政策学习摘要（仅政策层面）","每周五「21:00」整理本周学习笔记索引，不改原文、不给诊疗建议"],
    taskExamplesEn: ["Every weekday at 07:30, push today's guideline learning unit from my track — enable immediately","Every Sunday at 20:00, recap this week's learning progress and suggest next week's sections","On the 1st of each month at 08:00, check whether the current guideline has a new edition; if so, preview migration impact without changing the track yet","Every weekday at 21:00, write 3 self-check questions from today's unit — no clinical advice","Every Wednesday at 07:30, push one insurance-policy learning brief (policy level only)","Every Friday at 21:00, index this week's study notes — no rewrites and no clinical advice"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  clinicalLearningSubscriptionSOUL,
  '<!-- AGENTS.md -->\n' +
  clinicalLearningSubscriptionAGENTS,
  '<!-- BOOTSTRAP.md -->\n' +
  clinicalLearningSubscriptionBOOTSTRAP,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","AGENTS.md","BOOTSTRAP.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 8145,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "cvm-ai-doctor",
    labelZh: "系统医生",
    labelEn: "System Doctor",
    descriptionZh: "系统医生 — 持续监控和诊断本机操作系统的健康状况,包括 CPU/内存/磁盘资源、系统服务状态、网络连通性、磁盘健康、系统日志分析等。快速定位系统级问题并提供可执行的修复方案。",
    descriptionEn: "System Doctor — continuously monitors and diagnoses the health of the host operating system, including CPU/memory/disk resources, system service status, network connectivity, disk health, system log analysis. Quickly locates system-level issues and provides actionable remediation.",
    welcomeZh: "描述系统症状，我来帮你做健康检查",
    welcomeEn: "Describe system symptoms and I'll run a health check",
    iconName: "cpu",
    color: "#0052D9",
    quickPrompts: [
    {
      titleZh: "系统健康检查",
      titleEn: "System health check",
      descriptionZh: "全面检查 CPU、内存、磁盘与关键服务状态",
      descriptionEn: "Check CPU, memory, disk, and critical services",
      promptZh: "请对本机做一次全面的系统健康检查，列出资源使用、异常服务和需要关注的问题。",
      promptEn: "Please run a full system health check on this host and list resource usage, abnormal services, and issues to watch.",
      color: "#e8f4ff",
      iconName: "activity",
    },
    {
      titleZh: "分析系统日志",
      titleEn: "Analyze system logs",
      descriptionZh: "定位最近错误与告警根因",
      descriptionEn: "Find root causes in recent errors and alerts",
      promptZh: "请分析最近的系统日志，找出错误、告警和可能的根因，并给出修复建议。",
      promptEn: "Please analyze recent system logs, identify errors, alerts, and likely root causes, with remediation steps.",
      color: "#fef3c7",
      iconName: "file-text",
    },
    {
      titleZh: "磁盘空间排查",
      titleEn: "Disk space triage",
      descriptionZh: "找出占用空间的大文件与目录",
      descriptionEn: "Find large files and directories consuming disk space",
      promptZh: "请帮我排查磁盘空间使用情况，找出占用最多的目录和文件，并建议清理方案。",
      promptEn: "Please investigate disk usage, find the largest directories and files, and suggest cleanup options.",
      color: "#dcfce7",
      iconName: "hard-drive",
    },
    {
      titleZh: "网络连通诊断",
      titleEn: "Network connectivity",
      descriptionZh: "检查端口、DNS 与外网连通性",
      descriptionEn: "Check ports, DNS, and external connectivity",
      promptZh: "请诊断本机网络连通性，包括 DNS、关键端口和外网访问，并说明异常点。",
      promptEn: "Please diagnose network connectivity on this host, including DNS, key ports, and external access.",
      color: "#f1f5f9",
      iconName: "globe",
    },
    {
      titleZh: "性能瓶颈分析",
      titleEn: "Performance bottleneck",
      descriptionZh: "定位 CPU、内存、IO 等资源瓶颈",
      descriptionEn: "Find CPU, memory, and I/O bottlenecks",
      promptZh: "请分析本机当前的性能瓶颈，检查 CPU、内存、磁盘 IO 和网络，找出占用最高的进程并给出优化建议。",
      promptEn: "Please analyze performance bottlenecks on this host — CPU, memory, disk I/O, and network — identify top processes and suggest optimizations.",
      color: "#eef2ff",
      iconName: "gauge",
    },
    {
      titleZh: "服务异常排查",
      titleEn: "Service failure triage",
      descriptionZh: "检查 systemd、Docker 等关键服务",
      descriptionEn: "Check systemd, Docker, and other critical services",
      promptZh: "请排查本机关键服务的运行状态，包括 systemd 服务、Docker 容器和端口监听，找出异常服务并给出修复步骤。",
      promptEn: "Please investigate critical services on this host — systemd units, Docker containers, and port listeners — find failures and provide remediation steps.",
      color: "#fff1f2",
      iconName: "wrench",
    },
    ],
    taskExamplesZh: ["每天「09:00」做系统健康检查（CPU / 内存 / 磁盘 / 关键服务），有异常再通知我，任务创建后立即启用","每「2 小时」检查磁盘使用率和系统负载，超过阈值立即告警，从今天开始持续生效","每周一「09:30」汇总上周系统错误日志和修复建议","每个工作日「18:00」复盘当天告警，区分已恢复和仍异常的项","每月 1 日「09:30」输出容量趋势：磁盘、内存和近期增长","每周五「16:00」检查关键服务重启记录，列出反复异常的项"],
    taskExamplesEn: ["Every day at 09:00, run a system health check (CPU / memory / disk / key services) and notify me only on anomalies — enable immediately","Every 2 hours, check disk usage and load; alert immediately if thresholds are exceeded, starting today","Every Monday at 09:30, summarize last week's system error logs and suggested fixes","Every weekday at 18:00, recap today's alerts and separate recovered items from still-open ones","On the 1st of each month at 09:30, send a capacity trend: disk, memory, and recent growth","Every Friday at 16:00, review key-service restarts and list recurring faults"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- IDENTITY.md -->\n' +
  cvmAiDoctorIDENTITY,
  '<!-- SOUL.md -->\n' +
  cvmAiDoctorSOUL,
  '<!-- HEARTBEAT.md -->\n' +
  cvmAiDoctorHEARTBEAT,
].join('\n\n'),
    instructionsFiles: ["IDENTITY.md","SOUL.md","HEARTBEAT.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 7429,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "cvm-cluster-doctor",
    labelZh: "集群医生",
    labelEn: "Cluster Doctor",
    descriptionZh: "集群医生 — 专注腾讯云 CVM 集群级别的健康巡检、多节点关联分析、风险门控修复。通过腾讯云 TAT 无需 SSH 即可批量执行 OS 诊断，支持集群评分（木桶原理）、5 种跨节点关联模式、串行安全修复。",
    descriptionEn: "Cluster Doctor — focuses on Tencent Cloud CVM cluster-level health patrol, multi-node anomaly correlation, and risk-gated remediation. Executes OS diagnostics in bulk via TAT without SSH, supports cluster scoring (weakest-link principle), 5 cross-node correlation patterns, and serial-safe remediation.",
    welcomeZh: "描述集群症状，我来帮你巡检和诊断",
    welcomeEn: "Describe cluster symptoms and I'll help patrol and diagnose",
    iconName: "server",
    color: "#006EFF",
    quickPrompts: [
    {
      titleZh: "集群健康巡检",
      titleEn: "Cluster health patrol",
      descriptionZh: "30 秒内完成所有节点健康快照",
      descriptionEn: "30-second health snapshot across all nodes",
      promptZh: "请对当前 CVM 集群执行一次快速健康巡检，汇总所有节点的实例状态、云端监控和 OS 实时数据。",
      promptEn: "Please run a quick health patrol on the current CVM cluster, summarizing instance status, cloud metrics, and OS data for all nodes.",
      color: "#e8f4ff",
      iconName: "activity",
    },
    {
      titleZh: "集群健康评分",
      titleEn: "Cluster health score",
      descriptionZh: "木桶原理评分，找出短板节点",
      descriptionEn: "Weakest-link scoring to find bottleneck nodes",
      promptZh: "请对集群进行健康评分（木桶原理），列出各节点得分、拖累整体的最差节点和改进建议。",
      promptEn: "Please score cluster health (weakest-link principle), list per-node scores, the worst node, and improvement suggestions.",
      color: "#fef3c7",
      iconName: "bar-chart-3",
    },
    {
      titleZh: "跨节点关联分析",
      titleEn: "Cross-node correlation",
      descriptionZh: "发现多节点间的关联异常模式",
      descriptionEn: "Detect correlated anomalies across nodes",
      promptZh: "请分析集群中是否存在跨节点关联异常，检查内存级联、负载不均、磁盘同步增长、网络退化或服务失效级联等模式。",
      promptEn: "Please analyze cross-node correlation patterns — memory cascade, load imbalance, disk growth sync, network degradation, or service failure cascade.",
      color: "#dcfce7",
      iconName: "network",
    },
    {
      titleZh: "批量 OS 诊断",
      titleEn: "Bulk OS diagnostics",
      descriptionZh: "通过 TAT 无需 SSH 批量执行诊断",
      descriptionEn: "Run diagnostics via TAT without SSH",
      promptZh: "请通过腾讯云 TAT 对集群所有节点批量执行 OS 级诊断，汇总 CPU、内存、磁盘和网络异常。",
      promptEn: "Please run bulk OS diagnostics on all cluster nodes via Tencent Cloud TAT and summarize CPU, memory, disk, and network issues.",
      color: "#f1f5f9",
      iconName: "server",
    },
    {
      titleZh: "风险门控修复",
      titleEn: "Risk-gated remediation",
      descriptionZh: "串行安全修复，逐台验证恢复",
      descriptionEn: "Serial-safe remediation with per-node verification",
      promptZh: "请根据当前集群异常制定风险门控修复方案，按风险等级串行执行，每次最多操作一台并验证恢复后再继续。",
      promptEn: "Please draft a risk-gated remediation plan for current cluster issues — serial execution, one node at a time, verify recovery before proceeding.",
      color: "#fff1f2",
      iconName: "shield-check",
    },
    {
      titleZh: "实例生命周期",
      titleEn: "Instance lifecycle",
      descriptionZh: "安全地重启、停止或启动 CVM 实例",
      descriptionEn: "Safely restart, stop, or start CVM instances",
      promptZh: "请帮我安全地管理 CVM 实例生命周期（重启/停止/启动），先说明影响范围和风险，再给出操作步骤。",
      promptEn: "Please help manage CVM instance lifecycle (restart/stop/start) — explain impact and risks first, then provide safe steps.",
      color: "#eef2ff",
      iconName: "refresh-cw",
    },
    ],
    taskExamplesZh: ["每天「09:00」对腾讯云 CVM 集群做健康巡检，按木桶原理打分并指出最弱节点，任务创建后立即启用","每个工作日「18:00」汇总当天跨节点异常关联，列出需人工确认的修复项","每周一「10:00」输出集群风险周报：容量、故障与待修复项","每「4 小时」检查节点 CPU / 磁盘 / 网络异常，超过阈值再通知我","每月 1 日「10:00」对照木桶短板给出扩容或下线建议，先不执行","每周五「16:00」核对集群变更与告警是否对齐，标出未关闭项"],
    taskExamplesEn: ["Every day at 09:00, patrol the Tencent Cloud CVM cluster, score it by the weakest-link rule, and name the weakest node — enable immediately","Every weekday at 18:00, summarize cross-node correlations from today and list remediations that need confirmation","Every Monday at 10:00, send a cluster risk weekly: capacity, incidents, and open fixes","Every 4 hours, check node CPU / disk / network anomalies and notify me only above threshold","On the 1st of each month at 10:00, suggest scale-up or retirement from the weakest-link gaps — do not execute","Every Friday at 16:00, check cluster changes against alerts and flag still-open items"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- IDENTITY.md -->\n' +
  cvmClusterDoctorIDENTITY,
  '<!-- SOUL.md -->\n' +
  cvmClusterDoctorSOUL,
  '<!-- HEARTBEAT.md -->\n' +
  cvmClusterDoctorHEARTBEAT,
].join('\n\n'),
    instructionsFiles: ["IDENTITY.md","SOUL.md","HEARTBEAT.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 9937,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "karpathy-knowledge-base",
    labelZh: "卡帕西知识库专家",
    labelEn: "Karpathy Knowledge Base",
    descriptionZh: "基于 Karpathy LLM Wiki 思路维护本地 Markdown 知识库：原始资料保持不变，AI 持续编译相互链接的 Wiki，并通过 MEMORY.md 热索引、完整目录和日志让知识随每次摄取与研究不断积累。",
    descriptionEn: "Maintains a local Markdown knowledge base using the Karpathy LLM Wiki pattern: immutable raw sources, an interlinked AI-maintained wiki, and a compact MEMORY.md index that compounds across ingests and research.",
    welcomeZh: "把资料交给我，或直接提出研究问题。我会先读 MEMORY.md，再从 Wiki 与原始资料中查找，并把值得长期保留的结论编译回知识库。",
    welcomeEn: "Give me a source or ask a research question. I read MEMORY.md first, navigate the wiki and raw sources, and compile durable findings back into the knowledge base.",
    iconName: "book-open",
    color: "#7c3aed",
    quickPrompts: [
    {
      titleZh: "初始化知识主题",
      titleEn: "Initialize a topic",
      descriptionZh: "确定知识库范围、目标和首批来源",
      descriptionEn: "Define scope, goals, and initial sources",
      promptZh: "请和我一起初始化这个知识库。主题是：；我希望长期回答的问题是：；首批资料是：。先检查现有 MEMORY.md 和 Wiki，再给出初始化方案。",
      promptEn: "Help me initialize this knowledge base. Topic:; long-term questions:; initial sources:. Check MEMORY.md and the existing wiki first, then propose the setup.",
      color: "#ede9fe",
      iconName: "book-open",
    },
    {
      titleZh: "摄取新资料",
      titleEn: "Ingest a source",
      descriptionZh: "读取一个来源并更新相关 Wiki 页面",
      descriptionEn: "Read one source and update related wiki pages",
      promptZh: "请把以下资料摄取到知识库：。先确认来源身份和范围，再摘要、交叉链接并更新索引与日志。",
      promptEn: "Ingest this source into the knowledge base:. Confirm its identity and scope, then summarize, cross-link, and update the indexes and log.",
      color: "#dbeafe",
      iconName: "file-text",
    },
    {
      titleZh: "查询知识库",
      titleEn: "Query the wiki",
      descriptionZh: "基于已编译知识与原始来源回答",
      descriptionEn: "Answer from compiled knowledge and raw sources",
      promptZh: "请基于当前知识库回答这个问题：。先从 MEMORY.md 和 index.md 定位，再引用具体 Wiki 页面与原始来源。",
      promptEn: "Answer this question from the current knowledge base:. Start with MEMORY.md and index.md, then cite the relevant wiki pages and raw sources.",
      color: "#dcfce7",
      iconName: "search",
    },
    {
      titleZh: "运行知识库体检",
      titleEn: "Lint the knowledge base",
      descriptionZh: "检查冲突、陈旧内容、断链和索引漂移",
      descriptionEn: "Find conflicts, stale claims, broken links, and index drift",
      promptZh: "请对整个知识库做一次 lint：检查冲突、过期主张、孤立页面、断链、缺失来源和 MEMORY.md/index.md 漂移；先报告，再执行安全修复。",
      promptEn: "Lint the knowledge base for contradictions, stale claims, orphan pages, broken links, missing sources, and MEMORY.md/index.md drift. Report first, then apply safe fixes.",
      color: "#fef3c7",
      iconName: "list-checks",
    },
    {
      titleZh: "整理主题地图",
      titleEn: "Build a topic map",
      descriptionZh: "把分散页面组织成概念关系和研究缺口",
      descriptionEn: "Organize pages into concepts, relationships, and gaps",
      promptZh: "请为这个主题整理一张知识地图：。标出核心概念、实体、关键来源、相互关系、冲突和待研究问题，并把有长期价值的结果归档到 Wiki。",
      promptEn: "Build a knowledge map for this topic:. Show core concepts, entities, sources, relationships, conflicts, and research gaps, then file durable results into the wiki.",
      color: "#e0f2fe",
      iconName: "network",
    },
    ],
    taskExamplesZh: ["每天「21:00」对知识库做一次 lint：断链、孤立页、无来源主张，只报告不自动大改，任务创建后立即启用","每周日「20:00」根据 MEMORY.md 活跃主题生成一周研究摘要","每个工作日「09:30」提醒我今日待摄取的来源，并更新 MEMORY 索引","每周三「21:00」把本周新增笔记按主题归入索引，不改原文","每月 1 日「20:00」盘点孤立页与重复主题，给出合并建议但不自动改","每周五「21:00」根据本周摄取来源更新 MEMORY 待办，不改正文"],
    taskExamplesEn: ["Every day at 21:00, lint the knowledge base for broken links, orphan pages, and unsourced claims — report only, no large auto-edits; enable immediately","Every Sunday at 20:00, write a weekly research digest from the active topics in MEMORY.md","Every weekday at 09:30, remind me of sources waiting to ingest and refresh the MEMORY index","Every Wednesday at 21:00, file this week's new notes into the index by topic without rewriting them","On the 1st of each month at 20:00, inventory orphan pages and duplicate topics and suggest merges — no auto-edits","Every Friday at 21:00, refresh MEMORY todos from this week's sources — do not rewrite notes"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  karpathyKnowledgeBaseSOUL,
  '<!-- AGENTS.md -->\n' +
  karpathyKnowledgeBaseAGENTS,
  '<!-- BOOTSTRAP.md -->\n' +
  karpathyKnowledgeBaseBOOTSTRAP,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","AGENTS.md","BOOTSTRAP.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 2539,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "meituan-living-assistant",
    labelZh: "美团生活助手",
    labelEn: "Meituan Living Assistant",
    descriptionZh: "美团生活优惠助手 — 一键领取美团各品类优惠券、领红包，搜索附近团购美食（火锅、烧烤、日料、咖啡、奶茶等）并完成下单，支持每日定时自动领券，省钱省心。",
    descriptionEn: "Meituan living deals assistant — one-click coupon collection, nearby group-buy food search and ordering, daily deals across dining and lifestyle services.",
    welcomeZh: "想吃什么喝什么？或者直接说「帮我领美团优惠券」",
    welcomeEn: "Craving something? Or just say \"help me get Meituan coupons\"",
    iconName: "utensils",
    color: "#ffc300",
    quickPrompts: [
    {
      titleZh: "领美团优惠券",
      titleEn: "Get Meituan coupons",
      descriptionZh: "一键领取美团各品类优惠券和红包",
      descriptionEn: "Collect all types of Meituan coupons with one tap",
      promptZh: "帮我领美团优惠券",
      promptEn: "Help me get Meituan coupons",
      color: "#fff7e6",
      iconName: "sparkles",
    },
    {
      titleZh: "附近好吃的",
      titleEn: "What's good nearby?",
      descriptionZh: "搜索附近团购美食，选好直接下单",
      descriptionEn: "Search nearby group-buy food and place an order",
      promptZh: "附近有什么好吃的？",
      promptEn: "What's good to eat nearby?",
      color: "#e8f4ff",
      iconName: "globe",
    },
    {
      titleZh: "今日优惠活动",
      titleEn: "Today's deals",
      descriptionZh: "看看今天有什么优惠活动",
      descriptionEn: "Explore today's deals and promotions",
      promptZh: "今天有什么优惠活动？",
      promptEn: "What deals are available today?",
      color: "#e8fdf5",
      iconName: "trending-up",
    },
    {
      titleZh: "每日定时领券",
      titleEn: "Daily auto coupons",
      descriptionZh: "设置每天定时自动领券，到点自动执行",
      descriptionEn: "Schedule automatic daily coupon collection",
      promptZh: "帮我设置每天定时自动领券",
      promptEn: "Set up daily automatic coupon collection",
      color: "#fdf4e7",
      iconName: "bell",
    },
    {
      titleZh: "附近好喝的",
      titleEn: "Good drinks nearby",
      descriptionZh: "搜索附近咖啡、奶茶等饮品，选好直接下单",
      descriptionEn: "Find nearby coffee, milk tea and drinks, then order",
      promptZh: "附近有什么好喝的？",
      promptEn: "What's good to drink nearby?",
      color: "#f3e8ff",
      iconName: "coffee",
    },
    {
      titleZh: "团购下单",
      titleEn: "Place an order",
      descriptionZh: "选好附近团购美食，直接帮你下单支付",
      descriptionEn: "Pick nearby group-buy deals and place an order",
      promptZh: "帮我下单一份附近的美食团购",
      promptEn: "Help me order a nearby group-buy deal",
      color: "#ffe8f0",
      iconName: "shopping-bag",
    },
    ],
    taskExamplesZh: ["每天「09:00」自动领取美团各品类优惠券和红包，领完后把结果发给我，任务创建后立即启用","每个工作日「11:30」根据我附近推荐今日午餐团购，列出 3 个高性价比选项","每周五「18:00」汇总本周省了多少、哪些券快过期","每天「17:30」根据天气和位置推荐晚餐外卖，列出 3 个选项","每周日「10:00」扫描本周可订的周末休闲/出行团购","每月 1 日「09:00」汇总上月优惠使用和过期券，给出本月领取提醒"],
    taskExamplesEn: ["Every day at 09:00, collect Meituan coupons and red packets across categories and send me the result — enable immediately","Every weekday at 11:30, recommend 3 high-value nearby lunch group-buys","Every Friday at 18:00, summarize how much I saved this week and which coupons expire soon","Every day at 17:30, recommend 3 dinner takeout options from weather and location","Every Sunday at 10:00, scan weekend leisure or travel group-buys still bookable this week","On the 1st of each month at 09:00, recap last month's coupon use and expiries, and remind me what to claim"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  meituanLivingAssistantSOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 714,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "multi-agent-orchestrator",
    labelZh: "多专家协作编排",
    labelEn: "Multi-Agent Orchestrator",
    descriptionZh: "源自 agency-orchestrator 的 DAG 协作理念：把一句话需求拆成有依赖关系的步骤，匹配最合适的子智能体角色，按 depends_on 串联执行并接力输出。让 OCTOP 从单 agent 升级为多专家协作。",
    descriptionEn: "From agency-orchestrator: turn one sentence into a dependency-ordered DAG, match each step to the best sub-agent role, and chain outputs via depends_on.",
    welcomeZh: "我是协作编排师，给我一个目标，我把它拆给合适的专家去跑",
    welcomeEn: "I'm the orchestrator — give me a goal and I'll decompose it across the right experts",
    iconName: "git-branch",
    color: "#0EA5E9",
    quickPrompts: [
    {
      titleZh: "拆解一个目标",
      titleEn: "Decompose a goal",
      descriptionZh: "把复杂目标编成 DAG 并派发",
      descriptionEn: "Turn a complex goal into a DAG and dispatch",
      promptZh: "请帮我把这个目标拆成有依赖关系的步骤，为每一步匹配最合适的子智能体角色（引用其 slug），写出 depends_on 与 output 接力，形成可执行的协作计划。",
      promptEn: "Decompose this goal into a dependency DAG, assign each step a sub-agent role, and chain outputs.",
      color: "#e0f2fe",
      iconName: "workflow",
    },
    {
      titleZh: "组建专家团",
      titleEn: "Form an expert team",
      descriptionZh: "为项目锁定一组固定阵容",
      descriptionEn: "Lock a fixed roster for a project",
      promptZh: "这是一个持续项目，请为我锁定一组固定的子智能体阵容（如 架构师+后端+前端+测试+运维），说明各自职责与协作顺序，便于反复复用。",
      promptEn: "Lock a reusable sub-agent roster for this ongoing project.",
      color: "#fef3c7",
      iconName: "users",
    },
    ],
    taskExamplesZh: ["每个工作日「09:30」检查进行中的多专家协作计划，汇报卡在哪一步，任务创建后立即启用","每周五「17:00」汇总本周已完成的协作任务与遗留依赖","每天「18:00」若有未关闭的 DAG 步骤，提醒我确认是否继续或换角"],
    taskExamplesEn: ["Every weekday at 09:30, inspect in-flight multi-expert plans and report which step is blocked — enable immediately","Every Friday at 17:00, summarize completed collaboration tasks and leftover dependencies","Every day at 18:00, if any DAG step is still open, ask me whether to continue or reassign the role"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  multiAgentOrchestratorSOUL,
  '<!-- AGENTS.md -->\n' +
  multiAgentOrchestratorAGENTS,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","AGENTS.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 3187,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "news-trend",
    labelZh: "热点记者",
    labelEn: "Hot News Reporter",
    descriptionZh: "热点新闻趋势跟踪 — 帮你获取来自微博、知乎、虎扑等 70+ 平台的热点新闻，筛选最有价值的热点，结合你的偏好进行个性化分析和总结。",
    descriptionEn: "Real-time news analyst — continuously tracking multi-source news hotspots, generating daily briefings, and providing trend insights with in-depth analysis.",
    welcomeZh: "想看哪些平台的热点，或需要今日资讯摘要？",
    welcomeEn: "Which hotspots to track, or need today's news briefing?",
    iconName: "trending-up",
    color: "#3498db",
    quickPrompts: [
    {
      titleZh: "今日热点早报",
      titleEn: "Today's news briefing",
      descriptionZh: "综合多平台热点，生成今日资讯摘要",
      descriptionEn: "Multi-platform hotspots in a daily briefing",
      promptZh: "请帮我生成今日热点早报，综合微博、知乎、百度等平台的热搜，筛选最有价值的内容并附上来源。",
      promptEn: "Please generate today's news briefing from Weibo, Zhihu, Baidu and other platforms — filter the most valuable items with sources.",
      color: "#e8f4ff",
      iconName: "newspaper",
    },
    {
      titleZh: "微博知乎热搜",
      titleEn: "Weibo & Zhihu trends",
      descriptionZh: "查看指定平台实时热搜榜单",
      descriptionEn: "Real-time trending lists from specific platforms",
      promptZh: "请获取微博和知乎当前的热搜榜单，列出前 10 条并简要说明每条为什么值得关注。",
      promptEn: "Please fetch current trending lists from Weibo and Zhihu, show top 10 items and briefly explain why each matters.",
      color: "#fef3c7",
      iconName: "flame",
    },
    {
      titleZh: "科技圈动态",
      titleEn: "Tech news roundup",
      descriptionZh: "AI、编程、互联网最新资讯",
      descriptionEn: "Latest AI, programming, and internet news",
      promptZh: "请汇总今天科技圈的最新动态，重点关注 AI、大模型、编程和互联网领域的热点新闻。",
      promptEn: "Please summarize today's tech news, focusing on AI, LLMs, programming, and internet industry hotspots.",
      color: "#dcfce7",
      iconName: "cpu",
    },
    {
      titleZh: "财经资讯速览",
      titleEn: "Finance news snapshot",
      descriptionZh: "股市、宏观、行业财经热点",
      descriptionEn: "Stock market, macro, and industry finance news",
      promptZh: "请获取今天的财经热点资讯，涵盖 A 股市场、宏观经济和行业重大新闻，并标注信息来源。",
      promptEn: "Please fetch today's finance hotspots — A-share market, macro economy, and major industry news with sources.",
      color: "#f1f5f9",
      iconName: "trending-up",
    },
    {
      titleZh: "金价汇率查询",
      titleEn: "Gold & forex rates",
      descriptionZh: "实时金价、汇率等金融数据",
      descriptionEn: "Real-time gold prices and exchange rates",
      promptZh: "请查询当前金价和主要货币汇率（美元、欧元、日元等），给出最新数据和简要走势说明。",
      promptEn: "Please check current gold prices and major exchange rates (USD, EUR, JPY, etc.) with a brief trend summary.",
      color: "#fff7ed",
      iconName: "coins",
    },
    {
      titleZh: "深度趋势分析",
      titleEn: "Deep trend analysis",
      descriptionZh: "多源交叉验证，深挖热点脉络",
      descriptionEn: "Cross-source verification and trend deep-dive",
      promptZh: "请对今天最重要的 3 个热点进行深度分析，交叉验证多个信息源，梳理事件脉络和后续可能影响。",
      promptEn: "Please deeply analyze today's top 3 hotspots — cross-verify sources, trace the narrative, and assess potential impact.",
      color: "#f3e8ff",
      iconName: "search",
    },
    ],
    taskExamplesZh: ["每天「08:00」推送今日多平台热点早报，精选 10 条并附简评，任务创建后立即启用","每个工作日「18:00」复盘白天热点变化，标出持续升温的话题","每周日「21:00」做一周热点趋势盘点，列出值得深读的 5 个话题","每个交易日「09:30」扫描财经与科技交叉热点，标出可能影响市场的突发","每天「12:00」推送午间快讯，只保留当天新增的 5 条","每周五「19:00」整理本周被多次验证的热点，写成一份周末阅读清单"],
    taskExamplesEn: ["Every day at 08:00, push a 10-item multi-platform hotspot briefing with short comments — enable immediately","Every weekday at 18:00, recap how daytime hotspots shifted and flag topics that are still heating up","Every Sunday at 21:00, review the week's trend and list 5 topics worth a deeper read","Every trading day at 09:30, scan finance/tech crossover hotspots and flag market-moving surprises","Every day at 12:00, push a midday bulletin with only the 5 new items from today","Every Friday at 19:00, compile this week's multiply-verified hotspots into a weekend reading list"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  newsTrendSOUL,
  '<!-- IDENTITY.md -->\n' +
  newsTrendIDENTITY,
  '<!-- HEARTBEAT.md -->\n' +
  newsTrendHEARTBEAT,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md","HEARTBEAT.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 7805,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "office-automation",
    labelZh: "小办 · 办公自动化",
    labelEn: "Auto · Office Automation",
    descriptionZh: "办公自动化助手「小办」— 内置 Word、Excel、PPT、PDF 全套文档技能，支持表格清洗与分析、文档编辑排版、演示文稿制作、PDF 提取与填表；还能整理会议纪要、生成周报日报、读取本地文件和获取办公资讯。",
    descriptionEn: "Office automation assistant 'Auto' — bundled Word, Excel, PPT, and PDF skills for spreadsheet analysis, document editing, presentation creation, and PDF extraction/forms; plus meeting minutes, weekly reports, file reading, and office news briefings.",
    welcomeZh: "把文档、表格或办公任务发给我，Word / Excel / PPT / PDF 都能搞定",
    welcomeEn: "Send me documents, spreadsheets, or office tasks — Word, Excel, PPT, PDF, all covered",
    iconName: "briefcase",
    color: "#7c3aed",
    quickPrompts: [
    {
      titleZh: "Excel 数据处理",
      titleEn: "Excel data processing",
      descriptionZh: "清洗、汇总、透视、制图和分析表格",
      descriptionEn: "Clean, summarize, pivot, chart, and analyze spreadsheets",
      promptZh: "请帮我处理以下 Excel/表格文件，完成清洗、汇总或分析（说明具体需求和文件路径）：\n\n",
      promptEn: "Please process the following Excel/spreadsheet file — clean, summarize, or analyze (describe needs and file path):\n\n",
      color: "#dcfce7",
      iconName: "table",
    },
    {
      titleZh: "编辑 Word 文档",
      titleEn: "Edit Word document",
      descriptionZh: "创建、编辑 .docx，含目录、页眉页脚",
      descriptionEn: "Create or edit .docx with TOC, headers, and formatting",
      promptZh: "请帮我处理以下 Word 文档需求（创建/编辑 .docx，说明文件路径和具体要求）：\n\n",
      promptEn: "Please help with this Word document task (create/edit .docx — file path and requirements):\n\n",
      color: "#e8f4ff",
      iconName: "file-text",
    },
    {
      titleZh: "制作 PPT 演示",
      titleEn: "Create presentation",
      descriptionZh: "从零创建或编辑 .pptx 演示文稿",
      descriptionEn: "Create or edit .pptx presentations from scratch",
      promptZh: "请帮我制作或编辑一份 PPT 演示文稿，主题是：___，要求：___。",
      promptEn: "Please create or edit a presentation. Topic: ___, requirements: ___.",
      color: "#fef3c7",
      iconName: "presentation",
    },
    {
      titleZh: "PDF 提取与处理",
      titleEn: "PDF extraction",
      descriptionZh: "提取文本/表格、合并拆分、填写表单",
      descriptionEn: "Extract text/tables, merge/split, fill forms",
      promptZh: "请帮我处理以下 PDF 文件（提取内容、合并拆分或填写表单，说明文件路径）：\n\n",
      promptEn: "Please process the following PDF — extract, merge/split, or fill forms (file path):\n\n",
      color: "#f1f5f9",
      iconName: "file",
    },
    {
      titleZh: "整理会议纪要",
      titleEn: "Meeting minutes",
      descriptionZh: "从笔记提炼结构化纪要",
      descriptionEn: "Turn notes into structured minutes",
      promptZh: "请帮我把以下会议内容整理成结构化纪要，包含议题、讨论要点、决议和待办事项：\n\n",
      promptEn: "Please organize the following meeting content into structured minutes — topics, discussion points, decisions, and action items:\n\n",
      color: "#eef2ff",
      iconName: "clipboard-list",
    },
    {
      titleZh: "生成周报日报",
      titleEn: "Weekly/daily report",
      descriptionZh: "根据工作记录生成规范报告",
      descriptionEn: "Generate formatted reports from work logs",
      promptZh: "请根据以下本周工作内容，生成一份结构清晰的周报（含完成事项、进行中、下周计划）：\n\n",
      promptEn: "Please generate a structured weekly report from the following work items (completed, in progress, next week plan):\n\n",
      color: "#f3e8ff",
      iconName: "calendar",
    },
    ],
    taskExamplesZh: ["每个工作日「18:00」根据当天会议记录起草日报，发给我确认，任务创建后立即启用","每周五「17:00」汇总本周待办与文档进展，生成周报大纲","每个月最后一个工作日「16:00」整理本月合同和表格待办清单发给我","每个工作日「09:00」把收件箱里待处理的邮件按优先级列成待办","每周一「10:00」根据日历起草本周会议纪要模板，先放草稿不发送","每天「12:30」把上午会议待办并入清单，标出今天下午要跟的项"],
    taskExamplesEn: ["Every weekday at 18:00, draft a daily report from today's meeting notes and send it for my review — enable immediately","Every Friday at 17:00, summarize this week's todos and document progress into a weekly-report outline","On the last weekday of each month at 16:00, compile this month's contract and spreadsheet follow-ups","Every weekday at 09:00, turn inbox items still needing action into a prioritized todo list","Every Monday at 10:00, draft this week's meeting-notes templates from the calendar — keep as drafts, do not send","Every day at 12:30, merge morning meeting follow-ups into the list and flag this afternoon's items"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  officeAutomationSOUL,
  '<!-- IDENTITY.md -->\n' +
  officeAutomationIDENTITY,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 2153,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "ops-engineer",
    labelZh: "运维工程师 Ops",
    labelEn: "Ops · SRE Engineer",
    descriptionZh: "精通 Linux 运维、Kubernetes、Docker、数据库、网络诊断的 Shell 命令专家，专为 AI 终端设计。",
    descriptionEn: "Shell command expert for Linux ops, Kubernetes, Docker, databases, and network diagnostics — designed for the AI Terminal.",
    welcomeZh: "告诉我你要排查的运维问题或想执行的命令",
    welcomeEn: "Tell me what ops issue to troubleshoot or which command to run",
    iconName: "terminal",
    color: "#16a34a",
    quickPrompts: [
    {
      titleZh: "K8s Pod 排障",
      titleEn: "Debug a K8s pod",
      descriptionZh: "查看 Pod 状态、事件与容器日志",
      descriptionEn: "Inspect pod status, events, and container logs",
      promptZh: "请给出排查 Kubernetes Pod 异常的 kubectl 命令，包括查看状态、事件、日志和进入容器调试。",
      promptEn: "Please provide kubectl commands to troubleshoot a failing Kubernetes pod, including status, events, logs, and exec debugging.",
      color: "#e8f4ff",
      iconName: "server",
    },
    {
      titleZh: "Docker 容器诊断",
      titleEn: "Diagnose Docker container",
      descriptionZh: "检查容器状态、日志与资源占用",
      descriptionEn: "Check container status, logs, and resource usage",
      promptZh: "请给出诊断 Docker 容器异常的命令，包括查看运行状态、日志、端口映射和资源使用。",
      promptEn: "Please provide Docker commands to diagnose a container issue, including status, logs, port mappings, and resource usage.",
      color: "#dcfce7",
      iconName: "wrench",
    },
    {
      titleZh: "磁盘与内存排查",
      titleEn: "Disk & memory triage",
      descriptionZh: "定位空间不足或内存飙高的原因",
      descriptionEn: "Find what's eating disk space or memory",
      promptZh: "请给出排查 Linux 磁盘空间和内存占用的命令，并说明如何找出占用最高的目录和进程。",
      promptEn: "Please provide Linux commands to investigate disk space and memory usage, and how to find the top directories and processes.",
      color: "#fef3c7",
      iconName: "hard-drive",
    },
    {
      titleZh: "服务与端口检查",
      titleEn: "Service & port check",
      descriptionZh: "确认进程监听、systemd 服务是否正常",
      descriptionEn: "Verify listeners and systemd service health",
      promptZh: "请给出检查 systemd 服务状态和端口监听的命令，帮我确认服务是否在运行、端口是否可达。",
      promptEn: "Please provide commands to check systemd service status and port listeners to verify the service is running and reachable.",
      color: "#f1f5f9",
      iconName: "activity",
    },
    {
      titleZh: "网络连通诊断",
      titleEn: "Network connectivity",
      descriptionZh: "DNS、路由与外网/内网连通测试",
      descriptionEn: "Test DNS, routing, and internal/external connectivity",
      promptZh: "请给出诊断网络连通问题的命令，包括 DNS 解析、路由追踪和端口连通测试。",
      promptEn: "Please provide commands to diagnose network connectivity, including DNS resolution, traceroute, and port checks.",
      color: "#eef2ff",
      iconName: "globe",
    },
    {
      titleZh: "写运维脚本",
      titleEn: "Write an ops script",
      descriptionZh: "生成可执行的 bash 巡检或批处理脚本",
      descriptionEn: "Generate a runnable bash patrol or batch script",
      promptZh: "请帮我写一个 bash 脚本，用于：",
      promptEn: "Please write a bash script for:",
      color: "#fdf4e7",
      iconName: "terminal",
    },
    ],
    taskExamplesZh: ["每天「09:00」巡检 Kubernetes 集群：异常 Pod、节点 NotReady、最近事件，有问题再通知我，任务创建后立即启用","每「4 小时」检查磁盘使用率和 inode，超过 80% 时立即告警，从今天开始持续生效","每周一「10:00」汇总上周故障与变更，列出待处理项发给我","每个工作日「18:00」复盘当天告警与变更窗口，标出未关闭工单","每周五「16:00」检查证书与密钥到期日，30 天内到期的列出来","每月 1 日「10:00」输出容量与成本摘要：节点、存储和本月费用"],
    taskExamplesEn: ["Every day at 09:00, inspect the Kubernetes cluster (bad pods, NotReady nodes, recent events) and notify me only if something is wrong — enable immediately","Every 4 hours, check disk usage and inodes; alert immediately if either exceeds 80%, starting today","Every Monday at 10:00, summarize last week's incidents and changes and send me the open items","Every weekday at 18:00, recap today's alerts and change windows and flag unclosed tickets","Every Friday at 16:00, check certificate and key expiry and list anything due within 30 days","On the 1st of each month at 10:00, send a capacity and cost brief: nodes, storage, and this month's bill"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  opsEngineerSOUL,
  '<!-- IDENTITY.md -->\n' +
  opsEngineerIDENTITY,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 3494,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "parenting-companion",
    labelZh: "育儿管家",
    labelEn: "Parenting Companion",
    descriptionZh: "陪伴孩子成长的 AI 育儿管家：记录疫苗、辅食、健康、里程碑，所有医疗与育儿建议必须带可验证来源。日记原汁原味——AI 不代笔。",
    descriptionEn: "An AI companion that grows with your child: tracks vaccines, feeding, health, and milestones. All medical advice ships with verified sources. Diaries kept verbatim — AI never ghostwrites.",
    welcomeZh: "记录宝宝的成长，或问我育儿与健康问题",
    welcomeEn: "Log your child's growth or ask parenting and health questions",
    iconName: "baby",
    color: "#f6c177",
    quickPrompts: [
    {
      titleZh: "记录疫苗接种",
      titleEn: "Log vaccination",
      descriptionZh: "记录接种时间、疫苗种类和反应",
      descriptionEn: "Record date, vaccine type, and reactions",
      promptZh: "请帮我记录一次疫苗接种：疫苗名称是___，接种日期是___，接种后有无不良反应：___。",
      promptEn: "Please log a vaccination: vaccine name ___, date ___, any adverse reactions: ___.",
      color: "#e8f4ff",
      iconName: "syringe",
    },
    {
      titleZh: "辅食排敏记录",
      titleEn: "Food introduction log",
      descriptionZh: "记录新食物尝试和过敏反应",
      descriptionEn: "Track new foods and allergic reactions",
      promptZh: "请帮我记录辅食新尝试：食物是___，吃了___量，有无过敏反应：___。",
      promptEn: "Please log a new food introduction: food ___, amount ___, any allergic reaction: ___.",
      color: "#dcfce7",
      iconName: "apple",
    },
    {
      titleZh: "发烧居家观察",
      titleEn: "Fever home care",
      descriptionZh: "记录体温并给出权威护理建议",
      descriptionEn: "Log temperature and provide sourced care advice",
      promptZh: "宝宝发烧了，当前体温___℃，其他症状：___。请记录并给出居家观察建议（需附权威来源）。",
      promptEn: "Baby has a fever, temperature ___°C, other symptoms: ___. Please log and provide home-care advice with authoritative sources.",
      color: "#fef3c7",
      iconName: "thermometer",
    },
    {
      titleZh: "成长里程碑",
      titleEn: "Development milestone",
      descriptionZh: "记录大动作、语言、认知新进展",
      descriptionEn: "Log motor, language, and cognitive milestones",
      promptZh: "请帮我记录一个成长里程碑：宝宝今天___（描述具体行为），大概月龄___。",
      promptEn: "Please log a development milestone: today baby ___ (describe the behavior), approximate age ___ months.",
      color: "#fdf4e7",
      iconName: "star",
    },
    {
      titleZh: "育儿知识查询",
      titleEn: "Parenting Q&A",
      descriptionZh: "权威来源验证的育儿健康问答",
      descriptionEn: "Parenting and health Q&A with verified sources",
      promptZh: "我有一个育儿问题想请教：___。请查阅权威来源后回答。",
      promptEn: "I have a parenting question: ___. Please answer after checking authoritative sources.",
      color: "#f3e8ff",
      iconName: "book-open",
    },
    {
      titleZh: "写成长日记",
      titleEn: "Growth diary entry",
      descriptionZh: "原汁原味记录成长瞬间",
      descriptionEn: "Record growth moments verbatim",
      promptZh: "请帮我把以下内容记入成长日记（一字不改）：",
      promptEn: "Please add the following to the growth diary verbatim:",
      color: "#fff1f2",
      iconName: "pen-line",
    },
    ],
    taskExamplesZh: ["每天「21:00」提醒我记录宝宝今日饮食、睡眠和异常，持续生效并立即启用","每周日「20:00」汇总本周发育与健康记录，对照档案给出注意事项","距离下次疫苗接种 7 天内，每天「09:00」提醒我预约和注意事项","每月孩子月龄日「20:00」汇总当月发育并对照 WHO 曲线，仅在偏好窗口内发送","每天「07:30」根据档案提醒今日辅食或过敏观察要点","每周五「21:00」整理本周日记条目索引，不改原文"],
    taskExamplesEn: ["Every day at 21:00, remind me to log today's feeding, sleep, and anything unusual — enable immediately and keep it running","Every Sunday at 20:00, summarize this week's growth and health notes against the profile","When the next vaccine is within 7 days, remind me daily at 09:00 to book it and review precautions","On the child's monthly age-day at 20:00, summarize growth vs WHO curves — send only in the preferred window","Every day at 07:30, remind me of today's feeding or allergy-watch notes from the profile","Every Friday at 21:00, index this week's diary entries without rewriting them"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  parentingCompanionSOUL,
  '<!-- IDENTITY.md -->\n' +
  parentingCompanionIDENTITY,
  '<!-- BOOTSTRAP.md -->\n' +
  parentingCompanionBOOTSTRAP,
  '<!-- HEARTBEAT.md -->\n' +
  parentingCompanionHEARTBEAT,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md","BOOTSTRAP.md","HEARTBEAT.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 6328,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "stock-assistant",
    labelZh: "老钱 · 证券观察员",
    labelEn: "Lao Qian · Market Observer",
    descriptionZh: "市场观察者「老钱」— 查行情、看基本面、做技术分析、追市场热点。以 A 股为主，兼顾港美股及全球市场，用老股民的风格给你说人话，严格合规不荐股。",
    descriptionEn: "Market observer 'Lao Qian' — real-time quotes, fundamental analysis, technical indicators (MA/MACD/RSI/BOLL), market trends. Primarily A-shares, also covers HK & US stocks and global markets. Veteran investor style, strictly compliant, no stock recommendations.",
    welcomeZh: "想了解哪只股票或今天的市场动向？",
    welcomeEn: "Which stock or market trend would you like to explore?",
    iconName: "candlestick-chart",
    color: "#e74c3c",
    quickPrompts: [
    {
      titleZh: "查个股行情",
      titleEn: "Stock quote lookup",
      descriptionZh: "实时报价、涨跌幅和成交量",
      descriptionEn: "Real-time price, change, and volume",
      promptZh: "请帮我查一下___（股票代码/名称）的最新行情，包括现价、涨跌幅、成交量和换手率。",
      promptEn: "Please look up the latest quote for ___ (ticker/name) — price, change %, volume, and turnover rate.",
      color: "#e8f4ff",
      iconName: "line-chart",
    },
    {
      titleZh: "板块资金流向",
      titleEn: "Sector fund flow",
      descriptionZh: "查看行业板块资金净流入排名",
      descriptionEn: "Sector net fund inflow rankings",
      promptZh: "请查询今天 A 股各板块的资金流向，列出净流入和净流出最多的板块。",
      promptEn: "Please check today's A-share sector fund flows — top net inflows and outflows.",
      color: "#dcfce7",
      iconName: "arrow-up-down",
    },
    {
      titleZh: "涨停跌停池",
      titleEn: "Limit-up/down pool",
      descriptionZh: "今日涨停、跌停股票一览",
      descriptionEn: "Today's limit-up and limit-down stocks",
      promptZh: "请查询今天 A 股的涨停池和跌停池，列出股票名称、代码和涨停/跌停原因（如有）。",
      promptEn: "Please fetch today's A-share limit-up and limit-down pools with names, tickers, and reasons if available.",
      color: "#fef3c7",
      iconName: "zap",
    },
    {
      titleZh: "技术面分析",
      titleEn: "Technical analysis",
      descriptionZh: "K 线、均线、MACD 等技术指标",
      descriptionEn: "Candlesticks, MA, MACD, and other indicators",
      promptZh: "请对___（股票代码）做技术面分析，包括近期 K 线走势、均线系统和 MACD/RSI 等指标。",
      promptEn: "Please run technical analysis on ___ — recent candlesticks, moving averages, MACD/RSI, etc.",
      color: "#f1f5f9",
      iconName: "candlestick-chart",
    },
    {
      titleZh: "基本面解读",
      titleEn: "Fundamental analysis",
      descriptionZh: "财报、估值、盈利能力分析",
      descriptionEn: "Financials, valuation, and profitability",
      promptZh: "请帮我分析___（股票代码）的基本面，包括最新财报要点、PE/PB 估值和盈利能力。",
      promptEn: "Please analyze fundamentals for ___ — latest earnings highlights, PE/PB valuation, and profitability.",
      color: "#eef2ff",
      iconName: "pie-chart",
    },
    {
      titleZh: "今日市场综述",
      titleEn: "Market overview",
      descriptionZh: "大盘走势、热点板块和市场情绪",
      descriptionEn: "Index moves, hot sectors, and market sentiment",
      promptZh: "请给我今天 A 股市场的综述，包括大盘指数表现、热点板块和市场整体情绪。",
      promptEn: "Please give me today's A-share market overview — index performance, hot sectors, and overall sentiment.",
      color: "#fff1f2",
      iconName: "bar-chart-3",
    },
    ],
    taskExamplesZh: ["设置「每个交易日 09:15」推送今日 A 股开盘综述（大盘、热点板块、北向资金），任务创建后立即启用","每个交易日「15:10」复盘收盘：指数涨跌、涨停跌停池和我关注的自选股，从今天开始持续生效","每周一「08:30」汇总上周板块资金流向和我自选股的技术面变化，发送给我","每个交易日「11:35」盘中快报：指数、成交额和我自选股异动","每个交易日「21:00」复盘美股与隔夜期货对 A 股的可能影响","每月最后一个交易日「16:00」汇总本月自选股涨跌和估值变化"],
    taskExamplesEn: ["Push today's A-share open briefing (index, hot sectors, northbound flow) every trading day at 09:15, and enable the job immediately","Every trading day at 15:10, recap the close: index moves, limit-up/down pool, and my watchlist — start today and keep it running","Every Monday at 08:30, summarize last week's sector fund flows and technical changes on my watchlist","Every trading day at 11:35, send a midday flash: index, turnover, and watchlist movers","Every trading day at 21:00, recap how US stocks and overnight futures may affect A-shares","On the last trading day of each month at 16:00, summarize this month's watchlist moves and valuation changes"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  stockAssistantSOUL,
  '<!-- IDENTITY.md -->\n' +
  stockAssistantIDENTITY,
  '<!-- HEARTBEAT.md -->\n' +
  stockAssistantHEARTBEAT,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md","HEARTBEAT.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 7637,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "superpowers-methodology",
    labelZh: "AI 工程方法论教练",
    labelEn: "AI Engineering Methodology Coach",
    descriptionZh: "源自 superpowers-zh 的 20 个工程技能，把探索→计划→TDD→调试→审查→验证的工作流内化为习惯，让 AI 写出的代码更可靠、更可维护。",
    descriptionEn: "Encodes superpowers-zh's 20 engineering skills into a reliable explore→plan→TDD→debug→review→verify workflow.",
    welcomeZh: "我是方法论教练，告诉我你要做的开发任务，我陪你走完可靠工程流",
    welcomeEn: "I'm your methodology coach — tell me the dev task and we'll run a reliable engineering flow",
    iconName: "zap",
    color: "#6366F1",
    quickPrompts: [
    {
      titleZh: "先写失败测试",
      titleEn: "Write a failing test first",
      descriptionZh: "用 TDD 方式开始一个功能",
      descriptionEn: "Start a feature test-first (TDD)",
      promptZh: "请按测试驱动开发（TDD）的方式帮我把这个功能落地：先写会失败的测试，再写最小实现让它通过，最后重构。",
      promptEn: "Help me build this feature test-first: write a failing test, then minimal implementation, then refactor.",
      color: "#ede9fe",
      iconName: "flask-conical",
    },
    {
      titleZh: "系统化调试",
      titleEn: "Systematic debugging",
      descriptionZh: "遇到 bug 先诊断再修",
      descriptionEn: "Diagnose before fixing a bug",
      promptZh: "这个功能出 bug 了。请按系统化调试：先建立可观察的失败复现，提出假设，用最小实验逐一排除，定位根因后再修，不要盲目改代码。",
      promptEn: "Debug systematically: reproduce, hypothesize, isolate, then fix — no blind edits.",
      color: "#fef3c7",
      iconName: "bug",
    },
    {
      titleZh: "写实现计划",
      titleEn: "Write an implementation plan",
      descriptionZh: "多步骤任务先写计划",
      descriptionEn: "Write a plan before a multi-step task",
      promptZh: "这是一个多步骤任务，请先写一份实现计划（目标、步骤、每步验收点、风险），确认后再动手。",
      promptEn: "Write an implementation plan with checkpoints before coding.",
      color: "#dbeafe",
      iconName: "list",
    },
    ],
    taskExamplesZh: ["每个工作日「09:30」提醒我今天的开发任务先写失败测试再动手，任务创建后立即启用","每周五「16:30」按 explore → plan → TDD → review 复盘本周一个功能","每天「18:00」检查是否有未完成的审查或验证步骤，列成明日待办"],
    taskExamplesEn: ["Every weekday at 09:30, remind me to write a failing test before coding today's task — enable immediately","Every Friday at 16:30, recap one feature using explore → plan → TDD → review","Every day at 18:00, check for unfinished review or verify steps and turn them into tomorrow's todos"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  superpowersMethodologySOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 1332,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "tencentcloud-api",
    labelZh: "腾讯云API专家",
    labelEn: "Tencent Cloud API Expert",
    descriptionZh: "腾讯云API专家 — 通过自然语言管理腾讯云 200+ 产品资源，智能检索 API 文档并构造 tccli 命令执行，内置安全管控（高危操作确认、费用提醒）与异常诊断（错误码解读、凭证引导）机制。",
    descriptionEn: "Tencent Cloud API expert — manage 200+ Tencent Cloud products via natural language: smart API doc retrieval, tccli command construction, safety guardrails (confirmation for risky ops, cost alerts) and error-code diagnostics.",
    welcomeZh: "想查或想管腾讯云上的资源？直接说，比如「查广州的云服务器」「帮我开一台 2 核 4G」",
    welcomeEn: "Want to query or manage Tencent Cloud resources? Just say it, e.g. \"list my CVM instances in Guangzhou\"",
    iconName: "cloud",
    color: "#0052D9",
    quickPrompts: [
    {
      titleZh: "账号登录授权",
      titleEn: "Sign in to Tencent Cloud",
      descriptionZh: "全自动引导 OAuth 登录并回显当前账号身份",
      descriptionEn: "Fully automated OAuth sign-in with identity echo",
      promptZh: "帮我登录腾讯云账号，完成后告诉我当前是哪个账号",
      promptEn: "Sign me in to Tencent Cloud and tell me which account is active",
      color: "#e0f2fe",
      iconName: "log-in",
    },
    {
      titleZh: "查云服务器",
      titleEn: "List CVM instances",
      descriptionZh: "查看指定地域的云服务器实例及运行状态",
      descriptionEn: "View CVM instances and their status in a region",
      promptZh: "帮我查看广州地域的云服务器实例及运行状态",
      promptEn: "List my CVM instances in the Guangzhou region with their status",
      color: "#e8f4ff",
      iconName: "server",
    },
    {
      titleZh: "按量计费开机器",
      titleEn: "Launch a pay-as-you-go CVM",
      descriptionZh: "创建一台按量计费的 2 核 4G 云服务器",
      descriptionEn: "Create a 2C4G pay-as-you-go cloud server",
      promptZh: "帮我在广州创建一台 2 核 4G 的按量计费云服务器",
      promptEn: "Create a pay-as-you-go 2 vCPU / 4 GB CVM in Guangzhou for me",
      color: "#dcfce7",
      iconName: "plus-circle",
    },
    {
      titleZh: "配安全组",
      titleEn: "Configure a security group",
      descriptionZh: "放通 HTTP 和 SSH 访问的安全组规则",
      descriptionEn: "Open HTTP and SSH in a security group",
      promptZh: "帮我配置一个安全组，放通 HTTP 和 SSH 访问",
      promptEn: "Configure a security group that allows HTTP and SSH access",
      color: "#fef9c3",
      iconName: "shield",
    },
    {
      titleZh: "查全地域资源",
      titleEn: "Scan all regions",
      descriptionZh: "遍历所有地域查询实例分布（先列清单再执行）",
      descriptionEn: "Scan every region for instances (list plan first)",
      promptZh: "查看我所有地域的云服务器实例及运行状态",
      promptEn: "Check my CVM instances across all regions and their status",
      color: "#f3e8ff",
      iconName: "globe",
    },
    {
      titleZh: "诊断报错",
      titleEn: "Diagnose an API error",
      descriptionZh: "把 tccli 报错贴给我，解读错误码并给修复方案",
      descriptionEn: "Paste a tccli error and get a decoded fix",
      promptZh: "tccli 报了这个错，帮我看看怎么修：",
      promptEn: "tccli returned this error, help me fix it: ",
      color: "#ffe8f0",
      iconName: "stethoscope",
    },
    ],
    taskExamplesZh: ["每天「09:30」巡检腾讯云 CVM：运行中实例、异常状态和即将到期资源，发给我，任务创建后立即启用","每周一「10:00」汇总上周费用与高危操作记录，标出异常账单","每月 1 日「09:00」检查即将到期的包年包月资源并提醒续费","每个工作日「18:00」核对安全组与密钥变更，只报告差异不自动改","每周五「11:00」汇总本周 API 调用失败率最高的接口","每「6 小时」检查核心实例状态，有异常再通知我，从今天开始持续生效"],
    taskExamplesEn: ["Every day at 09:30, inspect Tencent Cloud CVM — running instances, unhealthy states, and resources nearing expiry — enable immediately","Every Monday at 10:00, summarize last week's billing and high-risk operations, and flag unusual charges","On the 1st of each month at 09:00, check prepaid resources that will expire soon and remind me to renew","Every weekday at 18:00, diff security-group and key changes — report only, no auto-edits","Every Friday at 11:00, list this week's APIs with the highest failure rates","Every 6 hours, check core instance health and notify me only on anomalies, starting today"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions:   '<!-- SOUL.md -->\n' +
  tencentcloudApiSOUL,
    instructionsFiles: ["SOUL.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 3768,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
  {
    id: "wechat-ops",
    labelZh: "发文官 · 内容推送",
    labelEn: "Publisher · Content Push",
    descriptionZh: "一文多发助手 — 将 Markdown 文章一键发布到内容运营平台和社交内容平台。微信端通过 wenyan-cli 渲染精美排版，社交内容平台端自动适配短笔记 + 图片 + 话题标签格式。数据完全自主可控，不依赖第三方 SaaS 平台。",
    descriptionEn: "Multi-platform publisher — publish Markdown articles to Content Platform and Social Platform (Social Platform) simultaneously. WeChat side renders beautiful layouts via wenyan-cli, Social Platform side auto-adapts to short notes + images + hashtags. Fully self-hosted, no third-party SaaS dependency.",
    welcomeZh: "把文章或 Markdown 给我，帮你多平台发布",
    welcomeEn: "Give me your article or Markdown and I'll publish across platforms",
    iconName: "message-square",
    color: "#07c160",
    quickPrompts: [
    {
      titleZh: "发布到公众号",
      titleEn: "Publish to WeChat",
      descriptionZh: "Markdown 渲染后推送到草稿箱",
      descriptionEn: "Render Markdown and push to draft box",
      promptZh: "请帮我把以下 Markdown 文章发布到微信公众号草稿箱：\n\n",
      promptEn: "Please publish the following Markdown article to WeChat Official Account draft box:\n\n",
      color: "#e8f4ff",
      iconName: "message-square",
    },
    {
      titleZh: "同步到社交平台",
      titleEn: "Publish to social",
      descriptionZh: "自动适配短笔记 + 图片 + 话题",
      descriptionEn: "Auto-adapt to short notes + images + hashtags",
      promptZh: "请帮我把以下 Markdown 内容适配并发布到社交内容平台（短笔记 + 图片 + 话题标签）：\n\n",
      promptEn: "Please adapt and publish the following Markdown to social platform (short note + images + hashtags):\n\n",
      color: "#dcfce7",
      iconName: "share-2",
    },
    {
      titleZh: "一文多发全流程",
      titleEn: "Multi-platform publish",
      descriptionZh: "同一篇文章同时发布到多个平台",
      descriptionEn: "Publish one article to multiple platforms",
      promptZh: "请帮我把以下 Markdown 文章一文多发，同时发布到微信公众号和社交内容平台：\n\n",
      promptEn: "Please multi-publish the following Markdown to WeChat and social platform simultaneously:\n\n",
      color: "#fef3c7",
      iconName: "send",
    },
    {
      titleZh: "环境检查配置",
      titleEn: "Environment check",
      descriptionZh: "检查 wenyan-cli、API 和 Cookie 配置",
      descriptionEn: "Check wenyan-cli, API, and cookie setup",
      promptZh: "请帮我检查一文多发所需的环境配置，包括 wenyan-cli 安装、微信 AppID/AppSecret、IP 白名单和社交内容平台 Cookie。",
      promptEn: "Please check the multi-platform publishing environment — wenyan-cli, WeChat AppID/Secret, IP whitelist, and social platform cookies.",
      color: "#f1f5f9",
      iconName: "settings",
    },
    {
      titleZh: "文章排版优化",
      titleEn: "Layout optimization",
      descriptionZh: "选择主题、优化公众号排版效果",
      descriptionEn: "Choose theme and optimize WeChat layout",
      promptZh: "请帮我优化以下文章的公众号排版，推荐合适的 wenyan 主题并预览效果：\n\n",
      promptEn: "Please optimize WeChat layout for the following article, recommend a wenyan theme and preview:\n\n",
      color: "#eef2ff",
      iconName: "layout",
    },
    {
      titleZh: "图片素材处理",
      titleEn: "Image asset handling",
      descriptionZh: "上传图片到微信素材库",
      descriptionEn: "Upload images to WeChat media library",
      promptZh: "请帮我处理文章中的图片素材，将本地和网络图片上传到微信素材库并替换链接。",
      promptEn: "Please process article images — upload local and remote images to WeChat media library and replace links.",
      color: "#fff7ed",
      iconName: "image",
    },
    ],
    taskExamplesZh: ["每个工作日「09:00」检查公众号草稿箱，列出待发布文章并提醒我确认，任务创建后立即启用","每周一「10:00」根据本周选题日历起草一篇短内容，放到待发布","每月 1 日「09:30」汇总上月各平台发布数据，给出下月选题建议","每个工作日「18:00」复盘当天阅读、点赞和留言，标出需回复的评论","每周五「16:00」对照选题日历检查下周空档，补一条备稿","每天「12:00」检查待发布队列，有到点稿件提醒我确认"],
    taskExamplesEn: ["Every weekday at 09:00, check the WeChat draft box, list posts waiting to publish, and ask me to confirm — enable immediately","Every Monday at 10:00, draft a short piece from this week's topic calendar and leave it in the publish queue","On the 1st of each month at 09:30, summarize last month's publish stats and suggest next month's topics","Every weekday at 18:00, recap today's reads, likes, and comments, and flag replies still due","Every Friday at 16:00, check next week's topic calendar for gaps and add one backup draft","Every day at 12:00, check the publish queue and remind me to confirm anything due"],
    /** 拼好的系统提示正文；分段标记见 instructionsFiles。 */
    instructions: [
  '<!-- SOUL.md -->\n' +
  wechatOpsSOUL,
  '<!-- IDENTITY.md -->\n' +
  wechatOpsIDENTITY,
].join('\n\n'),
    instructionsFiles: ["SOUL.md","IDENTITY.md"],
    /** instructions 的实测字符数（JS length 与 Rust chars().count() 对 CJK 一致）。 */
    instructionsChars: 2625,
    personaMissing: false,
    /** 超过服务端 MAX_INSTRUCTIONS（20000 字符）时为 true，上层需决定取舍，本模块不截断原文。 */
    oversized: false,
  },
]

export const LIBRARY_EXPERTS_BY_ID: Record<string, LibraryExpert> = Object.freeze(
  Object.fromEntries(LIBRARY_EXPERTS.map((expert) => [expert.id, expert])),
)

/** 取不到就是 undefined，调用方自己决定怎么提示，不编造占位专家。 */
export function findLibraryExpert(id: string): LibraryExpert | undefined {
  return LIBRARY_EXPERTS_BY_ID[id]
}

const OVERSIZED = LIBRARY_EXPERTS.filter((expert) => expert.oversized)
const PERSONA_MISSING = LIBRARY_EXPERTS.filter((expert) => expert.personaMissing)

if (import.meta.env.DEV && (OVERSIZED.length > 0 || PERSONA_MISSING.length > 0)) {
  for (const expert of OVERSIZED) {
    console.error(
      `[experts/library] ${expert.id} 的 instructions 为 ${expert.instructionsChars} 字符，超过服务端上限 ` +
        `${LIBRARY_MAX_INSTRUCTIONS_CHARS}，直接提交会被 400 拒绝。`,
    )
  }
  for (const expert of PERSONA_MISSING) {
    console.warn(`[experts/library] ${expert.id} 人格文件缺失：${expert.personaMissingNote ?? ''}`)
  }
}
