-- 外部消息通道。一行 = 一条命名通道，按用户分组。
--
-- 结构对齐 Octop 的 channels 表
-- （.octop-ref/octop/src/octop/infra/db/repos/channels.py:11-20，只读对齐）：
-- channel_id 主键、owner_user_id 分组、config_json 存配置。
--
-- 为什么照它把配置塞进一个 JSON 列而不是拆成十几列：
-- 各通道的配置项根本对不上（微信是 accounts 数组，Discord 是 bot_token 加
-- 代理，QQ 还带一整块群组策略）。拆列的结果是十几列里大半恒为 NULL，
-- 且每加一个平台就要一次迁移。Octop 用 JSON 列，本项目跟进 —— 这是
-- 「不画没有后端的假开关」在存储层的对应做法：只有真用得上的字段才建列。
--
-- 两条写在明面上的纪律：
--
-- 1. `config_json` 里的凭据（微信 bot_token 等）只进不出。API 层
--    绝不把整个 JSON 回给浏览器，要回就拆出白名单字段。见
--    crates/quill-server/src/channels/store.rs 的 to_public。
--    本项目至今没有任何加密设施（连 LLM 的 api_key 都是明文，见 0002），
--    为通道单独引入加密会让「凭据怎么保护」有两个答案，所以不引入。
--
-- 2. 游标 `cursor_json` 必须持久化。iLink 的 get_updates_buf 是同步游标，
--    不回传或不落库，服务端会重放整个历史消息。这里的存的是**同步游标**，
--    不存运行状态 —— 进程重启后本来就该重连。

CREATE TABLE channels (
  channel_id     TEXT    PRIMARY KEY,
  owner_user_id  BLOB    NOT NULL,
  kind           TEXT    NOT NULL,
  name           TEXT    NOT NULL DEFAULT '',
  config_json    TEXT    NOT NULL DEFAULT '{}',
  enabled        INTEGER NOT NULL DEFAULT 0,

  -- iLink 同步游标 + 每个对端的 context_token。
  -- 这两样是**协议要求持久化**的，不属于「运行状态」：
  --  - 游标不落库 → 服务端重放整个历史消息；
  --  - context_token 不落库 → 重启后回复发不出去（iLink 要求原样带回，
  --    否则消息挂不上会话）。
  -- 两者都按通道存，不混进 config_json —— 它们是机器写的，
  -- config_json 是人改的，混在一起会把用户的编辑和轮询写入搅在一起。
  sync_state_json TEXT   NOT NULL DEFAULT '{}',

  created_at     INTEGER NOT NULL DEFAULT 0,
  updated_at     INTEGER NOT NULL DEFAULT 0,

  -- 「一个用户只能有一条同 kind 的通道」：否则前端存不住选中哪条，
  -- 而重复行在列表里看不出区别。允许多个微信账号是 config_json 里的事
  -- （accounts 数组），不是多行 —— 对齐 Octop 的口径。
  UNIQUE (owner_user_id, kind),

  CHECK (length(channel_id) BETWEEN 1 AND 64),
  CHECK (length(kind)       BETWEEN 1 AND 32),
  CHECK (length(name)       <= 128),
  CHECK (json_valid(config_json)),
  CHECK (json_valid(sync_state_json)),
  CHECK (length(config_json)   <= 65536),
  CHECK (length(sync_state_json) <= 65536),
  CHECK (enabled IN (0, 1)),
  CHECK (created_at >= 0),
  CHECK (updated_at >= 0),
  CHECK (owner_user_id IS NOT NULL)
) STRICT;

CREATE INDEX ix_channels_owner ON channels (owner_user_id);