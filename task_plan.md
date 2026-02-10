# Task Plan: Gemini CLI + Antigravity 无痛切换与余量刷新落地

## Goal
在 `cc-switch` 中完成 Gemini 供应商能力落地：支持 Gemini CLI + Antigravity 多账号/多模型无痛切换，并支持余量刷新与前端展示闭环。

## Current Phase
Phase 1

## Phases
### Phase 1: 现状核对与问题定位
- [ ] 核对当前未提交改动与 Gemini 相关链路
- [ ] 识别切换不生效与刷新缺失的关键断点
- [ ] 记录结论到 findings.md
- **Status:** in_progress

### Phase 2: 后端切换与刷新链路补齐
- [ ] 补齐 Gemini CLI 与 Antigravity 双通道切换逻辑
- [ ] 保证多模型配置写入与读取一致
- [ ] 补齐/修正余量刷新数据来源
- **Status:** pending

### Phase 3: 前端表单与列表联动
- [ ] 修复 ProviderForm 元数据提交丢失问题
- [ ] Gemini 表单支持 Antigravity 与多模型
- [ ] Provider 列表刷新逻辑与文案完善
- **Status:** pending

### Phase 4: 验证与回归
- [ ] 运行类型检查与 Rust 编译检查
- [ ] 检查核心场景：新增、切换、刷新、展示
- [ ] 记录测试结果
- **Status:** pending

### Phase 5: 交付说明
- [ ] 汇总改动点与原因
- [ ] 输出验证结果与后续建议
- **Status:** pending

## Key Questions
1. 当前 Gemini 切换链路里，CLI 与 Antigravity 的分流条件是否可靠？
2. 多模型配置是否贯穿前端表单、持久化与切换应用？
3. 余量刷新是否覆盖所有 Gemini 供应商而非受限于单一脚本开关？

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| 使用文件化计划追踪复杂改造 | 多次中断后可快速恢复上下文，避免重复排查 |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
|       | 1       |            |

## Notes
- 每个阶段完成后更新状态与测试结论。
- 如果遇到同类错误，必须换路径而非重复同一操作。
