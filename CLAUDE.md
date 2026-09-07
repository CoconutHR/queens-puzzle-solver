# CLAUDE.md — Queens 求解器

## 仓库结构

```
queens-puzzle/
├── src/        CLI 二进制（依赖 core；clap + colored 真彩色输出）
├── core/       库 crate —— 求解器、棋盘类型、IO
│   └── src/
│       ├── io/json.rs             canonical JSON 解析
│       ├── io/text.rs             文本网格格式
│       └── io/archived_queens.rs  LinkedIn 归档 JSON
├── docs/       formats.md（格式规范）、pipeline.svg（处理链路图）
└── puzzles/    示例谜题
```

根 `Cargo.toml` 同时是 workspace root 与 CLI 二进制包的清单。

本仓库只做求解：web 前端、WASM 绑定、生成器模块均已移除，不要重新引入。

## Git

- `origin` 指向上游 `daniel-jones-dev/queens-puzzle`，本机账号只有只读权限，直接推送会 403。
- 推送目标为 `fork`：https://github.com/CoconutHR/queens-puzzle-solver
  已设置 `branch.main.pushRemote = fork`，在 main 上直接 `git push` 即可。
- 提交信息使用英文、conventional commits 风格。

## 构建与校验

```bash
# 构建 CLI
cargo build --release

# 校验（CI 与 pre-commit 均按此执行）
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

注意：根包既是 workspace root 也是二进制包，`cargo test` / `cargo clippy` 默认只覆盖根包，
**必须**加 `--workspace` 才能覆盖 core（core 现有 15 个单元测试）。

## 谜题格式

规范见 [docs/formats.md](docs/formats.md)。处理链路见 `docs/pipeline.svg`，README 中引用了同一张图。
