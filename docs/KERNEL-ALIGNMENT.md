# KERNEL-ALIGNMENT —— Q028 / Q029 比对结论

> **生成日期**：2026-10-08
> **goose 参考版本**：`vendor/goose` **v1.53.0**（`vendor/goose/Cargo.toml:11` → `version = "1.53.0"`）
> **本文档的每条结论都附复现命令；命令输出会随代码变化，本文档不保证长期有效。**
> 本轮**不改任何 Rust/TS 代码**，只记录比对事实。
> 复现环境是 Windows + Git Bash（MSYS2）。本机 GNU grep 在**按目录递归**时把路径分隔符
> 渲染成 `\`（例：`crates/\quill-server\src\llm.rs`），它等价于
> `crates/quill-server/src/llm.rs`。下文输出片段按原样粘贴，未做修饰。

复现版本号：

```bash
$ grep -m1 '^version' vendor/goose/Cargo.toml
version = "1.53.0"
```

---

## Q028 斜杠命令

### 一句话结论

**quill 没有斜杠命令（0 行实现）**：`crates/` 与 `ui/web/src/` 里没有任何 `/xxx` 的
拦截、解析、注册表、路由或补全实现；grep 命中的 15 行全部是「文档里提到 goose 有这功能」、
「测试数据里出现斜杠字符」两类，与实现无关。goose 侧是一条**跨 12 个文件、19 处
`slash_commands` 引用**的接线链（`slash_commands/` 模块 6 个文件 + 7 个 `run_command`
实现者 + ACP 暴露 + agent 注册），quill 侧缺口是**整块**。

### 1. quill 侧的证据（先核有没有）

复现命令与真实输出（`head -50`）：

```bash
$ grep -rniE "slash|/command|斜杠" crates/ ui/web/src/ --include=*.rs --include=*.ts --include=*.tsx | head -50
crates/\quill-backup\src\manifest.rs:306:        return Some("含反斜杠（Windows 分隔符，会绕过上级目录检查）");
crates/\quill-backup\src\manifest.rs:321:            return Some("含空路径段（双斜杠或以 / 开头）");
crates/\quill-core\src\state_machine.rs:24://! | `goose/src/agents/agent.rs:1690-1768` | step 注册顺序 = 转移优先级：`entry_hook, slash_command, steer, max_turns, bang_shell, compaction, tool_pair_compaction, tool_approval, doctor, project, skills, recipe, tool_execution, unknown_tool, retry, stop_hook, exit_on_error, status, llm` |
crates/\quill-core\src\state_machine.rs:108://! - `ops_slash_command.rs:15-84`（含 `ops_status.rs`、`ops_doctor.rs`、`ops_recipe.rs`、
crates/\quill-core\src\state_machine.rs:109://!   `ops_skills.rs` 的命令分支）：斜杠命令拦截 kickoff。需要命令注册表、extension manager、
crates/\quill-core\src\state_machine.rs:131://! - 未表达的 goose 行为：retry/stop-hook 的「重新打开轮次」回边、steer、斜杠命令与
crates/\quill-core\src\state_machine.rs:913:            "slash_command",
crates/\quill-provider\src\openai.rs:407:    fn a_base_url_is_trimmed_and_slashes_are_dropped() {
crates/\quill-server\src\api_extensions.rs:553:        let e = hub_error("请求不合法", HubError::Input("技能标识含斜杠".into()));
crates/\quill-server\src\api_extensions.rs:1187:        // 归一失败（上游给了个带斜杠的 slug）时退回文件名，而不是整包失败 ——
crates/\quill-server\src\sse.rs:100:            // 负载里带换行、带引号、带反斜杠、带花括号都不能破帧。
crates/\quill-server\src\sse.rs:102:            json!({"text": "带\"引号\"和 \\ 反斜杠"}),
crates/\quill-server\src\channels\weixin.rs:509:    fn base_url_joins_without_double_slash() {
crates/\quill-server\src\skillhub\unpack\mod.rs:519:    // 反斜杠在 Windows 上是路径分隔符，冒号会造出 `C:` 这种盘符。
crates/\quill-server\tests\backup_http.rs:587:    // 反斜杠转义成两个，拿未转义的路径去 contains 原始正文，在 Windows 上
crates/\quill-server\tests\team_http.rs:89:/// 建若干个属于 uid 的普通专家（走真实 HTTP 接口，斜杠也顺带被覆盖到）。
```

逐条判读：

| 命中 | 性质 |
|---|---|
| `quill-core/src/state_machine.rs:24` | 文档注释，抄 goose 的 op 注册顺序，提到 `slash_command` 是 goose 的 op 名 |
| `quill-core/src/state_machine.rs:108-111` | 文档注释，在「没搬的 ops」清单里点名 `ops_slash_command.rs:15-84`，并写「是 kickoff 上的**拦截分支**，没搬」 |
| `quill-core/src/state_machine.rs:131-132` | 文档注释，把「斜杠命令与 `!` 命令的 kickoff 拦截」列为「未表达的 goose 行为」 |
| `quill-core/src/state_machine.rs:913` | 单元测试里的字符串常量表，`"slash_command"` 只是 goose op 名（用于校验状态映射） |
| 其余 11 行 | 路径校验（manifest）、URL 反斜杠、技能 slug、SSE 测试负载、测试注释 —— 与斜杠命令无关 |

再核「有没有在任何地方按 `/` 前缀分流」：

```bash
$ grep -rn "starts_with(\"/\")\|starts_with('/')" crates/ --include=*.rs
crates/\quill-backup\src\manifest.rs:301:    if rel.starts_with('/') {
crates/\quill-wiki\src\store.rs:136:        if p.is_absolute() || rel.starts_with('/') || rel.starts_with('\\') {
(grep exit=0；两处都是路径安全校验，不是对话消息分流)
```

UI 侧核「输入框对 `/` 的处理」（`ui/web/src/chat/ChatPage.tsx`）：

```bash
$ sed -n '237,240p;562,582p' ui/web/src/chat/ChatPage.tsx
  async function handleSubmit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    const sentText = text.trim()
    if (sending || !sentText) return
            <textarea
              ref={composerInput}
              name="message"
              autoComplete="off"
              aria-label={t('draftChat.message', { defaultValue: '消息' })}
              placeholder={t('draftChat.placeholder', { defaultValue: '输入消息，Enter 发送，Shift+Enter 换行' })}
              rows={2}
              value={text}
              onChange={(event) => setText(event.target.value)}
              onKeyDown={(event) => {
                if (
                  event.key !== 'Enter'
                  || event.shiftKey
                  || event.nativeEvent.isComposing
                  || event.nativeEvent.keyCode === 229
                ) return
                event.preventDefault()
                event.currentTarget.form?.requestSubmit()
              }}
              disabled={sending}
            />
```

`onChange` 只做 `setText`，`onKeyDown` 只处理 Enter 提交；**没有** `/` 弹层、没有命令补全、
没有前缀剥离。发送路径是 `ui/web/src/chat/chatApi.ts:40 sendChatMessage()` →
`POST /api/sessions/{id}/messages`（`crates/quill-server/src/routes.rs:116`）→
`crates/quill-server/src/api_chat.rs:750 post_message` → `:1013 run_turn`，这条链上
**没有**任何命令分支。

### 2. goose 侧：模块清单与接线（逐条 file:line）

模块清单（命令与输出）：

```bash
$ ls vendor/goose/crates/goose/src/slash_commands/
mod.rs
recipe_slash_command.rs
skill_slash_command.rs
slash_command.rs
types.rs
util.rs
```

```bash
$ grep -rn "slash_commands" vendor/goose/crates/goose/src/ | head
vendor/goose/crates/goose/src/\lib.rs:51:pub mod slash_commands;
vendor/goose/crates/goose/src/\acp\response_builder.rs:6:use crate::slash_commands::types::{SlashCommandEntry, SlashCommandSource};
vendor/goose/crates/goose/src/\acp\response_builder.rs:469:    if !crate::agents::execute_commands::slash_commands_enabled() {
vendor/goose/crates/goose/src/\acp\response_builder.rs:473:    crate::slash_commands::slash_command::list_acp_commands(working_dir)
vendor/goose/crates/goose/src/\acp\server\custom_dispatch.rs:738:    async fn dispatch_list_slash_commands(
vendor/goose/crates/goose/src/\acp\server\custom_dispatch.rs:742:        self.on_list_slash_commands(req).await
vendor/goose/crates/goose/src/\acp\server.rs:126:mod slash_commands;
vendor/goose/crates/goose/src/\acp\server\slash_commands.rs:5:    pub(super) async fn on_list_slash_commands(
vendor/goose/crates/goose/src/\acp\server\recipe\mod.rs:33:use crate::slash_commands::recipe_slash_command;
vendor/goose/crates/goose/src/\agents\execute_commands.rs:9:use crate::slash_commands::{recipe_slash_command, skill_slash_command};
```

逐块出处（每行都用 `sed -n` 核过首行内容）：

| # | goose 位置 | 内容（首行原文） |
|---|---|---|
| 1 | `slash_commands/mod.rs:1-5` | 导出 `recipe_slash_command` / `skill_slash_command` / `slash_command` / `types` / `util` |
| 2 | `slash_commands/types.rs:1-6` | `pub enum SlashCommandSource { Builtin, Recipe, Skill }` |
| 3 | `slash_commands/types.rs:9-15` | `pub struct SlashCommandEntry { name, description, source, source_path, input_hint }` |
| 4 | `slash_commands/util.rs:1-3` | `pub fn normalize_command_name(name: &str) -> String`（去前导 `/` + 小写） |
| 5 | `slash_commands/slash_command.rs:7-18` | `list_builtin_commands()`：把 `execute_commands::list_commands()` 包成 `SlashCommandEntry` |
| 6 | `slash_commands/slash_command.rs:20-28` | `list_acp_commands(working_dir)`：builtin + recipe + skill 三源合并 |
| 7 | `slash_commands/slash_command.rs:30-53` | `merge_command_sources()`：优先级 builtin > recipe > skill，按 `normalize_command_name` 去重 |
| 8 | `slash_commands/recipe_slash_command.rs:14` | 配置键 `const SLASH_COMMANDS_CONFIG_KEY: &str = "slash_commands";` |
| 9 | `slash_commands/recipe_slash_command.rs:16-20` | `SlashCommandMapping { command, recipe_path }` |
| 10 | `slash_commands/recipe_slash_command.rs:22-32` | `list_commands()`：从 `Config::global()` 读映射，失败回退空表 + warn |
| 11 | `slash_commands/recipe_slash_command.rs:40-57` | `set_recipe_slash_command()`：写入映射（`trim_start_matches('/').to_lowercase()`） |
| 12 | `slash_commands/recipe_slash_command.rs:59-66` | `get_recipe_for_command()` |
| 13 | `slash_commands/recipe_slash_command.rs:68-88` | `commands_from_mappings()`：读 recipe 元数据生成条目 |
| 14 | `slash_commands/recipe_slash_command.rs:116-143` | `input_hint_for_recipe()`：`<必选>` / `[--可选 <x>]` |
| 15 | `slash_commands/recipe_slash_command.rs:149-225` | `resolve_command()`：解析参数、模板化成 prompt |
| 16 | `slash_commands/recipe_slash_command.rs:227-277` | `parse_recipe_args()`：位置参数与 `--flag` |
| 17 | `slash_commands/skill_slash_command.rs:8-10` | `list_commands()`：从已装 skills 生成 |
| 18 | `slash_commands/skill_slash_command.rs:12-41` | `format_installed_skills()`（`/skills` 的正文） |
| 19 | `slash_commands/skill_slash_command.rs:43-60` | `resolve_command()`：skill → prompt，忽略大小写匹配 |
| 20 | `slash_commands/skill_slash_command.rs:62-81` | `commands_from_sources()`，标记 `SlashCommandSource::Skill` |
| 21 | `agents/execute_commands.rs:13-17` | `slash_commands_enabled()`：开关 `GOOSE_SLASH_COMMANDS_ENABLED`，默认 `true` |
| 22 | `agents/execute_commands.rs:22-65` | `CommandDef` 与 9 个内置命令：`prompts / prompt / compact / clear / skills / doctor / goal / grind / status` |
| 23 | `agents/execute_commands.rs:67-70` | `pub struct ParsedSlashCommand { command, params_str }` |
| 24 | `agents/execute_commands.rs:72-93` | `parse_slash_command()`：trim、compact 别名归一、切第一个空格 |
| 25 | `agents/execute_commands.rs:95-97` | `list_commands()` 返回内置命令表 |
| 26 | `agents/execute_commands.rs:105-117` | `is_known_slash_command()` |
| 27 | `agents/execute_commands.rs:126-133` | `command_starts_turn()`（只有带描述的 goal/grind 才起新轮） |
| 28 | `agents/execute_commands.rs:136-` | `Agent::execute_command`（真实执行入口） |
| 29 | `agents/state_machine/ops_slash_command.rs:19-36` | 状态机版 `parse_slash_command()`：`/compact`、`/summarize`、"Please compact this conversation" 归一 |
| 30 | `agents/state_machine/ops_slash_command.rs:44-84` | `SlashCommandOperation::run`：仅当「kickoff 后恰好 1 条消息」时拦；逐个 handler 试 `run_command` |
| 31 | `agents/state_machine/ops_slash_command.rs:56-58` | 先读开关，关掉就 `not_applicable()` |
| 32 | `agents/agent.rs:1752-1760` | 接线：`command_handlers = operations + status_operation`，包进 `SlashCommandOperation`，排在 `entry_hook` 之后、其余 op 之前 |
| 33 | `agents/state_machine/{ops_compaction.rs:178, ops_doctor.rs:23, ops_retry.rs:91, ops_recipe.rs:213, ops_skills.rs:236, ops_status.rs:36, ops_toolcalling.rs:739}` | 7 个 `fn run_command` 实现者（命令的实际执行分支） |
| 34 | `acp/response_builder.rs:466-477` | ACP 暴露 `available_commands`（`list_acp_commands`） |
| 35 | `acp/server/slash_commands.rs:5-43` | `on_list_slash_commands()`（cwd / session 解析） |
| 36 | `acp/server/custom_dispatch.rs:738-742` | `dispatch_list_slash_commands()` |
| 37 | `slash_commands/slash_command.rs:60-77`（测试函数，`#[test]` 在 `:59`） | 内置命令名单断言：`["prompts","prompt","compact","clear","skills","doctor","goal","grind","status"]` |

「12 个文件 / 19 处引用」的复现：

```bash
$ grep -rl "slash_commands" vendor/goose/crates/goose/src/ | wc -l
12
$ grep -rn "slash_commands" vendor/goose/crates/goose/src/ | wc -l
19
```

接线命令输出：

```bash
$ grep -rn "fn run_command" vendor/goose/crates/goose/src/agents/state_machine/*.rs
vendor/goose/crates/goose/src/agents/state_machine/ops_compaction.rs:178:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_doctor.rs:23:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_retry.rs:91:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_recipe.rs:213:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_skills.rs:236:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_status.rs:36:    async fn run_command(
vendor/goose/crates/goose/src/agents/state_machine/ops_toolcalling.rs:739:    async fn run_command(
```

```bash
$ sed -n '1752,1760p' vendor/goose/crates/goose/src/agents/agent.rs
        let mut command_handlers = operations.clone();
        command_handlers.push(status_operation);
        let command_operation: Arc<dyn Operation<Session, GooseEffect> + '_> =
            Arc::new(SlashCommandOperation::new(command_handlers));
        let operations: Vec<_> =
            std::iter::once(Arc::new(EntryHookOperation::new(self.hook_manager.clone()))
                as Arc<dyn Operation<Session, GooseEffect> + '_>)
            .chain(std::iter::once(command_operation))
            .chain(operations)
            .collect();
```

### 3. Q028 对齐表（goose ↔ quill）

| goose 出处 | quill 落点 | 覆盖度 |
|---|---|---|
| `slash_commands/types.rs:1-15`（条目/来源类型） | 无 | **未覆盖** |
| `slash_commands/util.rs:1-3`（名字归一） | 无 | **未覆盖** |
| `slash_commands/slash_command.rs:7-53`（三源合并 + 去重优先级） | 无 | **未覆盖** |
| `slash_commands/recipe_slash_command.rs:14-277`（recipe 命令：映射、参数、模板化） | 无 | **未覆盖** |
| `slash_commands/skill_slash_command.rs:8-81`（skill 命令：列表、hint、执行） | 无 | **未覆盖** |
| `agents/execute_commands.rs:13-17`（开关） | 无 | **未覆盖** |
| `agents/execute_commands.rs:22-65`（9 个内置命令表） | 无 | **未覆盖** |
| `agents/execute_commands.rs:72-133`（解析 / 已知判定 / 起轮判定） | 无 | **未覆盖** |
| `agents/state_machine/ops_slash_command.rs:19-84`（kickoff 拦截） | 只有文档注释承认「没搬」：`quill-core/src/state_machine.rs:108-111`、`:131-132` | **未覆盖**（有意留白，已记账） |
| `agents/agent.rs:1752-1760`（注册顺序） | `quill-core/src/state_machine.rs:24` 的表里列了 op 名，但状态机没有对应 op | **未覆盖** |
| 7 个 `run_command`（各 op 的命令分支） | 无 | **未覆盖** |
| `acp/response_builder.rs:466-477` + `acp/server/slash_commands.rs:5` + `acp/server/custom_dispatch.rs:738`（对外暴露） | 无对应 HTTP/API 路由（`routes.rs` 里无 commands 路由） | **未覆盖** |
| UI 命令补全 | `ui/web/src/chat/ChatPage.tsx:562-578` 只处理 Enter 提交；无 `/` 分支 | **未覆盖** |

### 4. 缺口与下一步落点（**建议，尚未实现**）

| 缺口 | 落点 |
|---|---|
| 命令条目/来源类型 | `quill-core`（照 `slash_commands/types.rs:1-15` 抄）——纯类型，无 I/O |
| 解析与拦截（「kickoff 后恰好 1 条消息才拦」这一条语义） | `crates/quill-core/src/state_machine.rs`（`ConversationState::NoKickoff × UserInput` 那条转移前，照 `ops_slash_command.rs:60-70`） |
| 真实执行分支（内置命令） | `crates/quill-server/src/api_chat.rs:750 post_message` / `:1013 run_turn` 之前（对应 goose `agents/execute_commands.rs:136`） |
| 命令清单对外暴露 | 新增 route（对应 `acp/response_builder.rs:466-477` 的 `available_commands`） |
| `/` 补全 UI | `ui/web/src/chat/ChatPage.tsx:562` 的 textarea（`onChange` 里判 `startsWith('/')`） |
| 依赖项（goose 侧命令用到而 quill 未搬） | `/compact` 需压缩执行（quill 侧 `quill-core/src/compaction.rs` 已有纯逻辑，模块文档自述「真正调模型生成摘要由壳侧注入（Q018 接线，本轮不做）」）；`/skills` 需 skill 列表（`quill-server/src/skills_repo.rs` 已有）；`/doctor` 需 doctor 子系统（quill 侧是 CLI 子命令 `crates/quill-cli/src/lib.rs:194`，不是对话内命令） |

---

## Q029 provider 覆盖度

### 一句话结论

`quill-provider` 覆盖了 goose provider 栈的**传输层**（Provider trait 4/26 方法、
OpenAI chat-completions 编解码、SSE、错误分类），而 goose 的**模型目录、多协议格式层、
重试、thinking 控制、多模态、live/voice**整块未覆盖；§1.6 记的「provider 组装 2 套」
经核实成立，但重叠面比 §1.6 的写法小：`quill-provider`（传输）与
`llm_providers.rs`（DB 配置 + 探测 + 组装）是**上下游关系**，真正重复的是
「`/models` 探测」与「base_url 归一 / 传输错误分类」这 3 处各写两遍。

### 1. 规模与模块清单

```bash
$ ls crates/quill-provider/src/
error.rs
lib.rs
local.rs
openai.rs
provider.rs
pump.rs
sse.rs
types.rs
wire.rs

$ ls vendor/goose/crates/goose-providers/src/
anthropic.rs
api_client.rs
azure_foundry.rs
browser_live_transport.rs
databricks.rs
databricks_auth.rs
databricks_v2.rs
decision.rs
declarative/
declarative.rs
google.rs
http_status.rs
lib.rs
live.rs
live_transport_websocket.rs
live_voice_provider.rs
local_inference.rs
ollama.rs
openai.rs
openai_compatible.rs
openai_live.rs
openai_live_voice_provider.rs
openrouter.rs
openrouter_format.rs
snowflake.rs
typesafe.rs
```

行数（`wc -l`，含测试）：

```bash
$ cat crates/quill-provider/src/*.rs | wc -l
3241
$ cat vendor/goose/crates/goose-providers/src/*.rs | wc -l
15687
$ cat vendor/goose/crates/goose-provider-types/src/*.rs vendor/goose/crates/goose-provider-types/src/*/*.rs | wc -l
30822
```

> 注：`docs/ARCHITECTURE.md` §1.3 记 `goose-provider-types` 为 8150 行，那是**只算顶层
> `src/*.rs`** 的口径（`for d in vendor/goose/crates/*/; do cat "$d"/src/*.rs | wc -l`），
> 不含 `formats/`、`conversation/`、`canonical/` 三个子目录（+22672 行）。
> 两个口径都能复现，不是错账，是**口径没写明**。

### 2. §1.6「provider 组装 2 套」核实

先看两套各自的文件头自述（原文）：

```bash
$ sed -n '1,4p' crates/quill-provider/src/lib.rs
//! LLM 供应商抽象。
//!
//! 这一层只做三件事：把请求/响应映射到 OpenAI 兼容线格式、把流式 SSE 解成增量、
//! 以及把失败分类成「下一步该做什么」。它不关心编排、存储或界面。

$ sed -n '1,7p' crates/quill-server/src/llm_providers.rs
//! 多模型供应商的数据模型、持久化与**真实**模型探测。
//!
//! 铁律：模型池里的每一个模型都必须来自上游 `/models` 的真实响应。
//! 探测不到就是探测不到 —— 返回中文原因，绝不退回硬编码列表，也绝不
//! 用一个编造的 `context_window` 填坑。派生字段（display_name /
//! modality / context_window）只允许从上游上报的字段里推，推不出来
//! 就给 `null` 或 `"unknown"`。
```

职责划分（用命令核实，不靠自述）：

```bash
$ grep -rn "llm::build" crates/quill-server/src/
crates/quill-server/src/\api_admin.rs:91:    let new_provider = match crate::llm::build(&llm_cfg) {
crates/quill-server/src/\api_chat.rs:942:    let request = crate::llm::build_request(&prep.llm_config, msgs.to_vec());
crates/quill-server/src/\api_chat.rs:1090:        let final_request = crate::llm::build_request(&prep.llm_config, msgs.clone());
crates/quill-server/src/\state.rs:169:        match crate::llm::build(&cfg) {
crates/quill-server/src/\server.rs:119:        match crate::llm::build(&env_llm_config) {
crates/quill-server/src/\server.rs:212:    match crate::llm::build(&base) {
crates/quill-server/src/\api_providers.rs:422:    crate::llm::build(&cfg).map(|_| ()).map_err(|detail| {

$ grep -rn "sqlx" crates/quill-provider/src/
(grep exit=1，无输出 —— quill-provider 不碰数据库)
$ grep -c "sqlx::query" crates/quill-server/src/llm_providers.rs
9
```

| 维度 | 第 1 套：`quill-provider`（crate，叶子） | 第 2 套：`llm_providers.rs` + `llm.rs`（quill-server 内） |
|---|---|---|
| 行数 | 3241 | 957 + 388 = 1345 |
| 职责 | 传输：wire 编解码、SSE、流泵、错误分类、`OpenAiCompatible` | 配置与组装：`LlmProvider` 行模型（DB）、`ProviderCache`、`/models` 探测、`to_llm_config`、`build` |
| 依赖 | 仅 async-stream/futures/reqwest/serde（`crates/quill-provider/Cargo.toml`，无内部 crate、无 sqlx） | 依赖 `quill-provider`（`llm.rs:4`）、sqlx、quill_agent |
| 入口 | `Provider` trait（`provider.rs:18`）、`OpenAiCompatible`（`openai.rs:35`） | `llm::build()`（`llm.rs:205`）→ 产出 `SharedProvider` |
| 运行时消费者 | 类型/函数被 server 直接引用（`api_chat.rs:9`、`tools.rs:21`、`skills_repo.rs:357` 等）；运行时对象只经 `llm::build` 产出 | `state.rs:66 AppState::llm()` → `api_chat.rs:796`、`api_dispatch.rs:316` |

**重叠（3 处，均可复现）**：

1. **`/models` 探测两套实现**

```bash
$ grep -rn "fn models_url" crates/ --include=*.rs
crates/\quill-provider\src\openai.rs:90:    pub fn models_url(&self) -> String {
crates/\quill-server\src\llm_providers.rs:416:fn models_url(base_url: &str) -> String {

$ grep -rn "\.models(" crates/ --include=*.rs
crates/\quill-provider\tests\openai_http.rs:355:        .models()
crates/\quill-provider\tests\openai_http.rs:374:        .models()
crates/\quill-provider\tests\live_local_profile.rs:20:    let models = provider.models().await.expect("应当能列出模型");
crates/\quill-provider\src\provider.rs:76:        let models = shared.models().await.expect("桩实现应当成功");
crates/\quill-provider\tests\real_local_model.rs:22:    let models = provider.models().await.expect("应能列出模型");

$ grep -rn "probe_models" crates/ --include=*.rs
crates/\quill-server\src\api_providers.rs:277:    let (models, error) = match llm_providers::probe_models(&p.base_url, &p.api_key).await {
crates/\quill-server\src\api_providers.rs:318:        match llm_providers::probe_models(&p.base_url, &p.api_key).await {
crates/\quill-server\src\llm_providers.rs:421:pub async fn probe_models(base_url: &str, api_key: &str) -> Result<Vec<ModelCard>, String> {
```

判读：服务端管理页走 `llm_providers::probe_models`（`api_providers.rs:277/318`）；
`quill_provider::Provider::models()`（`openai.rs:178`）**只有 quill-provider 自己的测试在调**，
没有任何生产调用点。两套解析结果也不同形状：`ModelInfo { id, owned_by }`
（`quill-provider/src/types.rs:268-271`）vs `ModelCard { id, display_name, modality, context_window, owned_by }`
（`llm_providers.rs:240-246`）。

2. **base_url 归一重复**

```bash
$ grep -n "normalize_base_url\|trim_end_matches" crates/quill-provider/src/openai.rs crates/quill-server/src/llm.rs
crates/quill-provider/src/openai.rs:42:        let base_url = normalize_base_url(&base_url.into())?;
crates/quill-provider/src/openai.rs:187:fn normalize_base_url(raw: &str) -> Result<String, ProviderError> {
crates/quill-provider/src/openai.rs:199:    Ok(trimmed.trim_end_matches('/').to_string())
crates/quill-server/src/llm.rs:199:                t.trim_end_matches('/').to_string()
crates/quill-server/src/llm.rs:265:            base_url: self.base_url.trim().trim_end_matches('/').to_string()
```

3. **传输错误分类重复**（同一判定，两份实现：一份给 `ProviderError`，一份给中文字符串）

```bash
$ grep -n "is_timeout()\|is_connect()" crates/quill-server/src/llm_providers.rs crates/quill-provider/src/openai.rs
crates/quill-server/src/llm_providers.rs:438:            let cause = if e.is_timeout() {
crates/quill-server/src/llm_providers.rs:440:            } else if e.is_connect() {
crates/quill-provider/src/openai.rs:218:    if error.is_timeout() {
crates/quill-provider/src/openai.rs:223:    if error.is_connect() {
```

结论：**§1.6 的「2 套」成立，但性质是「传输层在 crate、配置/组装层在 server」**；
合并方向（ARCHITECTURE §3 P1-5「合并两套组装」）应是把 `probe_models` 也改走
`Provider` trait（顺便让 `ModelInfo` 补上 `context_window`），而不是把 crate 搬进 server。

### 3. 覆盖表

覆盖度判据只有两种：**代码在不在**（`ls`/`grep` 有命中）与**有没有生产调用点**（`grep` 调用方）。
「部分」＝能力存在但只覆盖一个子集（例如 trait 有 26 个方法而 quill 有 4 个）。

| # | 能力 / 模块 | goose 出处 | quill 落点 | 覆盖度 | 证据（可复现命令） |
|---|---|---|---|---|---|
| 1 | Provider trait | `goose-provider-types/src/base.rs:504-755`（26 个方法） | `crates/quill-provider/src/provider.rs:18-26`（4 个方法：`name`/`chat`/`stream`/`models`） | **部分** | `awk 'NR>=504&&NR<=755' vendor/goose/crates/goose-provider-types/src/base.rs \| grep -cE '^    (async )?fn '` → `26`；`awk 'NR>=18&&NR<=26' crates/quill-provider/src/provider.rs \| grep -cE '^    fn '` → `4` |
| 2 | Provider 元数据 / 描述符（display_name、config_keys、setup_steps、known_models） | `base.rs:30`（`pub struct ProviderMetadata`）、`base.rs:324`（`pub trait ProviderDescriptor`） | 无（quill 的 provider 元数据在 DB 行 + 前端 `presets.ts`） | **未覆盖** | `grep -rn "ProviderMetadata\|ProviderDescriptor" crates/ --include=*.rs` → `grep exit=1`（0 行） |
| 3 | Provider 注册表 + 工厂 | `goose/src/providers/provider_registry.rs:97`（`ProviderRegistry`）、`init.rs:56`（`static REGISTRY`）、`init.rs:275`（`pub async fn create`） | `crates/quill-server/src/llm_providers.rs:73-88`（`ProviderCache::default_provider`，只有默认项查询） | **部分** | `grep -n "pub struct ProviderRegistry\|static REGISTRY\|pub async fn create" vendor/goose/crates/goose/src/providers/{provider_registry.rs,init.rs}` |
| 4 | Provider 实例化（配置 → trait 对象） | `goose/src/providers/init.rs:275/280/289/313`（`create*`） | `crates/quill-server/src/llm.rs:205`（`build`） | **已覆盖**（但只 1 个后端） | `grep -rn "llm::build" crates/quill-server/src/` → 7 处；`sed -n '205,209p' crates/quill-server/src/llm.rs` |
| 5 | OpenAI 兼容 provider（chat/completions + SSE） | `goose-providers/src/openai.rs:123`（`OpenAiProvider`）、`:674`（`impl Provider`）、`openai_compatible.rs:31` | `crates/quill-provider/src/openai.rs:35`（`OpenAiCompatible`）、`:153`（`impl Provider`） | **部分**（无 thinking 保留、无 responses API、无 toolshim） | `ls crates/quill-provider/src/`；`grep -n "impl Provider for" crates/quill-provider/src/openai.rs` → `153:` |
| 6 | OpenAI Responses API 格式 | `goose-provider-types/src/formats/openai_responses.rs`（3124 行） | 无 | **未覆盖** | `ls vendor/goose/crates/goose-provider-types/src/formats/`；`grep -rn "responses" crates/quill-provider/src/` → `grep exit=1`（0 行） |
| 7 | Anthropic 原生 Messages | `goose-providers/src/anthropic.rs:377`（`fetch_supported_models`）、`goose-provider-types/src/formats/anthropic.rs`（3329 行） | 无（`Protocol::Anthropic` 只是标签，仍走 OpenAI 兼容口） | **未覆盖** | `grep -rn "anthropic" crates/quill-provider/src/` → 仅 3 行，全在 `wire.rs:708-715` 的 usage 字段形状测试；`sed -n '6,9p' crates/quill-server/src/llm.rs`（注释自认「后续真接 Anthropic 原生 Messages 时再按协议分流」） |
| 8 | Ollama 专有协议 | `goose-providers/src/ollama.rs:525`、`formats/ollama.rs`（451 行） | 无（`ui/web/src/models/presets.ts:121` 有 `preset_id: 'ollama'`，但只指向 OpenAI 兼容口） | **未覆盖** | `grep -n "ollama" ui/web/src/models/presets.ts`；`ls vendor/goose/crates/goose-providers/src/ollama.rs` |
| 9 | OpenRouter 专有格式 | `goose-providers/src/openrouter.rs`（1387 行）、`openrouter_format.rs`（374 行） | 无专有格式化（preset 存在） | **未覆盖** | `grep -n "openrouter" ui/web/src/models/presets.ts crates/quill-server/src/llm.rs` |
| 10 | Databricks / Azure Foundry / Google / Snowflake / Bedrock 等 | `goose-providers/src/{databricks.rs:1118行, databricks_v2.rs:1282行, azure_foundry.rs:1419行, google.rs:215行, snowflake.rs:397行}` | 无 | **未覆盖** | `wc -l vendor/goose/crates/goose-providers/src/{databricks,databricks_v2,azure_foundry,google,snowflake}.rs` |
| 11 | 声明式 / 自定义 provider | `goose-providers/src/declarative.rs:74`（`fixed_provider_configs`）、`:150`（`DeclarativeProviderConfig`）、`:337`（`load_custom_providers`）、`declarative/definitions/`（48 个 JSON） | `crates/quill-server/src/llm_providers.rs`（`ProviderKind::Preset/Custom`，DB 行）+ `ui/web/src/models/presets.ts`（11 个 preset） | **部分**（有自定义端点，无 JSON 声明文件加载、无 engine 分派） | `ls vendor/goose/crates/goose-providers/src/declarative/definitions/ \| wc -l` → `48`；`grep -c "preset_id:" ui/web/src/models/presets.ts` → `12`（含 1 处类型声明，实际 11 个 preset） |
| 12 | 模型发现（`/models`） | `base.rs:551`（`fetch_supported_models`）、`:555`（`fetch_supported_model_info`），各 provider 重写 | `llm_providers.rs:421`（`probe_models`）+ `quill-provider/src/openai.rs:178`（未接线） | **已覆盖，但两套**（见 §2） | `grep -rn "fn models_url" crates/`（2 处）；`grep -rn "\.models(" crates/`（无生产调用点） |
| 13 | 错误分类 | `goose-provider-types/src/errors.rs`（320 行）、`goose-providers/src/http_status.rs`（836 行） | `crates/quill-provider/src/error.rs`（347 行） | **部分** | `wc -l vendor/goose/crates/goose-providers/src/http_status.rs crates/quill-provider/src/error.rs` |
| 14 | 重试（退避循环） | `goose-provider-types/src/retry.rs`（358 行）、`goose/src/providers/mod.rs:106`（`pub use retry::{retry_operation, RetryConfig}`） | 只有重试**判定**：`crates/quill-provider/src/error.rs:42`（`is_retryable`，认 408/429/5xx），无循环 | **部分** | `grep -n "pub fn is_retryable" crates/quill-provider/src/error.rs`；`grep -rn "retry_operation" crates/ --include=*.rs` → 无输出 |
| 15 | 对话 / 消息类型 | `goose-provider-types/src/conversation.rs`（2016 行）、`conversation/message.rs`（2451 行）、`conversation/token_usage.rs`（288 行） | `crates/quill-provider/src/types.rs`（497 行：`Role`/`MessageContent`/`Message`/`ToolCall`/`ToolSpec`/`TokenUsage`/`ChatResponse`/`ModelInfo`/`ChatRequest`/`StreamDelta`） | **部分** | `grep -n "pub struct\|pub enum" crates/quill-provider/src/types.rs` |
| 16 | 多模态（图片 / 文档 / 音频） | `goose-provider-types/src/images.rs`（552 行）、`documents.rs` | 无（`MessageContent` 只有 `Text`/`ToolCalls`/`ToolResult`） | **未覆盖** | `sed -n '17,34p' crates/quill-provider/src/types.rs` |
| 17 | 线格式转换层 | `goose-provider-types/src/formats/`：openai 5899 / anthropic 3329 / openai_responses 3124 / databricks 2075 / google 1889 / snowflake 732 / ollama 451 | `crates/quill-provider/src/wire.rs`（1073 行，仅 OpenAI chat completions） | **部分** | `ls vendor/goose/crates/goose-provider-types/src/formats/`；`wc -l crates/quill-provider/src/wire.rs` |
| 18 | 流式 SSE 解码 + 聚合 | 各 provider 内部（`api_client.rs` 1103 行等） | `crates/quill-provider/src/sse.rs`（253 行）+ `pump.rs`（180 行） | **已覆盖** | `wc -l crates/quill-provider/src/{sse.rs,pump.rs}`；`grep -rn "pump_stream" crates/quill-provider/src/lib.rs` |
| 19 | 模型元数据 / 上下文窗口 | `goose-provider-types/src/model.rs`（1063 行 `ModelConfig`）、`context_limit.rs` | `llm_providers.rs:135-153`（`max_context_tokens` 由用户填 + 校验）、`:395-397`（探测 `context_window`） | **部分**（无已知模型目录，靠手填） | `grep -n "max_context_tokens" crates/quill-server/src/llm_providers.rs \| head`；`ls vendor/goose/crates/goose-provider-types/src/context_limit.rs` |
| 20 | thinking / reasoning | `goose-provider-types/src/thinking.rs`（742 行）、`base.rs:726`（`set_thinking_effort`） | 只读：`types.rs:239`（`ChatResponse.reasoning`）、`wire.rs:197/223`（`read_reasoning`）；`grep -rni "thinking" crates/quill-provider/src/` → `exit=1`（quill 用 `reasoning` 这个词，没有 thinking 概念） | **部分** | `grep -rn "reasoning" crates/quill-provider/src/types.rs crates/quill-provider/src/wire.rs \| head` → 8 行，全部是读取而非控制 |
| 21 | prompt cache 语义 | `goose-provider-types/src/cache_semantics.rs`（342 行） | 只在 usage 里读：`wire.rs:275-291`（`cache_read_input_tokens` 等） | **部分** | `grep -rn "cache_semantics\|cache_read" crates/quill-provider/src/ \| head` |
| 22 | 工具参数修补（toolshim） | `goose/src/providers/toolshim.rs`（2217 行） | 无 | **未覆盖** | `ls vendor/goose/crates/goose/src/providers/toolshim.rs`；`grep -rni "toolshim\|tool_shim" crates/ --include=*.rs` → 无输出 |
| 23 | 用量估算 / 成本 | `goose/src/providers/usage_estimator.rs`（128 行）、`canonical_cost.rs`（435 行）、`token_counter.rs` | `crates/quill-server/src/session_metrics.rs`（只聚合上游上报值，不估算） | **部分** | `grep -rni "estimate_usage\|token_counter" crates/ --include=*.rs` → 无输出 |
| 24 | live / 实时语音 | `goose-providers/src/live.rs`（298 行）、`openai_live.rs`（1265 行）、`openai_live_voice_provider.rs`（425 行）、`live_voice_provider.rs` | 无 | **未覆盖** | `wc -l vendor/goose/crates/goose-providers/src/{live.rs,openai_live.rs,openai_live_voice_provider.rs}` |
| 25 | HTTP 客户端 / TLS（mTLS、CA、OAuth 头） | `goose-providers/src/api_client.rs:25`（`ApiClient`）、`:45`（`AuthMethod`）、`:53`（`TlsCertKeyPair`） | `quill-provider/src/openai.rs` 内部裸 `reqwest::Client`（只有 Bearer + timeout `:63`） | **部分** | `grep -n "pub fn with_timeout\|Client::builder" crates/quill-provider/src/openai.rs` |
| 26 | 规范模型目录（models.dev / canonical） | `goose-provider-types/src/canonical/`（catalog.rs 631、name_builder.rs 629、models_dev.rs 243） | 无 | **未覆盖** | `grep -rniE "models_dev\|models\.dev" crates/ --include=*.rs` → `grep exit=1`（0 行）；`ls vendor/goose/crates/goose-provider-types/src/canonical/` → `catalog.rs data model.rs models_dev.rs name_builder.rs registry.rs` |
| 27 | 权限路由 | `goose-provider-types/src/permission.rs` | `quill-control`（多租户用户/角色，口径不同，见 ARCHITECTURE §1.3） | **部分**（口径不同） | `ls crates/quill-control/src/`；`ls vendor/goose/crates/goose-provider-types/src/permission.rs` |
| 28 | 运行模式（GooseMode） | `goose-provider-types/src/goose_mode.rs` | 无 | **未覆盖** | `grep -rni "goose_mode\|GooseMode" crates/ --include=*.rs` → 无输出 |
| 29 | 本地推理引擎 | `goose-local-inference` crate（8086 行）/`goose-providers/src/local_inference.rs`（34 字节，re-export） | `crates/quill-provider/src/local.rs`（83 行，仅指向 llama.cpp 的 OpenAI 兼容 HTTP 口） | **部分** | `wc -l crates/quill-provider/src/local.rs` → `83`；`for d in vendor/goose/crates/*/; do n=$(basename "$d"); printf "%-26s %6s\n" "$n" "$(cat "$d"/src/*.rs 2>/dev/null \| wc -l)"; done` → `goose-local-inference 8086` |
| 30 | 请求日志（可插拔 RequestLogger） | `goose-provider-types/src/request_log.rs:14`（`static LOGGER: OnceLock<Arc<dyn RequestLogger>>`） | 无（quill 侧是 `eprintln!` 前缀日志，如 `api_auth.rs:98`，无 provider 请求日志） | **未覆盖** | `grep -rni "request_log" crates/ --include=*.rs` → 无输出；`ls crates/quill-server/src/otel.rs` → `No such file or directory` |

统计（30 行，按覆盖度列逐行计数）：**已覆盖 2 行**（#4、#18）＋ **已覆盖但两套 1 行**（#12）；
**部分 15 行**（#1、#3、#5、#11、#13、#14、#15、#17、#19、#20、#21、#23、#25、#27、#29）；
**未覆盖 12 行**（#2、#6、#7、#8、#9、#10、#16、#22、#24、#26、#28、#30）。

### 4. 差集表（quill 缺的能力，按「值不值得补」排序）

差集 = 覆盖表里判为**未覆盖 / 部分**的 27 行（#1–#30 去掉已覆盖的 #4、#18），外加 #12 的
「两套重叠」问题（#12 本身算已覆盖，但重复实现该收口）。
排序依据：**能不能被用户直接撞上** > **运行稳定性** > **运维/成本** > **产品线是否已有**。
覆盖表 #1（trait 4/26 方法）是其余各行的**汇总口径**，不单列；每一行的括号里标回覆盖表编号。

| 序 | 缺的能力（覆盖表 #） | 值不值得补 | 一句理由 |
|---|---|---|---|
| 1 | Anthropic 原生 Messages（#7） | **值** | quill 已经给了 `anthropic` 这个可选 protocol（`ui/web/src/models/presets.ts:49`、`crates/quill-server/src/llm.rs:13`），选它却静默走 OpenAI 兼容口 —— 前端承诺与后端行为不一致，用户一选就中。 |
| 2 | 重试循环（#14） | **值** | 重试判定已经写好（`error.rs:42` 认 408/429/5xx）却没有循环，多 provider/限流场景下 429 会直接冒到用户；补的是「判定 → 退避重试」这一步，改动局部。 |
| 3 | `/models` 探测收口（#12 的重叠部分） | **值** | 同一件事两套实现（`probe_models` vs `Provider::models`），且 `Provider::models` 无生产调用点 —— 属于纯冗余，合并即减少一处漂移源（也即 ARCHITECTURE §3 P1-5）。 |
| 4 | 其它上游协议与格式层：Ollama / OpenRouter 专有格式 / Databricks / Azure Foundry / Google / Snowflake / OpenAI Responses API（#6、#8、#9、#10、#17） | **较值** | 这些上游都有 OpenAI 兼容口（quill 已能连上），缺的是**专有特性与专有鉴权**；要不要补取决于是否做多云。 |
| 5 | 已知模型目录 / 上下文窗口自动填（#19、#26） | **较值** | 现在靠用户手填 `max_context_tokens`（`llm_providers.rs:135-139` 的报错文案就是在教用户手填），有目录后可自动带出，降低配置错误率。 |
| 6 | 声明式 provider 目录（#11，goose 48 个 JSON ↔ quill 11 个 preset） | **中** | 自定义端点已经能用（`kind=custom`），缺的只是「目录」；补它是体验提升，不是能力缺口。 |
| 7 | 多模态（#16） | **中** | `MessageContent` 只有文本/工具三类，要不要看图取决于产品决策；不定决策前补了也会闲置。 |
| 8 | thinking 控制（#20） | **中低** | 只读 `reasoning` 已经够显示「预算被思考吃光」（`types.rs:260 truncated_by_reasoning`），设置 effort/budget 的收益只对推理模型成立。 |
| 9 | Provider 注册表 / 元数据 / 描述符（#2、#3） | **中低** | 缺的是架构整洁性与「用 UI 展示 provider 能力（config_keys / setup_steps）」的可能；quill 现在是 DB 行 + 前端目录，够用但不可枚举。 |
| 10 | 消息类型与错误分类的细化（#13、#15） | **中低** | 随序 4 的多协议一起补即可；单 OpenAI 兼容口径下现有 `MessageContent` 与 `ProviderError` 已够。 |
| 11 | prompt cache 语义（#21） | **低** | usage 里已经读到 cache_read/write（`wire.rs:275`），缺的是「按 provider/model 声明缓存语义」，只影响省钱账。 |
| 12 | HTTP 客户端 / TLS 配置（#25） | **低** | 单机 + 本地 llama.cpp 场景用不到 mTLS；接企业网关时才需要。 |
| 13 | toolshim（#22） | **低** | 只有不支持 function calling 的模型才需要（2217 行），quill 目标模型都支持。 |
| 14 | 用量估算 / 成本（#23） | **低** | 上游没报 usage 时 quill 选择留空而不是估算（`session_metrics.rs:88-99` 的规则 5、`ui/web/src/chat/chatApi.ts:50`「大量字段是 null —— 那是 quill 没记这一项，不是 0」），估算会把「不知道」变成「猜的数」，与既有铁律冲突。 |
| 15 | 本地推理引擎（#29） | **低** | llama.cpp / Ollama 已经能通过 OpenAI 兼容 HTTP 口用上，内嵌推理引擎只省一次进程管理。 |
| 16 | live / 实时语音（#24） | **最低** | quill 没有实时语音产品线，补了没有消费方。 |
| 17 | GooseMode / 权限路由（#27、#28） | **最低（不建议补）** | goose 的 `GooseMode` 与 provider 侧 `permission_routing` 服务于 goose 的模式开关；quill 的权限在 `quill-control` 且是多租户口径，直接搬会带来两套权限语义。 |
| 18 | 请求日志（#30） | **最低** | quill 侧是 `eprintln!` 前缀日志（`api_auth.rs:98` 一类），另起一套可插拔 RequestLogger 没有消费方。 |

### 5. 未验证清单（不做成结论的部分）

| # | 未验证项 | 原因 |
|---|---|---|
| 1 | goose 各内置斜杠命令（`/compact`、`/goal`、`/status` 等）的运行时行为 | 本仓库只 vendor goose 源码，没有 goose 可执行体与配置，无法真跑；本轮只核到接线与实现位置 |
| 2 | quill UI 在浏览器里对 `/` 的实际表现（浏览器快捷键、输入法组合态） | 只做了源码 grep 与 `sed` 读代码，没有启动前端在浏览器里实测 |
| 3 | `quill_provider::Provider::models()` 是否计划接线到管理页 | 代码里没有调用点；「为什么留着」属于规划问题，不是代码事实 |
| 4 | `databricks` / `azure_foundry` / `google` 等 provider 与 quill OpenAI 兼容实现的**行为兼容性** | 无对端凭据，未发起真实请求；只核了「有没有」 |
| 5 | 覆盖表里所有「部分」行的**行为等价性**（例如 quill 的 `Message` 与 goose 的 `MessageContentBlock` 是否逐字段等价） | Q029 的范围是覆盖度（有/没有/部分），不是逐行等价；等价性比对未做 |
| 6 | 覆盖表 #9「OpenRouter 专有格式化」对实际 OpenRouter 端点的必要性 | 未对 OpenRouter 发起请求，无法判断缺 `openrouter_format.rs` 会不会导致请求被拒 |
| 7 | `goose-provider-types` 真实行数口径（8150 vs 30822）在 `docs/ARCHITECTURE.md` 里应记哪个 | 两个口径都能复现；选哪个是文档口径决策，本文只并列记录 |

---

## 附：本文档四条硬性复现命令

```bash
$ grep -rn "slash_commands" vendor/goose/crates/goose/src/ | head
vendor/goose/crates/goose/src/\lib.rs:51:pub mod slash_commands;
vendor/goose/crates/goose/src/\acp\response_builder.rs:6:use crate::slash_commands::types::{SlashCommandEntry, SlashCommandSource};
vendor/goose/crates/goose/src/\acp\response_builder.rs:469:    if !crate::agents::execute_commands::slash_commands_enabled() {
vendor/goose/crates/goose/src/\acp\response_builder.rs:473:    crate::slash_commands::slash_command::list_acp_commands(working_dir)
vendor/goose/crates/goose/src/\acp\server\custom_dispatch.rs:738:    async fn dispatch_list_slash_commands(
vendor/goose/crates/goose/src/\acp\server\custom_dispatch.rs:742:        self.on_list_slash_commands(req).await
vendor/goose/crates/goose/src/\acp\server.rs:126:mod slash_commands;
vendor/goose/crates/goose/src/\acp\server\slash_commands.rs:5:    pub(super) async fn on_list_slash_commands(
vendor/goose/crates/goose/src/\acp\server\recipe\mod.rs:33:use crate::slash_commands::recipe_slash_command;
vendor/goose/crates/goose/src/\agents\execute_commands.rs:9:use crate::slash_commands::{recipe_slash_command, skill_slash_command};
```

```bash
$ ls vendor/goose/crates/goose-providers/src/
anthropic.rs
api_client.rs
azure_foundry.rs
browser_live_transport.rs
databricks.rs
databricks_auth.rs
databricks_v2.rs
decision.rs
declarative/
declarative.rs
google.rs
http_status.rs
lib.rs
live.rs
live_transport_websocket.rs
live_voice_provider.rs
local_inference.rs
ollama.rs
openai.rs
openai_compatible.rs
openai_live.rs
openai_live_voice_provider.rs
openrouter.rs
openrouter_format.rs
snowflake.rs
typesafe.rs
```

```bash
$ ls crates/quill-provider/src/
error.rs
lib.rs
local.rs
openai.rs
provider.rs
pump.rs
sse.rs
types.rs
wire.rs
```

```bash
$ grep -c "sqlx::query" crates/quill-server/src/llm_providers.rs
9
```

---

## Q027 权限模型

### 一句话结论

**两边的权限根本不是同一件事**：goose 的 `permission` 是**单用户 agent 对「工具调用」的许可系统**
（模式级 / 工具级 / 单次调用级 / 扩展级四类许可，外加「LLM 判读只读」「工具注解」两条软化路径）；
quill 的权限是 `quill-control` 的**多租户 API 角色鉴权**（`owner`/`member` 两种角色，只决定
能不能调 `/api/admin/*` 这类实例级接口）。**逐条比对后：goose 的 9 类许可（§1 表）在 quill 一个对应物都没有**；
任务里的待命题「quill 现在没有任何工具级/命令级许可」**属实**，§4 用三条命令证明。

### 1. goose 侧：许可在哪、几类

```bash
$ ls vendor/goose/crates/goose/src/permission/
mod.rs
permission_inspector.rs
permission_judge.rs
permission_store.rs

$ wc -l vendor/goose/crates/goose/src/permission/*.rs
   10 vendor/goose/crates/goose/src/permission/mod.rs
  395 vendor/goose/crates/goose/src/permission/permission_inspector.rs
  271 vendor/goose/crates/goose/src/permission/permission_judge.rs
  144 vendor/goose/crates/goose/src/permission/permission_store.rs
  820 total
```

```bash
$ grep -rn "PermissionLevel\|Permission::" vendor/goose/crates/ --include=*.rs | wc -l
224
```

许可词汇遍布 goose 主 crate、ACP 服务器与 CLI（`goose-cli/src/commands/configure.rs:1485`
有 `tool_permission` 配置项），不只 `permission/` 那 820 行。

逐类（`file:line` 均相对仓库根）：

| # | 许可类型 | 定义处（`file:line`） | 判定/落点 | 粒度 |
|---|---|---|---|---|
| 1 | 模式级 | `vendor/goose/crates/goose-provider-types/src/goose_mode.rs:22-32`：`Auto / Approve / SmartApprove / Chat` | 按模式分派：`permission/permission_inspector.rs:159-195`；模式随会话落库 `session/session_manager.rs:1054`（`goose_mode TEXT NOT NULL DEFAULT 'auto'`）、字段 `:90`；另有全局 `GOOSE_MODE`（`config/base.rs:1263`） | 会话 |
| 2 | 工具级（用户显式） | `config/permission.rs:21-25` `PermissionLevel{AlwaysAllow,AskBefore,NeverAllow}`；`:29-33` `PermissionConfig`；`:43-44` 两个类别 `user`/`smart_approve` | 查询 `:77-79` `get_user_permission`；落 `permission.yaml`（带文件锁 `:196-222`）；优先级 NeverAllow > AlwaysAllow > AskBefore（`:139-155`）；对外 ACP `acp/server/tools.rs:148` `on_set_tool_permissions` | 工具名 |
| 3 | 单次调用级（工具 + 参数哈希） | `permission/permission_store.rs:79-90` `check_permission`；`:117-127` `hash_tool_context`（blake3 哈希 arguments）；`:12-19` `ToolPermissionRecord{allowed,expiry,readable_context}` | 落 `permissions/tool_permissions.json`；过期清理 `:129-140` | 工具名 + **参数哈希** |
| 4 | 单次确认的即时决定 | `goose-provider-types/src/permission.rs:5-11` `Permission{AlwaysAllow,AllowOnce,Cancel,DenyOnce,AlwaysDeny}`；`:14-17` `PrincipalType{Extension,Tool}`；`:20-23` `PermissionConfirmation` | 路由 `agents/tool_confirmation_router.rs:19-56`；判到 `executable` `agents/state_machine/ops_tool_approval.rs:64,285-287`；`AlwaysAllow/AlwaysDeny` 回写工具级 `:67-76` | 本次请求；「总是」升格为工具级 |
| 5 | LLM 判读只读 | `permission/permission_judge.rs:145-185` `detect_read_only_requests`；判读工具 `:41-89` `platform__tool_by_tool_permission` | 候选收集在 `permission_inspector.rs:229-266`，出 Allow / RequireApproval | 单个请求（按 request id） |
| 6 | 工具注解 | `permission_inspector.rs:52-64` `apply_tool_annotations`（`read_only_hint`）；`config/permission.rs:91-107` 把 `read_only_hint=false` 记入 smart_approve | SmartApprove 下只读注解直接 Allow（`permission_inspector.rs:173-176`） | 工具名 |
| 7 | 扩展级 | `permission_inspector.rs:178-181`：`MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE` 一律要求审批；`config/permission.rs:280-294` `remove_extension`（按 `扩展名__` 前缀清）`:300-305` | — | 扩展（命名空间前缀） |
| 8 | 安全检查叠加 | `tool_inspection.rs:170-261` `apply_inspection_results_to_permissions`：非 permission inspector（security / egress / adversary）可把 Allow/RequireApproval 覆盖成 Deny | `permission_inspector.rs:116-128` | 单个请求 |
| 9 | 审批的用户形态 | `agents/state_machine/ops_tool_approval.rs:125-133` `Message::assistant().with_action_required(...).user_only()`；决定写回 `agents/state_machine/tool_confirmation.rs:75-117` | 落库，交给客户端 | 单个请求 |

关于「命令级 / 路径级」的核实：goose **没有独立的命令规则或路径规则表**。命令级/路径级粒度是
靠第 3 类的「工具名 + 参数哈希」顺带得到的 —— 同一 shell 工具、不同命令串是不同的键，
文件工具的不同路径也是不同的键。但对这一类要加一句实测限制：

```bash
$ grep -rn "record_permission\|check_permission\|ToolPermissionStore" vendor/goose/ --include=*.rs | grep -v "permission_store.rs"
vendor/goose/crates/goose/src/permission/mod.rs:10:pub use permission_store::ToolPermissionStore;
```

→ `ToolPermissionStore` 的读写方法在 `vendor/goose` 整棵树里**只被 re-export，没有任何调用点**。
即第 3 类在当前 vendored 版本里是**有实现、没接线**（是否被 vendor 之外的程序使用，未验证 ——
vendor 里没有证据）。

### 2. quill 侧：有什么（是角色鉴权，不是许可）

| quill 构件 | `file:line` | 作用 |
|---|---|---|
| 角色 `UserRole{Owner,Member}` | `crates/quill-control/src/user.rs:11-16`；schema `CHECK (role IN ('owner','member'))` `crates/quill-store/migrations/0001_init.sql:36` | 全系统只有两种角色 |
| 状态 `UserStatus{Active,Disabled}` | `crates/quill-control/src/user.rs:41-46`；`0001_init.sql:37` | 停用即令牌失效 |
| 鉴权上下文 `AuthContext{user_id,is_admin}` | `crates/quill-server/src/auth.rs:11-16` | `is_admin` 由角色推导 |
| `is_admin = role == Owner` | `crates/quill-server/src/auth.rs:235`（会话令牌）、`:347`（环境变量令牌回库核对） | 两条令牌源统一口径 |
| `RequireAdmin` extractor | `crates/quill-server/src/auth.rs:442-468` | 非 admin 一律 403；用于 `/api/admin/*`、provider 管理、备份 |
| owner 才能做的账户操作 | `crates/quill-control/src/service.rs:668-677` `require_owner`；调用点 `:274` 建用户、`:541` 撤销他人会话、`:558/:563` 看他人资料/列用户、`:574` 启停、`:587/:626/:637` 邀请码 | 角色鉴权的全部判据 |
| **不是鉴权**的中间件 | `crates/quill-server/src/middleware.rs:18-68` | 只做 panic 兜底 + 请求 ID 回填，不判身份 |
| 用户管理 HTTP | `crates/quill-server/src/api_users.rs:103-128`（list）、`:134-198`（set_status）；`:201-211` POST 501、`:214-224` DELETE 501 | 建号/删号有意不做 |

三个「像许可、其实不是许可」的字段，单独点名：

```bash
$ grep -rn "approval_mode" crates/ ui/web/src/ --include=*.rs --include=*.ts --include=*.tsx
crates/quill-server/src/api_auth.rs:382:            "approval_mode": "manual",
crates/quill-server/src/api_auth.rs:406:            "approval_mode": "manual",
crates/quill-server/src/api_auth.rs:419:                "approval_mode": "manual",
crates/quill-server/tests/http_contract.rs:169:        text.contains("\"approval_mode\":\"manual\""),
ui/web/src/api/types.ts:17:  approval_mode: string
ui/web/src/account/Account.tsx:86:                <dd>{user.approval_mode}</dd>
```

- `approval_mode` 是 `/api/auth/me` 里**写死的常量 `"manual"`**（`api_auth.rs:382/406/419`），
  前端只在 `ui/web/src/account/Account.tsx:86` 当一个词条展示。**它背后没有任何审批实现**：
  全仓库没有代码读它来决定行为，唯一的「消费者」是契约测试 `http_contract.rs:169` 与 UI 展示。
- `plugins.permissions_json`（`crates/quill-store/migrations/0001_init.sql:199`）**零读写**：
  `grep -rn "permissions_json" crates/ ui/ --include=*.rs --include=*.ts --include=*.tsx` 无输出；
  对应路由 `/api/extensions/plugins` 是 501（`crates/quill-server/src/routes.rs:203-205`）。
- `skills.tool_allowlist` 存得下、读得出（`crates/quill-server/src/skills_repo.rs:64,133,268-286`），
  但**工具挂载/执行完全不查它** —— `crates/quill-server/src/tools.rs:365` 自己的注释把这类问题
  叫「`tool_allowlist` 只存不用」；`with_skills`（`:299-338`）与 `call`（`:108-124`）都没有引用。
- MCP 服务器 `enabled`（`tools.rs:629-632`）与 SKILL `enabled`（`tools.rs:635-643`）是**按用户的
  功能开关**，不是许可：关掉=不挂，开着就无条件给模型调。

### 3. 逐条对齐：goose 的每一类 ↔ quill 有没有对应物

| goose 许可（§1 编号） | quill 对应物 | 判定 | 值不值得补 | 补的话落点 |
|---|---|---|---|---|
| 1 模式级 `GooseMode` | 无；`grep -rniE "goose_mode\|GooseMode" crates/` → 0 行 | **没有** | 值 | 会话加一列（照 `0001_init.sql:245-287` 风格迁移）+ 判定入口对齐 `permission_inspector.rs:159-195` 的分派 |
| 2 工具级 `PermissionLevel` | 无；`grep -rniE "always_allow\|ask_before\|never_allow\|PermissionLevel\|smart_approve" crates/ ui/web/src/` → 0 行 | **没有** | 值 | 新表 `user_id + tool_name + level`；过滤点 `tools.rs:79-103`（注册）与 `:108-124`（执行）；UI 落 `/api/extensions` |
| 3 单次调用级（工具+参数哈希） | 无 | **没有**（goose 自身也无调用点，见 §1 尾注） | 较值 | 与 #4 一起做；真正危险的是 MCP 工具（`tools.rs:348-412`），先做 MCP 白名单更划算 |
| 4 单次确认 `Permission{AllowOnce,DenyOnce,…}` | 无 | **没有** | 值 | 需要「暂停-请示-恢复」通道：quill 是同步 HTTP/SSE（`api_chat.rs:750-757`、`api_chat_stream.rs`），没有 goose `ActionRequired` 的落库-恢复机制（`ops_tool_approval.rs:125-133`）；补它要先有会话态（见 Q030 §3 #7）与前端确认条 |
| 5 LLM 判读只读 | 无 | **没有** | 中（可后置） | 依赖 #1/#2 先有；落点 `quill-core/src/state_machine.rs:169` 已有的 `AwaitingApproval` 模型 + 新审批 op |
| 6 工具注解 `read_only_hint` | 无（`ToolSpec` 只有 name/description/parameters） | **没有** | 中 | MCP `tools/list` 带回 annotations 时在 `tools.rs:571-587` `mcp_tool_spec` 保留；是 #5 的前置 |
| 7 扩展级审批 | 无 | **没有** | 中低 | 已有 skills/MCP 启停（`skills_repo.rs:204`、`mcp_repo.rs`），缺的是「装/删/改配置要 owner 确认」；可先复用 `RequireAdmin`（`auth.rs:446`） |
| 8 安全检查叠加 | 无 | **没有** | 中低 | 需先有 #2/#4 的判定链，否则没有可叠加的对象 |
| 9 审批的用户形态 | 无（响应里只有 `tool_calls` 轨迹，`api_chat.rs:1252`） | **没有** | 随 #4 | 同上 |
| — | quill 的角色鉴权（owner/member + `RequireAdmin`） | goose **没有对应物**（无多租户） | 不补（quill 产品面，来自 octop） | 保持 |

一句话读表：**「goose 的许可 → quill 的鉴权」不能一一映射**；两边只在「谁能不能调这个管理接口」
这一层重叠，而 goose 最核心的一层（agent 能否执行某个工具调用、以什么参数、要不要问用户）
在 quill 是**空的**。

### 4. 「quill 没有任何工具级/命令级许可」核实

```bash
$ grep -rniE "approval|permission" crates/ | wc -l
50

$ grep -rniE "approval|permission" crates/ | grep -vE "quill-backup[\\/]src[\\/]error.rs|quill-core[\\/]src[\\/]state_machine.rs"
crates/quill-server/src/api_auth.rs:382:            "approval_mode": "manual",
crates/quill-server/src/api_auth.rs:406:            "approval_mode": "manual",
crates/quill-server/src/api_auth.rs:419:                "approval_mode": "manual",
crates/quill-server/tests/http_contract.rs:169:        text.contains("\"approval_mode\":\"manual\""),
crates/quill-store/migrations/0001_init.sql:199:  permissions_json TEXT NOT NULL DEFAULT '[]',
```

分类（全量 50 行）：

- **21 行**是 `quill-backup/src/error.rs` 的 `io::ErrorKind::PermissionDenied`（文件系统错误码，不是许可）。
- **其余 24 行**全部落在 `quill-core/src/state_machine.rs` —— 那是**没有 I/O、没有 provider、
  没有工具、没有会话存储**的纯状态模型（文件头 `:1-5` 自述），里面 `AwaitingApproval` /
  `ApprovalRequested` 等只是把 goose 审批语义**画进状态机图**，没有任何判定实现。
- 去噪后只剩上面 5 行：3 条写死的 `approval_mode: "manual"` + 1 条契约断言 + 1 个零读写的
  `plugins.permissions_json` 列。

再证「工具执行路径无门禁」：

```bash
$ grep -rniE "always_allow|ask_before|never_allow|PermissionLevel|AllowOnce|DenyOnce|AlwaysDeny|AlwaysAllow|smart_approve|tool_permissions" crates/ ui/web/src/ --include=*.rs --include=*.ts --include=*.tsx
（无输出；grep exit=1）

$ grep -rn "prep.registry.call\|registry.call" crates/quill-server/src/api_chat.rs
1049:            let result = prep.registry.call(call);
```

`ToolRegistry::call`（`crates/quill-server/src/tools.rs:108-124`）只做一件事：查 handler，
查到就执行。它前面没有 `permission`/`approval`/`executable` 任何判定，执行前也没有询问用户
的分支；`api_chat.rs:1049` 是全仓库唯一的工具执行调用点。**所以「quill 现在没有任何工具级/
命令级许可」属实。**

### 5. 未验证清单（Q027）

| # | 未验证项 | 原因 |
|---|---|---|
| 1 | goose 审批链的端到端运行行为（`ActionRequired` 在客户端如何呈现、`AlwaysAllow` 何时落 `permission.yaml`） | 本仓只 vendored 源码，无 goose 可执行体与配置，无法真跑；本轮只核到实现与接线位置 |
| 2 | `ToolPermissionStore` 无调用点的原因 | 代码只能证明「vendor 整棵树里没有调用点」；「是不是废弃/待接线」属于历史与规划，不是代码事实 |
| 3 | `permission.yaml` 多进程并发下的一致性 | 有文件锁实现（`config/permission.rs:196-222`），未实跑并发场景 |
| 4 | `plugins.permissions_json` / `skills.tool_allowlist` 是否被仓库外的消费者使用 | 只核了本仓库；Octop 前端等外部消费者未查 |
| 5 | goose security/egress/adversary 三类 inspector 的判定细节与叠加顺序 | 本轮只核到「能覆盖 permission 结论」的机制（`tool_inspection.rs:170-261`），未逐条读 `security/*.rs` 的规则 |

---

## Q030 会话模型

### 一句话结论

goose 的 `session` 是**「会话 = 一段可重放的完整 message 流 + 会话级元数据」**（消息按
`(created_timestamp,id)` 排序、工具往返也是消息、可见性写在消息 metadata 里、对话状态由消息
尾部推导）；quill 的会话是**「会话 = 一张表 + 一问一答两条消息」**（工具往返不落库、无可见性、
`state` 列写了 `'IDLE'` 后再没动过）。**12 条维度里，等价 0 条、部分 5 条、缺失/语义相反 7 条**，
见 §3。

### 1. goose 侧

```bash
$ ls vendor/goose/crates/goose/src/session/
chat_history_search.rs
diagnostics.rs
export_markdown.rs
extension_data.rs
import_formats/
last_message_snippet.rs
legacy.rs
mod.rs
nostr_share.rs
session_manager.rs
session_naming.rs

$ grep -rn "fn " vendor/goose/crates/goose/src/session/mod.rs | head -20
（无输出；grep exit=1 —— mod.rs 只有 26 行，全部是模块声明与 re-export，没有函数）

$ sed -n '23,26p' vendor/goose/crates/goose/src/session/mod.rs
pub use session_manager::{
    Session, SessionInsights, SessionManager, SessionNameUpdate, SessionType, SessionUpdateBuilder,
};
```

关键事实（`file:line` 均相对仓库根）：

| 维度 | goose 出处 |
|---|---|
| `Session` 结构（24 个字段） | `vendor/goose/crates/goose/src/session/session_manager.rs:63-99`（`id` / `working_dir` / `name` / `user_set_name` / `session_type` / `created_at` / `updated_at` / `extension_data` / `usage` / `accumulated_usage` / `accumulated_cost` / `schedule_id` / `recipe` / `user_recipe_values` / `conversation` / `message_count` / `last_message_at` / `provider_name` / `model_config` / `goose_mode` / `archived_at` / `project_id` / `parent_session_id` / `last_message_snippet`） |
| 会话类型 | `:48-57` `SessionType{User,Scheduled,SubAgent,Hidden,Terminal,Gateway,Acp}`（全仓 117 处引用） |
| 建会话 / id 生成 | `:1620-1647` `INSERT INTO sessions (id, …) VALUES (? || '_' || MAX(SUBSTR(id,10))+1, …)`，即 `YYYYMMDD_N` |
| 存储 schema | `:1028-1058` `CREATE TABLE sessions`、`:1066-1080` `CREATE TABLE messages(role, content_json, created_timestamp, metadata_json)`、`:1084-1101` `usage_ledger`；索引 `:1103-1124` |
| 消息写入口 | `:1922-1964` `add_message`（写入前把 `created` 钳到 `MAX(created_timestamp)` 之后，注释 `:1927-1936` 说明「消息按 `(created_timestamp,id)` 读回」） |
| 整段替换（压缩用） | `:1967-2005` `replace_conversation_inner`；上层 `:460-472` |
| 读回（含消息） | `:433-435` `get_session(id, include_messages)`；`:1665-1690` |
| 软删 / 归档 | `:92` `archived_at`；`:1055` 列；写 `:301-303`；ACP `acp/server/manage_sessions.rs:273,291`；**列表不过滤** `:2085-2097` |
| 硬删 | `:2288-2318` `delete_session` 真删 messages → usage_ledger → sessions |
| 消息可见性 | `goose-provider-types/src/conversation/message.rs:829-833` `MessageMetadata{user_visible, agent_visible}`；`user_only()` `:894-898`；`with_visibility` `:1292`；列表按 `json_extract(metadata_json,'$.userVisible')` 计数 `session_manager.rs:743-745` |
| tool 往返在消息里 | `goose-provider-types/src/conversation/message.rs:318` `MessageContentBlock` 枚举；构造器 `tool_request :485`、`tool_response :526` |
| 跨会话记忆 | `session/chat_history_search.rs:50-90` `ChatHistorySearch::{new,execute}`（关键词 + 日期 + `exclude_session_id` + `session_types`） |
| 自动命名 | `:596-…` `maybe_update_name`；`session_naming.rs:10` `MSG_COUNT_FOR_SESSION_NAME_GENERATION = 3` |

### 2. quill 侧

```bash
$ grep -rn "CREATE TABLE sessions\|CREATE TABLE messages" crates/quill-store/
crates/quill-store/\migrations\0001_init.sql:46:CREATE TABLE sessions_auth (
crates/quill-store/\migrations\0001_init.sql:245:CREATE TABLE sessions (
crates/quill-store/\migrations\0001_init.sql:335:CREATE TABLE messages (
crates/quill-store/\migrations\0008_token_metrics.sql:43:CREATE TABLE messages_new (
```

`sessions` 列（`crates/quill-store/migrations/0001_init.sql:245-287`）：`user_id, id, kind, room_id,
team_id, parent_session_id, title, expert_id, provider_id, model, state, error_state, last_error,
replan_count, next_seq, summary, summary_upto_seq, compacted_count, checkpoint_key, checkpoint_path,
workspace_path, message_count, input_tokens, output_tokens, created_at, updated_at, last_active_at,
deleted_at`；`CHECK (kind IN ('solo','team_leader','team_member'))` `:276`；
`CHECK (state IN ('IDLE','PLANNING','DISPATCHING','COLLECTING','DELIVERING','ABORTED','idle','running','cancelling'))` `:277`。

`messages` 当前形状（以重建后的 `crates/quill-store/migrations/0008_token_metrics.sql:43-77` 为准）：
`user_id, id, session_id, seq, role, status, content, reasoning, expert_id, dispatch_id, tool_call_id,
tool_name, citations_json, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
turn_ms, error_code, created_at`；`CHECK (role IN ('user','assistant','system','tool_call','tool_result'))` `:68`；
`UNIQUE INDEX ux_messages_seq ON messages (user_id, session_id, seq)` `:97`。

读写口径全在 `crates/quill-server/src/chat_repo.rs`（SQL 常量 + 类型化行）；工具往返只在内存里，
最终只有正文落库（见 §3 #4）。

### 3. 逐条对齐（会话标识 / 消息模型 / seq / 状态 / 软删 / 上下文 / 专家绑定 / 跨会话记忆）

| # | 维度 | goose（`file:line`） | quill（`file:line`） | 判定 |
|---|---|---|---|---|
| 1 | 会话标识 | `Session.id: String` `session_manager.rs:64`；`YYYYMMDD_N`（`:1630-1647`）；`id TEXT PRIMARY KEY` `:1029` | `id BLOB` + `PRIMARY KEY(user_id,id)` + `CHECK(length(id)=16)` `0001_init.sql:247,274-275`；HTTP 层大写 32 位 hex（`chat_repo.rs:21` `hex(id)`；`api_chat.rs:210`） | **部分**（goose 人类可读全局唯一；quill 随机 16 字节按用户分区） |
| 2 | 租户归属 | 无 user 概念（`sessions` 无 user 列，`:1028-1058`） | `user_id BLOB NOT NULL`，所有读路径带 `user_id` 谓词（`chat_repo.rs:21-22,28-29,35-36`） | quill 多出（产品面差异，非缺口） |
| 3 | 消息角色 | `messages.role TEXT` `:1070`；`Message.role: Role` `message.rs:964`；工具在 content block（枚举 `:318`） | `role CHECK IN ('user','assistant','system','tool_call','tool_result')` `0008:68`；生产**只写** user/assistant（`api_chat.rs:820`、`:1206` 是全仓仅有两处 `role:` 写入） | **部分**（表预留 tool 角色，写入侧不用） |
| 4 | 工具往返 | 每次 tool request/result 独立入库（`:1943-1956`，content_json 存 block）；读回恢复（`:1665-1690`、`:1967-2005`） | 只在内存 `msgs`（`api_chat.rs:1035-1060`）；轨迹只进 HTTP 响应 `"tool_calls": tool_trace`（`api_chat.rs:1252`）；下一轮历史 `HISTORY_SQL` 只取 user/assistant（`chat_repo.rs:504-506`），`HISTORY_LIMIT = 40`（`api_chat.rs:21`） | **缺失** |
| 5 | 消息可见性 | `MessageMetadata{user_visible, agent_visible}` `message.rs:829-833`（`user_only :894`、`with_visibility :1292`）；列表按 `$.userVisible` 计数 `session_manager.rs:743-745` | 无可见性列；`status`（streaming/complete/interrupted/failed `0008:69`）是流式状态，不是可见性 | **缺失** |
| 6 | 排序 / `seq` | 无 seq 列；顺序 = `(created_timestamp,id)`（索引 `:1112-1116`；写时钳位 `:1927-1936`） | `seq` + `UNIQUE(user_id,session_id,seq)` `0008:97`；`sessions.next_seq` `0001:260`；写前 `MAX(seq)+1`（`chat_repo.rs:536-551`）；`touch_session` 推进到 `seq_assistant+1`、`message_count+2`（`:611-613`） | **等价物存在，语义不同**（显式序号 vs 时间戳序） |
| 7 | 状态 | 会话无状态列（`goose_mode` 是模式，不是状态，`:1054`）；对话状态从消息尾部**推导**（`goose-agent/src/machine.rs`；推导规则已被 `quill-core/src/state_machine.rs:24-70` 抄录） | `state` 9 值 CHECK `0001:277`，但**只在插入时写死 `'IDLE'`**（`chat_repo.rs:140`），此后全仓没有一条 UPDATE 改它；`error_state`/`replan_count` 在 Rust 侧零读写（`grep` 计数 0/0），`last_error` 唯一 1 处命中是测试函数名；`summary`/`summary_upto_seq`/`compacted_count`/`checkpoint_key`/`checkpoint_path` 同样零读写 | **缺失（列在、逻辑不在）** |
| 8 | 软删 | `archived_at`（`:92,1055`，写 `:301-303`，ACP `manage_sessions.rs:273,291`），但列表 WHERE 不含 archived（`:2085-2097`）；`delete_session` 是**硬删**（`:2288-2318`） | `deleted_at` 软删：读路径 `deleted_at IS NULL`（`chat_repo.rs:22,29`）；`SOFT_DELETE_SQL` 幂等（`:35-38`）；`api_chat.rs:291-335` 注释明说「消息记录保留」，无硬删入口 | **语义相反** |
| 9 | 上下文占用口径 | `Session.usage`/`accumulated_usage`（`:76-79`）+ `usage_ledger`（`:1084-1101`）+ 压缩（`goose-context-management` 整 crate；`save_compacted_conversation :464-472`） | 实测值 = 最后一条 assistant 的 `input_tokens`（`chat_repo.rs:238-241`）；`/api/sessions/{id}/context` = 实测值 + **字符数**分段（`api_chat.rs:377-470`、`session_metrics.rs:188-240`）；`compaction_threshold_tokens` **已接线**（2026-10-09，Q018）：阈值由壳侧 `quill-server/src/chat_compaction.rs` 的 `prepare_history()` 真读、纯逻辑在 `quill-core/src/compaction.rs`，界面标「已生效」并写明数字是估算值 | **部分** |
| 10 | 会话与「专家」绑定 | 无专家概念；有 `recipe` / `provider_name` / `model_config` / `extension_data`（`:80-88`）、`session_type`（`:48-57`）、子会话 `parent_session_id`（`:96`；创建 `orchestrator.rs:791-796`） | `expert_id`（`0001:253`；`SET_EXPERT_SQL chat_repo.rs:44`）、`kind`(solo/team_leader/team_member)、`team_id`、`parent_session_id`、`room_id`；专家=system 提示注入（`api_chat.rs:1284-1330`）；建会话默认挂通用专家（`api_chat.rs:184-188`） | **quill 多出 team/room（自研，goose 无多智能体）；goose 的 recipe/model_config/extension_data 在 quill 缺失** |
| 11 | 跨会话记忆 | `chat_history_search.rs:50-90` | 无：`grep -rn "chat_history_search\|search_chat_history" crates/ --include=*.rs` 无输出 | **缺失** |
| 12 | 会话命名 | `maybe_update_name`（`:596`）+ `session_naming.rs:10`（3 条消息后自动命名）+ `user_set_name`（`:69`） | `title` 建会话时由请求给、截 64 字符（`api_chat.rs:188-195`、`:643`），无自动命名、无 `user_set_name` | **部分** |

计数：**部分 5 条**（#1、#3、#9、#10、#12）＋ **缺失 5 条**（#4、#5、#7、#11 及 #2 的反向）＋
**语义相反 1 条**（#8）＋ **语义不同 1 条**（#6）；没有任何一条判「等价」。

### 4. `docs/ARCHITECTURE.md` §1.3 「会话模型是 `api_chat` 自研」具体差在哪

`docs/ARCHITECTURE.md:106` 那一行记的是：

```
| session | `goose/session` | `api_chat.rs` 自研 | 部分 |
```

对照 §1/§3，这里的「自研」与由此产生的「部分」，具体差在五处：

1. **最关键的差 —— 消息流不是 goose 意义上的完整会话**：工具往返**不入库**（§3 #4），
   于是「重放一段会话就能重建当时模型看到的全部上下文」在 quill 不成立；`messages` 表里
   只有一问一答。goose 的会话之所以能被压缩、导出、检索、审批回放，前提就是消息流完整。
2. **`seq` 是自研的显式序号**（goose 没有），且只按「一问一答 +2」推进（`chat_repo.rs:611-613`）
   —— 一旦要把工具消息入库，这个步进口径必须先改，否则 `UNIQUE(user_id,session_id,seq)`
   （`0008:97`）会挡住第二条 tool 消息。
3. **状态列是「照形状留的坑位」**：`state`/`error_state`/`replan_count`/`last_error`/
   `summary_upto_seq`/`compacted_count`/`checkpoint_*` 在 Rust 侧零读写（§3 #7），
   `quill-core/src/state_machine.rs` 只是一张**纯逻辑状态机模型**（文件头自述「没有 I/O、
   没有 provider、没有工具、没有会话存储」），没有接进 `api_chat`。
4. **没有消息可见性**（§3 #5）：quill 表达不了 goose 的 `user_only` / `agent_visible`，
   所以导出、回放、以及 Q027 §3 #4 的「审批只给用户看」都缺一个数据位。
5. **没有跨会话检索与压缩**（§3 #9/#11）：会话增长只能靠 `HISTORY_LIMIT = 40`
   （`api_chat.rs:21`）截断，压缩阈值配置只在库里躺着。

**第一步该动什么（按依赖顺序，只给第一步）：先让工具往返入 `messages`。**

理由：它是其余各项的数据基础，且 schema 已经预留好 —— `role` CHECK 已含 `'tool_call'`/
`'tool_result'`（`0001_init.sql:356`）、`tool_call_id`/`tool_name` 列已在（`:346-347`）。
具体三处：

- 写入：`crates/quill-server/src/api_chat.rs:1040-1060`（工具循环里在 `registry.call` 前后各落一条）；
- 读回：`crates/quill-server/src/chat_repo.rs:504-506`（`HISTORY_SQL` 的
  `role IN ('user','assistant')` 扩到含 tool 两类，并按 goose 的 content-block 形状拼回 `msgs`）；
- 序号：`crates/quill-server/src/chat_repo.rs:611-613`（`message_count + 2` 与
  `next_seq = seq_assistant + 1` 改成按实际落库条数推进）。

不先做这一步，后面无论压缩（压缩的对象就是完整消息流）还是可见性（可见性长在消息 metadata 上）
都没有可挂载的对象。

### 5. 未验证清单（Q030）

| # | 未验证项 | 原因 |
|---|---|---|
| 1 | goose `SessionManager` 的运行时行为（建会话 / 读回 / 压缩的真实表现） | 只 vendored 源码，无 goose 可执行体；本轮只核实现与接线位置 |
| 2 | goose `archived_at` 在 ACP/UI 上是否被更上层过滤 | 代码里 `list_sessions_matching`（`session_manager.rs:2085-2097`）不过滤 archived；「上层有没有再过滤」未逐层追完 |
| 3 | quill `state` 列是否被本仓库之外的进程写（Octop 前端、运维脚本） | 只核本仓库 Rust/SQL/TS；外部写入未验证 |
| 4 | goose 压缩（`goose-context-management`）与 `quill-core/src/compaction.rs` 的行为等价性 | Q030 只核「有/没有/部分」，等价性未比对（性质同 Q029 未验证 #5） |
| 5 | goose 子会话（`SessionType::SubAgent`）与 quill `team_member` 会话的语义是否可比 | 两边都只核到字段与创建点，未跑真实派工/子会话流程 |
