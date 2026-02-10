# Findings & Decisions

## Requirements
- Gemini 需要同时支持 Gemini CLI 与 Antigravity 账号切换。
- 切换要“无痛”：切换后应立即对目标客户端生效。
- 支持 Gemini 多模型配置（不止单模型）。
- 支持余量/token 刷新并在列表展示。

## Research Findings
- 当前工作区已有大量未提交改动，Gemini 相关代码已部分改造。
- `ProviderForm` 存在 `meta` 写入条件错误风险，会导致 `isPartner/partnerPromotionKey` 丢失。
- `ProviderList` 余量刷新纳入条件可能过窄，导致部分 Gemini 账号不参与刷新。

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| 先修数据链路（表单->存储->切换）再做展示优化 | 避免前端看起来可用但切换无效 |
| 保留兼容逻辑并增量修复 | 当前分支已存在大量改动，降低冲突风险 |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| 会话多次中断 | 引入 task_plan/findings/progress 持续记录 |

## Resources
- 代码基线：`/Users/zhengz/git/cc-switch`
- 参考仓库（用户提供）：`https://github.com/lbjlaq/Antigravity-Manager`

## Visual/Browser Findings
- 用户截图显示已存在“刷新余量”按钮，但切换生效与余量数据一致性仍有问题。
