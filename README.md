# Queens 求解器

[![CI](https://github.com/CoconutHR/queens-puzzle-solver/actions/workflows/ci.yml/badge.svg)](https://github.com/CoconutHR/queens-puzzle-solver/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

一个面向 **Queens**（LinkedIn 上的皇后谜题）的命令行求解器。

程序采用人类解题的思路：先用逻辑推导一步步推进，据此给谜题评定难度；逻辑走不通时改用暴力搜索兜底，
验证谜题是否有解、是否唯一。

在 Queens 中，一个 *n×n* 的棋盘被划分为 *n* 个彩色区域，目标是放下 *n* 个皇后，使得：

- 每一行、每一列、每一个彩色区域都**恰好有一个**皇后；
- 任意两个皇后**互不相邻**（含斜向相邻）。

<p align="center">
  <img src="docs/example_game.png" alt="一个 7×7 Queens 谜题的求解中盘面" width="380">
</p>

<p align="center"><em>求解中的盘面：皇冠表示已放下的皇后，<code>x</code> 表示被排除的格子。</em></p>

## 处理链路

![求解链路](docs/pipeline.svg)

## 功能

- **逻辑求解器**：按难度递增应用人类式解题技巧，并按所用最难的技巧给出难度评级
  （Trivial / Easy / Medium / Hard）。
- **暴力兜底**：逻辑无法推进时枚举全部解，用于判定无解或多解。
- **命令行界面**：真彩色终端棋盘渲染，逐步给出解题结果。
- 支持从简洁的**文本格式**或 LinkedIn 谜题的**归档 JSON** 中读取题目。

> 终端棋盘使用真彩色区域背景渲染，建议使用支持 24 位色的终端查看。

## 构建

需要 [Rust 工具链](https://www.rust-lang.org/tools/install)。

```sh
git clone https://github.com/CoconutHR/queens-puzzle-solver
cd queens-puzzle
cargo build --release
```

可执行文件位于 `target/release/queens-puzzle`。下文的示例统一使用 `cargo run --` 调用。

## 用法

```
queens-puzzle [OPTIONS] <COMMAND>

Commands:
  solve  Solve a puzzle from a file and rate its difficulty

Options:
  -h, --help        Print help
  -V, --version     Print version
```

### 求解

求解文本格式的谜题：

```sh
cargo run -- solve puzzles/linkedin_20240926.txt
```

求解归档 JSON 中的谜题（默认取文件中 id 最小的那个，用 `--id` 指定其它题目）：

```sh
cargo run -- solve --json puzzles/linkedinPuzzles.json --id 353
```

输出依次为：原始盘面、解完的盘面、难度评级。

## 谜题文件格式

完整的格式规范见 [docs/formats.md](docs/formats.md)，涵盖文本格式、归档 JSON 与 canonical JSON。

## 求解原理

求解器反复扫描棋盘，寻找当前可应用的**最简单**技巧，应用后从头重新扫描，直到解出或再无技巧可用。
每一步都能给出人类可读的解释。难度即解题过程中所需的最难技巧：

| 难度 | 技巧 | 思路 |
|------|------|------|
| Trivial | Mark queen | 某行、列或区域只剩一个格子时，该格必为皇后。 |
| Trivial | Mark empty | 与皇后同行、同列、同区域或斜向相邻的格子必为空。 |
| Easy | Pointers | 若某区域剩余格子都落在同一行或列，则该行列其余格子必为空。 |
| Medium / Hard | Naked set | 某区块中 *N* 个格子必然包含皇后，可排除同区块其它格子。 |
| Hard | Hidden set | *N* 个区域若只落在 *N* 行或列内，则这些行列被它们独占，其余格子为空。 |

若逻辑卡住，**暴力求解器**会按列递归放置皇后（尊重已推导出的皇后），并报告找到的解。
唯一解但无法用逻辑推出时，评级为 `Requires guessing`；无解或多解时不给出评级。

## 谜题存档

[playqueensgame.com](https://www.playqueensgame.com) 收录了 LinkedIn 上出现过的全部谜题，
可按 `https://www.playqueensgame.com/api/daily?date=YYYY-MM-DD` 获取单日题目。
（`https://queensstorage.blob.core.windows.net/puzzles/linkedinPuzzles.json` 是另一个来源，但不完整。）

## 后续方向

按大致优先级排列：

- **更多的解题技巧**，以减少对暴力搜索的依赖：
  - 区域剩余未知格全部落在同一行或列时的规则；
  - 相邻的两个或三个候选格共同排除其公共邻居的规则；
  - 把 naked set 拆分为更具体、更易识别的几种情形。
- **记录求解器的变更列表**，以支持走子历史 / 撤销和更丰富的提示。
- **性能**：缓存 `queens()` 的结果，避免反复扫描棋盘。
- **重构**：把核心棋盘表示（网格、格子状态、IO、变更列表）拆成独立模块或 crate。
- **fetch 命令**：直接从存档 API 下载谜题。
- **截图导入**：识别粘贴的棋盘截图并提取区域布局。

## 许可证

基于 [MIT License](LICENSE) 发布。
