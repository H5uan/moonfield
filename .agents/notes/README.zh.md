# Agent Notes

[English](README.md) | 中文

这里存放唯一一种设计文档:**Agent Note(代理笔记)**,记录影响本代码库的决策或提案——代码和文档承载不了的"为什么"和"我们放弃了什么"。本文件定义 Agent Note 的位置、何时写、以及文件内格式。

## 布局与命名

每条 Agent Note 都有两个轴,都编码在路径里——`{lifecycle}/{class}/yyyy-mm-dd-topic-title.md`:

- **生命周期**(顶级目录)是笔记的状态:
  - `proposed/` — 实现前待评审的提案。
  - `implemented/` — 已落地的决策,与真实实现保持同步。
  - `rejected/` — 考虑过并否决;仅在其理由能阻止一个诱人的、有意义的错误时保留。
- **分类**(嵌套目录)是决策的种类。下面的封闭集合由 `scripts/verify_agents.py` 强制,未知文件夹会让门禁失败;新增分类需同时更新此处集合与脚本。

| 分类 | 覆盖内容 |
|---|---|
| `feature` | 新的用户或模型可见能力。 |
| `bug-fix` | 修复缺陷,或补上复盘暴露的缺口。 |
| `simplification` | 在不新增能力的前提下删除代码、行为或表面。 |
| `architecture` | 已交付源码的结构性决策。 |
| `process` | 围绕代码的工具、策略或工作流——不是运行时行为。 |
| `testing` | 测试基础设施与策略。 |

文件名中的日期是主题**首次提出**的日期。笔记之间的交叉引用使用相对 Markdown 链接——绝不使用裸文字——从而可被机械检查。

## 何时写

每个**非平凡改动**必须在同一提交(和同一 Pull Request)中新增或更新至少一条 Agent Note。非平凡指:改变了行为、架构、跨文件或跨 crate 的契约、流程或工具、测试策略,或磁盘/线上/配置格式。面向未来的重大工作提案从 `proposed/` 开始;已成事实的决策从 `implemented/` 开始。只有纯机械或局部改动才豁免。更新已拥有该决策的笔记即满足规则——绝不创建重复笔记。

CI 门禁只验证已有笔记的格式;改动是否非平凡由作者判断。`dsh-pre-push-checks` 会提醒作者。

## 文件内格式

`scripts/verify_agents.py` 强制格式、分类、生命周期一致性、双语配对与相对链接;CI 在每次 push 和 PR 时运行。

### 头部块

每条笔记前两行 Exactly:

```markdown
# Agent Note: <title>

Status: <status>
```

`Status:` 需要与生命周期目录一致:`proposed`、`implemented`,或 `rejected — <原因,一行>`。

### 正文骨架

- `proposed/`:

```markdown
## Problem
## Proposal
## Alternatives considered
## Acceptance criteria
## Risks
```

- `implemented/`——以现在时描述已实现的事实;门禁拒绝 spec 词(`Proposal`/`Plan`/`Acceptance criteria`):

```markdown
## Problem
## Decision
## Alternatives considered
## Consequences
```

- `rejected/`——提案本身冻结;判决写在 `Status:` 行。

`## Alternatives considered` 强制存在于每条笔记:为每个真实备选写一段加粗开头的原因说明。绝不事后编造备选。

### 中文对照

每条笔记带一个 `.zh.md` 镜像(逐节对应)和一个 `.i18n.yaml` 一致性记录(en/zh 文件名 + 英文文件的 `sha256`)。机器校验的头部 token——`# Agent Note: ` 和 `Status:`——在中文文件里保持英文原样。三个文件必须一起更新。

## 生命周期迁移

在生命周期目录间移动文件意味着在同一个改动里更新 `Status:` 行并重新满足该目录的骨架。`proposed/` → `implemented/` 把提案改写成现在时的已实现事实,把验收标准与风险折叠进后果。`proposed/` → `rejected/` 只需在 `Status:` 行补上理由。

## 验证

```sh
python3 scripts/verify_agents.py
```

本地跑全部门禁;CI 在 `agent-docs` job 中运行。