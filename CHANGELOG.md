# Changelog

All notable changes to this project will be documented here.

## [Unreleased]

> 说明：本轮起新增的变更记录使用中文，历史版本记录保留原文。

### 移除
- **Web UI**（`web/`）：Play / Solve / Editor / Generator 四个页面、Playwright 测试与 GitHub Pages 部署
- **WASM 绑定**（`wasm/` crate）—— 唯一的使用方就是 Web UI
- **CLI `generate` 子命令**，CLI 变为纯求解器
- **谜题生成器**（`core/src/generator.rs`、`shuffle_queens`）：项目只保留求解能力。同时移除仅供生成器
  使用的 `assign_cell_region` / `unassign_cell_region`，以及随之不再需要的 `rand`、`log` 依赖
- **`.claude/` 目录**（上游作者的 AI 记忆文件）
- **失去调用方的残留接口**：`solver::next_hint`、`io::json::serialize` 与 `PuzzleJson::from_puzzle`
  （canonical JSON 导出）、`QueensPuzzle::cells_cardinally_adjacent`、CLI 中永不触发的规则高亮渲染分支
- **CLI 的 `-v/-vv` 与 `env_logger`**：生成器移除后全仓库已无日志输出点，输出统一改为直接打印

### 文档
- README、CLAUDE.md、docs/formats.md 改为中文说明
- 新增 `docs/pipeline.svg` 处理链路图，并在 README 中引用

### 新增
- **截图识别**（`io::image` 模块）：通过颜色梯度检测定位网格线和棋盘边界，提取 n×n 区域布局。
  对网格线与边距颜色无要求（白色、黑色、米色均可）。CLI 按 `.png` / `.jpg` / `.webp` 扩展名自动分派。
  测试用例：`puzzles/screenshot-1.png`（8×8 白线米底）
- 依赖：`core` 新增 `image = "0.25"`

## [0.2.0] - 2026-07-09

### Added
- **Web UI** deployed to GitHub Pages with four pages:
  - **Play** — interactive board with click-to-place queens, auto-cross, undo, timer, and share-by-URL
  - **Solve** — step through the solver's deduction chain with rule explanations and board highlights
  - **Editor** — paint custom region layouts, place queens manually, and export or share the result
  - **Generator** — run parallel background workers to generate and collect puzzles at chosen sizes
- **Hint system** on the Play page: highlights involved cells and dims the rest; distinguishes queen-placement from elimination hints visually
- **Share links**: puzzles are encoded as base64url JSON in the URL hash and decoded on load
- **Live uniqueness analysis** in the Editor: reports valid/invalid, solution count, and difficulty as you paint
- **Puzzle import**: paste or load canonical JSON directly into the Play or Editor page
- **WASM bindings** (`queens-puzzle-wasm` crate) wrapping the core library for use in the browser and web workers
- `RequiresGuessing` difficulty level for puzzles that exhaust all logical rules before reaching a solution
- GitHub Pages deployment via CI on every push to `main`
- Playwright end-to-end tests covering Play, Solve, Editor, Generator, and Analysis flows
- Git hooks via `.githooks/` (pre-commit: fmt + clippy; activated via `core.hooksPath`)
- Dependency audit CI job (`cargo audit` + `npm audit`)
- Content-Security-Policy header in the web app

### Fixed
- SPA routes now survive a hard reload on GitHub Pages (404 redirects to `index.html`)
- Timer no longer persists to localStorage after reset or when loading an already-solved puzzle
- Region border corners rendered correctly with SVG `strokeLinecap="square"` (eliminates CSS miter artifact)
- Worker instances are snapshotted before cleanup closures to prevent stale-reference bugs
- Hint-mode click on dimmed cells dismisses the hint and applies the click, except during queen-placement hints where it applies the click without dismissal

## [0.1.1] - 2026-06-16

### Added
- CI: `cargo fmt --check` and `cargo clippy -- -D warnings` on every push and pull request

## [0.1.0] - 2026-06-16

### Added
- Logical solver applying five techniques in order of increasing difficulty:
  - **Mark queen** (Trivial) — only cell left in a row, column, or region must be a queen
  - **Mark empty** (Trivial) — every cell adjacent to a queen (row, column, region, diagonal) must be empty
  - **Pointers** (Easy) — if a region's remaining cells share a row or column, the rest of that row/column is empty
  - **Naked set** (Medium/Hard) — *N* cells in a block that must collectively hold a queen eliminate other cells
  - **Hidden set** (Hard) — *N* regions confined to *N* rows/columns claim those rows/columns exclusively
- Brute-force fallback that enumerates all solutions when logic gets stuck
- Difficulty rating: Trivial, Easy, Medium, or Hard based on the hardest technique required
- Puzzle generator: places non-attacking queens at random then grows regions until all cells are assigned, guaranteeing exactly one solution
- CLI with `solve` and `generate` subcommands, `-v`/`-vv` verbosity, and true-colour terminal board rendering
- Text puzzle format and archived LinkedIn JSON format support
- CI: build and test workflow on every push and pull request
