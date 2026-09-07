use std::collections::HashMap;
use std::path::Path;

use image::{ImageReader, Rgb, RgbImage};

use crate::grid::Cell;
use crate::puzzle::QueensPuzzle;

// --- 策略一（饱和度 mask + 投影）阈值 ---

/// 像素被视为“棋盘彩色像素”所需的最小饱和度（max - min）。
/// 白线、米色背景、UI 文字都是低饱和度，棋盘格子是彩色。
const SAT_THRESHOLD: u8 = 18;

/// 行投影中，纵向候选区域所需的彩色像素比例。
const BOARD_CANDIDATE_THRESHOLD: f32 = 0.45;

/// 纵向候选区域的最小高度。
const MIN_CANDIDATE_HEIGHT: usize = 100;

/// 纵向候选区域允许合并的缺口（棋盘横向白线的宽度）。
const CANDIDATE_MERGE_GAP: usize = 20;

/// 行列投影中，单个格子所需的彩色像素比例。
const GRID_RUN_THRESHOLD: f32 = 0.40;

/// 单个格子的最小边长（像素）。
const MIN_CELL: usize = 10;

/// 棋盘相对正方形允许的比例误差。
const MAX_SQUARE_ERROR: f32 = 0.12;

/// 格子尺寸允许的极差比例。
const MAX_CELL_SPREAD: f32 = 0.18;

// --- 共用参数 ---

/// 颜色聚类距离（RGB 欧氏距离）。
const COLOR_CLUSTER_DISTANCE: f32 = 35.0;

/// 对格子中心多大比例的区域采样颜色。
const SAMPLE_RATIO: f32 = 0.50;

/// Read a Queens puzzle from an image file (PNG / JPEG / WebP) and return the corresponding puzzle.
///
/// Two complementary strategies are tried:
/// 1. Saturation mask + projection — robust for full-screen screenshots with UI
///    around the board (text, icons, buttons), because UI elements are largely
///    low-saturation while board cells are colored.
/// 2. Gradient-profile fallback — robust for tightly cropped boards and boards
///    with pale/pastel cells whose saturation is too low for strategy 1.
pub fn try_from_path(path: &Path) -> Result<QueensPuzzle, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| format!("failed to open image: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("failed to detect image format: {e}"))?;
    let img = reader
        .decode()
        .map_err(|e| format!("failed to decode image: {e}"))?
        .to_rgb8();
    analyze(&img)
}

/// Analyze a Queens screenshot and return the corresponding puzzle.
///
/// See [`try_from_path`] for the two strategies used. Grid geometry is expressed
/// as per-cell spans (`x_runs` / `y_runs`), which are then sampled and clustered
/// by color to recover the regions.
pub fn analyze(img: &RgbImage) -> Result<QueensPuzzle, String> {
    let (w, h) = img.dimensions();
    if w < 100 || h < 100 {
        return Err("image is too small".to_string());
    }

    if let Some(grid) = detect_by_saturation(img) {
        if let Ok(puzzle) = build_puzzle(img, &grid) {
            return Ok(puzzle);
        }
    }
    if let Some(grid) = detect_by_gradient(img) {
        if let Ok(puzzle) = build_puzzle(img, &grid) {
            return Ok(puzzle);
        }
    }
    Err("could not locate a Queens board in the image".to_string())
}

/// A located board: size plus the pixel span of each cell row/column.
struct Grid {
    size: usize,
    x_runs: Vec<(usize, usize)>,
    y_runs: Vec<(usize, usize)>,
}

// ============================================================
// 策略一：饱和度 mask + 行列投影
// ============================================================

