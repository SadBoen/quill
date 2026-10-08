-- 删掉两张**死表**：`wiki_index` 与 `plugins`（queue Q104，源自 Q044 的审计）。
--
-- **为什么删**（三条都当场核过，不是照着条目的说法做）：
--   1. **零写者**：`grep -rn "INSERT INTO wiki_index\|INSERT INTO plugins" crates/`
--      只在 `quill-store/tests/schema_constraints.rs` 的夹具里命中 —— 任何真实部署里
--      这两张表都是空的，所以删它们**不丢任何数据**（红线「不做不可逆破坏」不适用）。
--   2. **零读者**：`grep -rn "FROM wiki_index\|FROM plugins" crates/` 同样只在测试里命中。
--   3. **没有上游对应物**：xu-wiki 是**文件式**的（没有 SQL，检索靠 ripgrep + scanner），
--      octop 也没有这两张表。所以它们不是「照谁抄的」，是 0001_init 里 quill 自己的
--      设计残留 —— 属于本仓最忌讳的那种「设计过但没接线」的结构。
--
-- **`wiki_index` 特别说明**：它被设计成一张 `WITHOUT ROWID` 的倒排索引（含约束与
-- 专门的用例）。但 quill 的资料库检索（Q035）**按设计**读的是 `index.md`
-- （`api_wiki.rs` 里那段「只读索引：不读页面正文、不调模型、不写任何文件」），
-- 所以这张表没有任何通往读者的路 —— 要么改掉那条契约去接它，要么删掉它。
-- 选删：那条契约是**有意的**（检索要快、要确定性），而这张表从未被写过。
-- 将来若真要上「按内容检索」，该做的是按 xu-wiki 的 scanner 重新设计，
-- 而不是复活一张从没写过的表（历史在 git 里，这条迁移也可以反向补回 CREATE TABLE）。
--
-- `plugins` 同理：quill 里**没有插件这个概念**，只有 `GET /api/extensions/plugins`
-- 那条**刻意**的诚实 501（Q037，前端 `capabilityGaps.ts` 如实登记）。
DROP TABLE wiki_index;

DROP TABLE plugins;
