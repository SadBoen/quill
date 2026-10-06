# Octop 设计基准（只读参考，不要改这个目录里的图）

`ui-models.png` 是 Octop 官方文档里的「模型管理」页截图，来源：

- 页面：https://docs.octop.cloud/en/guide/guides/llm-providers
- 图片：https://docs.octop.cloud/images/ui-models.png
- 抓取时间：2026-10-05
- 对应 Octop 版本：截图右上角标注 `v0.9.18`

## 源码：sparse checkout（2026-10-05 加入，2026-10-05 扩到建图所需范围）

对话界面与预设专家不再只看截图，直接对着 Octop 源码看：

```bash
git clone --filter=blob:none --sparse --depth 1 \
  https://github.com/TencentCloud/Octop.git .octop-ref/octop
cd .octop-ref/octop
# 2026-10-05 更新到最新版：git fetch --depth 1 origin main
git sparse-checkout set \
  dashboard/src/pages/Experts \
  dashboard/src/pages/Chat \
  dashboard/src/components \
  dashboard/src/hooks \
  dashboard/src/api \
  dashboard/src/locales \
  dashboard/src/utils \
  dashboard/src/routes \
  src/octop/infra/agents/experts
```

- 仓库：https://github.com/TencentCloud/Octop
- pin：`eb28011249c02cafd389b2d424294c6c1b9cf422`（`chore: release 1.0.2b6`，2026-10-05 07:47，**已核对为 main 最新**）
- 许可：MIT，Copyright (c) 2026 Octop（原文见 `.octop-ref/octop/LICENSE`）
- 取用范围：`experts/library/`（预设专家，已 vendor 进 `ui/web/src/experts/library/`，保留 MIT 出处）
  与 dashboard 侧（专家页/对话页基准，**不 vendor，只读**）

**注意**：线上 https://oc.sadinsun.top 那个站是 v1.0.1，仓库已经到 1.0.2b6。
**以仓库源码为准，不要对着运行站抄**——那会落后一版。

**不要 vendor Octop 的美术资源**：`dashboard/public/experts/avatars/*.svg` 是他们的插画头像，
quill 用首字母色块代替。

## 抄功能用 `graph.mjs`，不要整读文件

`dashboard/src/pages/Experts/` 里 `index.tsx` 28KB、`EditAgentDrawer.tsx` 48KB、
`CreateFromExpertDrawer.tsx` 46KB、`index.module.less` 43KB。每次要抄一个功能就整读
进上下文，既慢又贵。`.octop-ref/graph.mjs` 把结构抽成可查询的表：

```bash
node .octop-ref/graph.mjs files            # 文件清单 + 体积，按大到小
node .octop-ref/graph.mjs comps            # 组件清单（带行号/体积/有无 props）
node .octop-ref/graph.mjs comps Team       # 按名字过滤
node .octop-ref/graph.mjs props AgentCard  # 某组件的 props interface + 签名
node .octop-ref/graph.mjs copy confirmDelete  # i18n key 的中文原文（抄界面文字用这个）
node .octop-ref/graph.mjs i18n experts.    # key + 中文 + 调用点 文件:行号
node .octop-ref/graph.mjs api expert       # api 层导出
node .octop-ref/graph.mjs find 'Popconfirm' # 在 Experts 页里按正则找行
node .octop-ref/graph.mjs read <file:line> # 以该行为中心的 ±20 行，目标行前缀 `>`
```

`read` 过去有个坑：窗口是「前 20 后 40」，请求行落在窗口第 22 位且没有任何标记，
于是人会照着窗口第一行去引用，正好差 20 行。现已改成对称并把目标行标出来 ——
`MILESTONES.md` 要求每条上游引用都能被复查，一个会让人引错行的工具比没有工具更糟。

> 这不是「有 codegraph 服务」，是本地替代。查过 `mcode-tools connector tools`，
> 当前环境只挂了 `email_workbench` 和 `matrix`（多模态），**没有 codegraph**；
> 本地 skill 目录也没有。Octop 又是私有代码，外部 code graph 根本没它的索引，
> 所以「把源码拉到本地 + 建索引」本来就是更合适的做法。



## 为什么放在这里

`ui/web/src/models/ModelsPage.tsx` 的三块结构（可用模型池 / 预设提供商 / 自定义）
是照这张图实现的。改那个页面的布局时，先对着这张图看一遍，
避免"凭记忆改"导致结构漂移。

## 与另一个 vendor 目录的区别

- `vendor/openoctopus-frontend/` —— OpenOctopus（Zpoteiti/OpenOctopus）的前端源码，
  是 quill `ui/web` 的移植基准，**需要**逐字节保持一致（有 sha256 校验）。
- 本目录 —— TencentCloud/Octop 的**产品设计参考图**，
  只用来对齐信息架构和视觉层次，**没有**代码级一致性要求。

两者是不同项目：Octop 是 `octop` CLI + `~/.octop/` + dashboard 的完整产品，
Octop 的模型管理页有真实的自动路由和多 provider 目录；
quill 只借它的页面结构，数据全部来自 quill 自己的 `/api/admin/providers` 与
`/api/admin/models`，不存在任何移植来的假数据。

## 预设专家的人格原文有 Octop 专属能力（已知取舍）

17 个内置专家、**36 份**人格 markdown 是**逐字 vendor** 的（MIT 允许）。
其中相当一部分提到技能包 / 插件 / 渠道 / 定时任务 / `HEARTBEAT.md` / `BOOTSTRAP.md`
这类 Octop 专属能力，个别文件甚至直接写着
"an office automation assistant **inside Octop**"（`office-automation/SOUL.md`）。

> 命中数取决于关键词怎么取，这里给的是可复算的那一组：
> `Select-String -Path ui/web/src/experts/library/**.md -Pattern 'skill package|plugin|channel|HEARTBEAT|BOOTSTRAP|cron|schedule'`
> 当前命中 **10 / 36**。旧版本这里写的是「40 个文件里有 27 个」—— 文件总数与命中数
> 都复现不出来，属于会腐烂的数字，改成能重跑的口径。

quill 目前**没有**插件、渠道与定时任务（`GET /api/extensions/plugins` 仍是 501 桩，
`/api/cron` 连路由都没注册），但**技能与 MCP 已经真跑通并挂进对话工具表**
（`GET/POST /api/extensions/skills`、`GET/POST /api/extensions/mcp` 都是真实现，
MCP 走 rmcp 真握手、`tools/call` 真调用）。所以旧稿里那句「`/api/extensions/*`
全是 501，也没有任何工具调用能力」已经不成立，选中这些专家后模型**仍可能提到
它做不到的事** —— 缺的是上面那几项，不是工具调用本身。
真实的能力边界见 `../ui/web/src/capabilityGaps.ts`（界面上直接给用户看的那份）。

这是已知取舍，处理方式：

- 人格原文不改（改了就不是"抄 Octop"了）；
- 专家库里**如实标注**这一点，不假装每个专家都能完整工作；
- 不因为"怕模型说错话"就偷偷截断或改写人格 —— 那属于伪造用户可见内容。