fn detect_by_saturation(img: &RgbImage) -> Option<Grid> {
    let width = img.width() as usize;
    let height = img.height() as usize;

    let mask = create_color_mask(img);
    let row_proj = row_projection(&mask, width, height);

    // 棋盘横向白线会把纵向连续区域切开，所以允许合并小缺口。
    let candidates = find_runs(
        &row_proj,
        BOARD_CANDIDATE_THRESHOLD,
        MIN_CANDIDATE_HEIGHT,
        CANDIDATE_MERGE_GAP,
    );
    if candidates.is_empty() {
        return None;
    }

    let mut best: Option<(Grid, f32)> = None;

    for &(top, bottom) in &candidates {
        if bottom - top + 1 < MIN_CANDIDATE_HEIGHT {
            continue;
        }

        // 列投影不能合并缺口 —— 格子之间正是靠白色分隔线切开的。
        let col_proj = col_projection(&mask, width, top, bottom);
        let x_runs = find_runs(&col_proj, GRID_RUN_THRESHOLD, MIN_CELL, 0);

        let left = x_runs.first()?.0;
        let right = x_runs.last()?.1;

        let local_rows = local_row_projection(&mask, width, left, right, top, bottom);
        let y_runs = find_runs(&local_rows, GRID_RUN_THRESHOLD, MIN_CELL, 0);

        // 必须是 N × N
        if x_runs.len() != y_runs.len() || !(4..=16).contains(&x_runs.len()) {
            continue;
        }
        let size = x_runs.len();

        // 棋盘必须接近正方形
        let bw = (right - left + 1) as f32;
        let bh = (bottom - top + 1) as f32;
        let square_error = (bw - bh).abs() / bw.max(bh);
        if square_error > MAX_SQUARE_ERROR {
            continue;
        }

        let widths: Vec<usize> = x_runs.iter().map(|&(a, b)| b - a + 1).collect();
        let heights: Vec<usize> = y_runs.iter().map(|&(a, b)| b - a + 1).collect();
        if !is_uniform(&widths) || !is_uniform(&heights) {
            continue;
        }

        let score = uniformity_score(&widths)
            * uniformity_score(&heights)
            * (1.0 - square_error)
            * size as f32;

        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            // y_runs 是相对 top 的偏移，这里统一转成绝对坐标。
            let abs_y_runs = y_runs.iter().map(|&(a, b)| (a + top, b + top)).collect();
            best = Some((
                Grid {
                    size,
                    x_runs: x_runs.clone(),
                    y_runs: abs_y_runs,
                },
                score,
            ));
        }
    }

    best.map(|(g, _)| g)
}

fn create_color_mask(img: &RgbImage) -> Vec<u8> {
    let width = img.width() as usize;
    let height = img.height() as usize;
    let mut mask = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x as u32, y as u32);
            let max = p[0].max(p[1]).max(p[2]);
            let min = p[0].min(p[1]).min(p[2]);
            if max - min >= SAT_THRESHOLD {
                mask[y * width + x] = 1;
            }
        }
    }
    mask
}

fn row_projection(mask: &[u8], width: usize, height: usize) -> Vec<f32> {
    (0..height)
        .map(|y| {
            let count = (0..width)
                .map(|x| mask[y * width + x] as usize)
                .sum::<usize>();
            count as f32 / width as f32
        })
        .collect()
}

fn col_projection(mask: &[u8], width: usize, top: usize, bottom: usize) -> Vec<f32> {
    let height = bottom - top + 1;
    (0..width)
        .map(|x| {
            let count = (top..=bottom)
                .map(|y| mask[y * width + x] as usize)
                .sum::<usize>();
            count as f32 / height as f32
        })
        .collect()
}

fn local_row_projection(
    mask: &[u8],
    image_width: usize,
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
) -> Vec<f32> {
    let width = right - left + 1;
    (top..=bottom)
        .map(|y| {
            let count = (left..=right)
                .map(|x| mask[y * image_width + x] as usize)
                .sum::<usize>();
            count as f32 / width as f32
        })
        .collect()
}

