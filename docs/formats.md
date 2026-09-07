# 谜题格式

## 文本格式

CLI 默认使用的格式。文件共 *n* 行，每行 *n* 个字符；每个字符是一个字母，代表该格所属的区域，
相同字母的格子属于同一区域。例如：

```
ppppppp
ooopppb
ggggggb
ggggwww
grggwww
gggywww
gggywww
```

由 `core/src/io/text.rs` 实现。

## 归档 JSON 格式

[LinkedIn Queens](https://www.archivedqueens.com/) 存档使用的 JSON：一个谜题数组，
每项包含 `id`、`regions`（每格的区域编号）以及答案 `grid`。

```json
[{ "id": 353, "date": "2025/04/18",
   "grid":    [[1,0,0,0,0,0,0], ...],
   "regions": [[0,0,0,0,0,0,1], ...] }]
```

实际只用到 `id` 与 `regions`，`grid` 被忽略；区域编号会在内部重新映射，因此不连续的编号也能处理。
CLI 通过 `--json` 启用该格式，并用 `--id` 选择题目（默认取 id 最小的那个）。

由 `core/src/io/json.rs` 实现。

## Canonical JSON 格式

谜题的通用交换格式：区域布局加上可选元数据。

```json
{
  "name":    "Wednesday's Puzzle",
  "source":  "Daniel Jones",
  "date":    "2026-06-29",
  "regions": [[0, 0, 1, 1], [0, 2, 1, 1], ...],
  "states":  [[0, 0, 0, 0], [0, 1, 2, 0], ...]
}
```

- `name` —— 谜题标题（可选）。
- `source` —— 出处 / 作者（可选）。
- `date` —— 谜题创建日期，ISO 8601，例如 `"2026-06-29"`（可选）。
- `regions[row][col]` —— 从 0 开始的区域编号；`null` 表示该格未分配。
- `states[row][col]` —— `0` 未知、`1` 皇后、`2` 空。
- 所有格子均为未知时（即尚未开始解题），`states` 可省略。
- `n` 由 `regions.length` 推断，棋盘恒为正方形。
- 以上可选字段求解器都会忽略，不会自行写入。

由 `core/src/io/json.rs` 实现；目前仅用于单元测试中内嵌盘面。