fn find_runs(
    values: &[f32],
    threshold: f32,
    min_length: usize,
    merge_gap: usize,
) -> Vec<(usize, usize)> {
    let mut runs = Vec::<(usize, usize)>::new();
    let mut start: Option<usize> = None;

    for (i, &value) in values.iter().enumerate() {
        match (start, value >= threshold) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                let end = i - 1;
                if end - s + 1 >= min_length {
                    runs.push((s, end));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        let end = values.len() - 1;
        if end - s + 1 >= min_length {
            runs.push((s, end));
        }
    }

    if runs.is_empty() || merge_gap == 0 {
        return runs;
    }

    let mut merged = Vec::<(usize, usize)>::new();
    let mut current = runs[0];
    for &next in runs.iter().skip(1) {
        if next.0.saturating_sub(current.1 + 1) <= merge_gap {
            current.1 = next.1;
        } else {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);
    merged
}

fn is_uniform(values: &[usize]) -> bool {
    if values.is_empty() {
        return false;
    }
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();
    if min == 0 {
        return false;
    }
    (max - min) as f32 / (min as f32) < MAX_CELL_SPREAD
}

fn uniformity_score(values: &[usize]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<usize>() as f32 / values.len() as f32;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|v| (*v as f32 - mean).powi(2))
        .sum::<f32>()
        / values.len() as f32;
    let coefficient = variance.sqrt() / mean;
    (1.0 - coefficient * 3.0).clamp(0.0, 1.0)
}

// ============================================================
// 策略二：梯度剖面回退
// ============================================================

fn detect_by_gradient(img: &RgbImage) -> Option<Grid> {
    let (w, h) = img.dimensions();
    let h_profile = gradient_rows(img);
    let v_profile = gradient_cols(img);

    let (h_start, h_end, h_cell) = find_board_and_cell_size(&h_profile, h)?;
    let (v_start, v_end, v_cell) = find_board_and_cell_size(&v_profile, w)?;

    // 棋盘是正方形，两个方向格子大小应当一致。
    let cell = h_cell.min(v_cell);
    if (h_cell as i32 - v_cell as i32).abs() > cell as i32 / 4 {
        return None;
    }

    let n_h = ((h_end - h_start) as f64 / cell as f64).round() as usize;
    let n_w = ((v_end - v_start) as f64 / cell as f64).round() as usize;
    if n_h != n_w || !(4..=16).contains(&n_h) {
        return None;
    }
    let size = n_h;

    let c = cell as usize;
    let x_runs: Vec<(usize, usize)> = (0..size)
        .map(|i| {
            let s = v_start as usize + i * c;
            (s, s + c - 1)
        })
        .collect();
    let y_runs: Vec<(usize, usize)> = (0..size)
        .map(|i| {
            let s = h_start as usize + i * c;
            (s, s + c - 1)
        })
        .collect();

    // 越界保护
    if x_runs.last()?.1 >= w as usize || y_runs.last()?.1 >= h as usize {
        return None;
    }

    Some(Grid {
        size,
        x_runs,
        y_runs,
    })
}

fn color_dist_sq(a: &Rgb<u8>, b: &Rgb<u8>) -> u64 {
    let dr = a[0] as i64 - b[0] as i64;
    let dg = a[1] as i64 - b[1] as i64;
    let db = a[2] as i64 - b[2] as i64;
    (dr * dr + dg * dg + db * db) as u64
}

/// 相邻行之间的色差平方和；网格线处出现峰值。
fn gradient_rows(img: &RgbImage) -> Vec<u64> {
    let (w, h) = img.dimensions();
    (0..h.saturating_sub(1))
        .map(|y| {
            (0..w)
                .map(|x| color_dist_sq(img.get_pixel(x, y), img.get_pixel(x, y + 1)))
                .sum()
        })
        .collect()
}

/// 相邻列之间的色差平方和；网格线处出现峰值。
fn gradient_cols(img: &RgbImage) -> Vec<u64> {
    let (w, h) = img.dimensions();
    (0..w.saturating_sub(1))
        .map(|x| {
            (0..h)
                .map(|y| color_dist_sq(img.get_pixel(x, y), img.get_pixel(x + 1, y)))
                .sum()
        })
        .collect()
}

/// 用梯度剖面的显著峰定位棋盘边界与格子大小。
///
/// 第一个峰和最后一个峰是棋盘与外边距的过渡（边界），中间的峰是内部网格线；
/// 内部峰间距的中位数即格子大小。
fn find_board_and_cell_size(profile: &[u64], dim: u32) -> Option<(u32, u32, u32)> {
    if profile.is_empty() || dim == 0 {
        return None;
    }

    let mut sorted: Vec<u64> = profile.to_vec();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let threshold = median * 5 + 1000;

    let mut peaks: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < profile.len() {
        if profile[i] > threshold {
            let mut sum: u64 = 0;
            let mut count: u64 = 0;
            while i < profile.len() && profile[i] > threshold {
                sum += i as u64;
                count += 1;
                i += 1;
            }
            peaks.push((sum / count) as u32);
        } else {
            i += 1;
        }
    }

    if peaks.len() < 3 {
        return None;
    }

    let start = peaks[0];
    let end = peaks[peaks.len() - 1];

    let internal = &peaks[1..peaks.len() - 1];
    let mut spacings: Vec<u32> = internal.windows(2).map(|w| w[1] - w[0]).collect();
    if spacings.is_empty() {
        return None;
    }
    spacings.sort_unstable();
    let cell_size = spacings[spacings.len() / 2];
    if cell_size == 0 {
        return None;
    }

    Some((start, end, cell_size))
}

// ============================================================
// 采样、聚类、构建谜题
// ============================================================

fn build_puzzle(img: &RgbImage, grid: &Grid) -> Result<QueensPuzzle, String> {
    let n = grid.size;
    let mut samples: Vec<Rgb<u8>> = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            samples.push(sample_cell_color(
                img,
                grid.x_runs[col].0 as u32,
                grid.y_runs[row].0 as u32,
                grid.x_runs[col].1 as u32,
                grid.y_runs[row].1 as u32,
            ));
        }
    }

    let (_, color_ids) = cluster_colors(&samples, COLOR_CLUSTER_DISTANCE);

    let mut region_cells: Vec<Vec<Cell>> = Vec::new();
    let mut color_to_region: HashMap<usize, usize> = HashMap::new();
    for (i, &cid) in color_ids.iter().enumerate() {
        let row = i / n;
        let col = i % n;
        let region = match color_to_region.get(&cid) {
            Some(&r) => r,
            None => {
                let r = region_cells.len();
                color_to_region.insert(cid, r);
                region_cells.push(Vec::new());
                r
            }
        };
        region_cells[region].push(Cell { row, col });
    }

    if region_cells.len() != n {
        return Err(format!(
            "expected {n} distinct regions, found {}",
            region_cells.len()
        ));
    }

    Ok(QueensPuzzle::new(region_cells))
}

/// 对格子中心 `SAMPLE_RATIO` 比例的区域取平均色，避开格子边缘和白线。
fn sample_cell_color(img: &RgbImage, left: u32, top: u32, right: u32, bottom: u32) -> Rgb<u8> {
    let width = (right - left + 1) as f32;
    let height = (bottom - top + 1) as f32;
    let sw = width * SAMPLE_RATIO;
    let sh = height * SAMPLE_RATIO;
    let cx = (left + right) as f32 / 2.0;
    let cy = (top + bottom) as f32 / 2.0;

    let l = ((cx - sw / 2.0).max(left as f32)) as u32;
    let r = ((cx + sw / 2.0).min(right as f32)) as u32;
    let t = ((cy - sh / 2.0).max(top as f32)) as u32;
    let b = ((cy + sh / 2.0).min(bottom as f32)) as u32;

    let mut sr = 0u64;
    let mut sg = 0u64;
    let mut sb = 0u64;
    let mut count = 0u64;
    for y in t..=b {
        for x in l..=r {
            let p = img.get_pixel(x, y);
            sr += p[0] as u64;
            sg += p[1] as u64;
            sb += p[2] as u64;
            count += 1;
        }
    }
    if count == 0 {
        let p = img.get_pixel((left + right) / 2, (top + bottom) / 2);
        return Rgb([p[0], p[1], p[2]]);
    }
    Rgb([(sr / count) as u8, (sg / count) as u8, (sb / count) as u8])
}

/// 增量式颜色聚类：与已有中心距离在阈值内则归入该类并更新中心，否则新建一类。
fn cluster_colors(samples: &[Rgb<u8>], threshold: f32) -> (Vec<Rgb<u8>>, Vec<usize>) {
    let mut palette: Vec<Rgb<u8>> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut ids = Vec::with_capacity(samples.len());

    for &sample in samples {
        let mut best_id = None;
        let mut best_distance = f32::MAX;
        for (i, &center) in palette.iter().enumerate() {
            let d = color_distance(sample, center);
            if d < best_distance {
                best_distance = d;
                best_id = Some(i);
            }
        }
        match best_id {
            Some(id) if best_distance <= threshold => {
                let n = counts[id] as f32;
                let old = palette[id];
                palette[id] = Rgb([
                    ((old[0] as f32 * n + sample[0] as f32) / (n + 1.0)).round() as u8,
                    ((old[1] as f32 * n + sample[1] as f32) / (n + 1.0)).round() as u8,
                    ((old[2] as f32 * n + sample[2] as f32) / (n + 1.0)).round() as u8,
                ]);
                counts[id] += 1;
                ids.push(id);
            }
            _ => {
                ids.push(palette.len());
                palette.push(sample);
                counts.push(1);
            }
        }
    }
    (palette, ids)
}

fn color_distance(a: Rgb<u8>, b: Rgb<u8>) -> f32 {
    let dr = a[0] as f32 - b[0] as f32;
    let dg = a[1] as f32 - b[1] as f32;
    let db = a[2] as f32 - b[2] as f32;
    (dr * dr + dg * dg + db * db).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_cropped_screenshot_with_white_grid() {
        // 裁剪好的 8×8 棋盘：白线 + 米色边距，格子偏浅（pastel）。
        // 饱和度策略不适用，应回退到梯度策略。
        let path = std::path::PathBuf::from("../puzzles/screenshot-1.png");
        let puzzle = try_from_path(&path).expect("should parse cropped screenshot");
        assert_eq!(puzzle.n(), 8);
    }

    #[test]
    fn analyze_full_screen_screenshot() {
        // 完整手机截图：棋盘周围有关卡号、分数、图标、按钮等 UI。
        // 饱和度策略应当直接命中，无需手动裁剪。
        let path = std::path::PathBuf::from("../puzzles/screenshot-full.png");
        let puzzle = try_from_path(&path).expect("should parse full-screen screenshot");
        assert_eq!(puzzle.n(), 8);
    }

    #[test]
    fn analyze_full_screen_screenshot_10x10() {
        // 完整手机截图的另一关：10×10 棋盘，验证尺寸检测在较大棋盘上也成立。
        let path = std::path::PathBuf::from("../puzzles/screenshot-10x10.png");
        let puzzle = try_from_path(&path).expect("should parse 10x10 full-screen screenshot");
        assert_eq!(puzzle.n(), 10);
    }
}
