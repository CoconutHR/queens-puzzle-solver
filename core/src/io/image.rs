use std::path::Path;

use image::{ImageReader, Rgb, RgbImage};

use crate::grid::Cell;
use crate::puzzle::QueensPuzzle;

// --- 策略一（彩色 mask + 投影）参数 ---

/// 被视为"棋盘彩色像素"所需的最小色度（max - min）。
/// 白线、米色背景、UI 文字都接近灰，棋盘格子是彩色。
const MIN_CHROMA: u8 = 18;

/// 亮度保护，避免把深色文字或接近纯白的高光当成棋盘颜色。
const MIN_VALUE: u8 = 25;
const MAX_VALUE: u8 = 250;

/// 行投影中，纵向候选区域所需的彩色像素比例。
const BOARD_RUN_THRESHOLD: f32 = 0.18;

/// 单个格子所需的彩色像素比例。
const GRID_RUN_THRESHOLD: f32 = 0.55;

/// 找大块候选区域时允许合并的缺口（按图像高度比例自适应）。
const BOARD_MERGE_GAP_RATIO: f32 = 0.012;

/// 棋盘相对正方形允许的比例误差。
const MAX_SQUARE_ERROR: f32 = 0.12;

/// 支持的棋盘边长范围。
const MIN_BOARD_SIZE: usize = 4;
const MAX_BOARD_SIZE: usize = 16;

/// 格子尺寸的变异系数上限（标准差 / 均值）。
const MAX_CELL_SIZE_CV: f32 = 0.10;

/// 格间距的变异系数上限。
const MAX_CELL_SPACING_CV: f32 = 0.18;

/// 格子内部彩色像素占比下限，用于排除"看着像格子其实是空的"误判。
const MIN_CELL_OCCUPANCY: f32 = 0.82;

// --- 采样与聚类参数 ---

/// 采样格子中心多大比例的区域。
const SAMPLE_RATIO: f32 = 0.55;

/// 采样时忽略色度低于此值的像素（残留网格边、抗锯齿）。
const SAMPLE_MIN_CHROMA: u8 = 8;

/// HSV 空间的颜色聚类阈值。
///
/// 注意：这个值很关键。棋盘常用"同色相、不同明度"的配色（如深粉/浅粉、
/// 深绿/浅绿），阈值过大会把它们合并成一类，导致区域数少于 n。
/// 实测 0.16 会误合并，0.10 及以下稳定正确，这里取 0.08 留余量。
const COLOR_DISTANCE_THRESHOLD: f32 = 0.08;

/// Read a Queens puzzle from an image file (PNG / JPEG / WebP) and return the corresponding puzzle.
///
/// Two complementary strategies are tried:
/// 1. Color mask + row/column projection — robust for full-screen screenshots with
///    UI around the board, because UI elements are largely near-gray while board
///    cells are strongly colored.
/// 2. Gradient-profile fallback — used when the board is tightly cropped or cells
///    are too pale for strategy 1 to separate from the background.
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

/// Parse a puzzle from in-memory image bytes (PNG / JPEG / WebP).
///
/// Lets a caller pass image data straight from memory — e.g. Python writing to
/// this binary's stdin — without writing a temporary file.
pub fn try_from_bytes(data: &[u8]) -> Result<QueensPuzzle, String> {
    let img = image::load_from_memory(data)
        .map_err(|e| format!("failed to decode image: {e}"))?
        .to_rgb8();
    analyze(&img)
}

/// Analyze a Queens screenshot and return the corresponding puzzle.
///
/// See [`try_from_path`] for the two strategies. Both produce per-cell pixel
/// spans, which are then sampled and clustered by color to recover regions.
pub fn analyze(img: &RgbImage) -> Result<QueensPuzzle, String> {
    let (w, h) = img.dimensions();
    if w < 150 || h < 150 {
        return Err("image is too small".to_string());
    }

    if let Some(grid) = detect_by_projection(img) {
        if let Ok((puzzle, _, _)) = build_puzzle(img, &grid) {
            return Ok(puzzle);
        }
    }
    if let Some(grid) = detect_by_gradient(img) {
        if let Ok((puzzle, _, _)) = build_puzzle(img, &grid) {
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
// 策略一：彩色 mask + 行列投影
// ============================================================

fn detect_by_projection(img: &RgbImage) -> Option<Grid> {
    let width = img.width() as usize;
    let height = img.height() as usize;

    let mask = build_color_mask(img);
    let rows = row_projection(&mask, width, height);

    // 缺口按图像高度自适应，小图/大图都能正确合并被白线切开的区域。
    let merge_gap = ((height as f32) * BOARD_MERGE_GAP_RATIO).round().max(4.0) as usize;
    let row_candidates = find_runs(&rows, BOARD_RUN_THRESHOLD, merge_gap, merge_gap);

    let mut best: Option<(Grid, f32)> = None;

    for &(top, bottom) in &row_candidates {
        if bottom - top + 1 < (height / 8).max(120) {
            continue;
        }

        let cols = col_projection(&mask, width, top, bottom);
        let x_runs = find_runs(&cols, GRID_RUN_THRESHOLD, 4, 0);
        if !(MIN_BOARD_SIZE..=MAX_BOARD_SIZE).contains(&x_runs.len()) {
            continue;
        }

        let left = x_runs.first()?.0;
        let right = x_runs.last()?.1;

        let local_rows = local_row_projection(&mask, width, left, right, top, bottom);
        let local_y = find_runs(&local_rows, GRID_RUN_THRESHOLD, 4, 0);
        if !(MIN_BOARD_SIZE..=MAX_BOARD_SIZE).contains(&local_y.len()) {
            continue;
        }

        // 棋盘必须是 N × N
        if x_runs.len() != local_y.len() {
            continue;
        }
        let size = x_runs.len();

        let y_runs: Vec<(usize, usize)> = local_y
            .into_iter()
            .map(|(a, b)| (a + top, b + top))
            .collect();

        let bw = (right - left + 1) as f32;
        let bh = (y_runs.last()?.1 - y_runs.first()?.0 + 1) as f32;
        let square_error = (bw - bh).abs() / bw.max(bh);
        if square_error > MAX_SQUARE_ERROR {
            continue;
        }

        // 用变异系数（标准差/均值）而不是极差，对离群点更稳健。
        let widths: Vec<usize> = x_runs.iter().map(|&(a, b)| b - a + 1).collect();
        let heights: Vec<usize> = y_runs.iter().map(|&(a, b)| b - a + 1).collect();
        let x_gaps = gaps(&x_runs);
        let y_gaps = gaps(&y_runs);

        let width_cv = coefficient_of_variation(&widths);
        let height_cv = coefficient_of_variation(&heights);
        let x_gap_cv = coefficient_of_variation(&x_gaps);
        let y_gap_cv = coefficient_of_variation(&y_gaps);

        if width_cv > MAX_CELL_SIZE_CV || height_cv > MAX_CELL_SIZE_CV {
            continue;
        }
        if !x_gaps.is_empty() && x_gap_cv > MAX_CELL_SPACING_CV {
            continue;
        }
        if !y_gaps.is_empty() && y_gap_cv > MAX_CELL_SPACING_CV {
            continue;
        }

        // 每个格子内部必须真的填满了彩色，排除空心的误判。
        let occupancy = cell_occupancy_score(&mask, width, &x_runs, &y_runs);
        if occupancy < MIN_CELL_OCCUPANCY {
            continue;
        }

        // 多特征加权求和。用加法而不是连乘，避免单项偏低就整体归零。
        let size_score = (size as f32).ln_1p() / (MAX_BOARD_SIZE as f32).ln_1p();
        let square_score = 1.0 - square_error;
        let uniformity =
            (1.0 - width_cv * 3.0).clamp(0.0, 1.0) * (1.0 - height_cv * 3.0).clamp(0.0, 1.0);
        let spacing =
            (1.0 - x_gap_cv * 2.0).clamp(0.0, 1.0) * (1.0 - y_gap_cv * 2.0).clamp(0.0, 1.0);

        let score =
            occupancy * 4.0 + square_score * 3.0 + uniformity * 3.0 + spacing * 2.0 + size_score;

        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            best = Some((
                Grid {
                    size,
                    x_runs: x_runs.clone(),
                    y_runs,
                },
                score,
            ));
        }
    }

    best.map(|(g, _)| g)
}

fn build_color_mask(img: &RgbImage) -> Vec<u8> {
    let width = img.width() as usize;
    let height = img.height() as usize;
    let mut mask = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x as u32, y as u32);
            let max = p[0].max(p[1]).max(p[2]);
            let min = p[0].min(p[1]).min(p[2]);
            if max - min >= MIN_CHROMA && (MIN_VALUE..=MAX_VALUE).contains(&max) {
                mask[y * width + x] = 1;
            }
        }
    }
    mask
}

fn row_projection(mask: &[u8], width: usize, height: usize) -> Vec<f32> {
    (0..height)
        .map(|y| {
            let base = y * width;
            let count = (0..width).map(|x| mask[base + x] as usize).sum::<usize>();
            count as f32 / width as f32
        })
        .collect()
}

fn col_projection(mask: &[u8], width: usize, top: usize, bottom: usize) -> Vec<f32> {
    let h = (bottom - top + 1) as f32;
    (0..width)
        .map(|x| {
            let count = (top..=bottom)
                .map(|y| mask[y * width + x] as usize)
                .sum::<usize>();
            count as f32 / h
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
    let width = (right - left + 1) as f32;
    (top..=bottom)
        .map(|y| {
            let count = (left..=right)
                .map(|x| mask[y * image_width + x] as usize)
                .sum::<usize>();
            count as f32 / width
        })
        .collect()
}

fn find_runs(
    values: &[f32],
    threshold: f32,
    min_len: usize,
    merge_gap: usize,
) -> Vec<(usize, usize)> {
    let mut raw = Vec::<(usize, usize)>::new();
    let mut start = None;

    for (i, &value) in values.iter().enumerate() {
        if value >= threshold {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            let e = i - 1;
            if e - s + 1 >= min_len {
                raw.push((s, e));
            }
        }
    }
    if let Some(s) = start {
        let e = values.len() - 1;
        if e - s + 1 >= min_len {
            raw.push((s, e));
        }
    }

    if raw.is_empty() || merge_gap == 0 {
        return raw;
    }

    let mut out = Vec::<(usize, usize)>::with_capacity(raw.len());
    let mut current = raw[0];
    for next in raw.into_iter().skip(1) {
        if next.0.saturating_sub(current.1 + 1) <= merge_gap {
            current.1 = next.1;
        } else {
            out.push(current);
            current = next;
        }
    }
    out.push(current);
    out
}

fn gaps(runs: &[(usize, usize)]) -> Vec<usize> {
    if runs.len() < 2 {
        return Vec::new();
    }
    runs.windows(2)
        .map(|w| w[1].0.saturating_sub(w[0].1 + 1))
        .collect()
}

fn coefficient_of_variation(values: &[usize]) -> f32 {
    if values.is_empty() {
        return f32::INFINITY;
    }
    let mean = values.iter().sum::<usize>() as f32 / values.len() as f32;
    if mean <= 0.0 {
        return f32::INFINITY;
    }
    let variance = values
        .iter()
        .map(|&v| (v as f32 - mean).powi(2))
        .sum::<f32>()
        / values.len() as f32;
    variance.sqrt() / mean
}

/// 每个格子内部（去掉 15% 边距）彩色像素的占比。
fn cell_occupancy_score(
    mask: &[u8],
    image_width: usize,
    x_runs: &[(usize, usize)],
    y_runs: &[(usize, usize)],
) -> f32 {
    let mut total = 0usize;
    let mut colored = 0usize;

    for &(y0, y1) in y_runs {
        for &(x0, x1) in x_runs {
            let w = x1 - x0 + 1;
            let h = y1 - y0 + 1;
            let pad_x = ((w as f32) * 0.15).round() as usize;
            let pad_y = ((h as f32) * 0.15).round() as usize;
            let sx = x0 + pad_x.min(w / 3);
            let ex = x1.saturating_sub(pad_x.min(w / 3));
            let sy = y0 + pad_y.min(h / 3);
            let ey = y1.saturating_sub(pad_y.min(h / 3));

            for y in sy..=ey {
                for x in sx..=ex {
                    colored += mask[y * image_width + x] as usize;
                    total += 1;
                }
            }
        }
    }

    if total == 0 {
        0.0
    } else {
        colored as f32 / total as f32
    }
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

    let cell = h_cell.min(v_cell);
    if (h_cell as i32 - v_cell as i32).abs() > cell as i32 / 4 {
        return None;
    }

    let n_h = ((h_end - h_start) as f64 / cell as f64).round() as usize;
    let n_w = ((v_end - v_start) as f64 / cell as f64).round() as usize;
    if n_h != n_w || !(MIN_BOARD_SIZE..=MAX_BOARD_SIZE).contains(&n_h) {
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

/// 用梯度剖面的显著峰定位棋盘边界与格子大小：首尾峰是棋盘与外边距的过渡，
/// 中间峰是内部网格线，其间距中位数即格子大小。
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

#[derive(Clone, Copy)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
}

#[derive(Clone, Copy)]
struct Sample {
    rgb: Rgb<u8>,
    hsv: Hsv,
}

/// Pixel geometry of a located board, for callers that need screen coordinates
/// (e.g. driving clicks) rather than just row/column indices.
pub struct BoardGeometry {
    /// Board size (n for an n x n board).
    pub size: usize,
    /// Board bounding box in image coordinates.
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    /// Center pixel of each cell, indexed `[row][col]`.
    pub cell_centers: Vec<Vec<(u32, u32)>>,
    /// Average color of each cell, indexed `[row][col]`.
    pub cell_colors: Vec<Vec<Rgb<u8>>>,
    /// Region id of each cell, indexed `[row][col]`.
    pub regions: Vec<Vec<usize>>,
}

/// A rectangular region of interest, in original-image coordinates.
///
/// Used when the caller already knows roughly where the board is (e.g. a fixed
/// app layout). The region does **not** have to be exactly the board — the
/// detector still runs inside it, so an approximate box is fine.
#[derive(Debug, Clone, Copy)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// Like [`analyze_detailed`], but restricted to `crop` when given.
///
/// Coordinates in the returned geometry are always in **original-image** space,
/// so cell centers can be used for clicks without any manual offset arithmetic.
pub fn analyze_cropped(
    img: &RgbImage,
    crop: Option<Crop>,
) -> Result<(QueensPuzzle, BoardGeometry), String> {
    let crop = match crop {
        None => return analyze_detailed(img),
        Some(c) => c,
    };

    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("image is empty".to_string());
    }

    // Clamp to the image so a slightly out-of-range box still works.
    let left = crop.left.min(w - 1);
    let right = crop.right.min(w - 1);
    let top = crop.top.min(h - 1);
    let bottom = crop.bottom.min(h - 1);

    if right <= left || bottom <= top {
        return Err(format!(
            "crop region is empty or invalid: ({left}, {top})..({right}, {bottom})"
        ));
    }

    let sub = crop_image(img, left, top, right, bottom);

    // The crop is only a hint. If nothing is found inside it, fall back to the
    // whole image rather than failing — a stale or slightly-off hint should not
    // break the call.
    let (puzzle, mut geo) = match analyze_detailed(&sub) {
        Ok(found) => found,
        Err(_) => return analyze_detailed(img),
    };

    // Translate geometry back into original-image coordinates.
    geo.left += left;
    geo.right += left;
    geo.top += top;
    geo.bottom += top;
    for row in geo.cell_centers.iter_mut() {
        for (x, y) in row.iter_mut() {
            *x += left;
            *y += top;
        }
    }
    Ok((puzzle, geo))
}

fn crop_image(img: &RgbImage, left: u32, top: u32, right: u32, bottom: u32) -> RgbImage {
    let width = right - left + 1;
    let height = bottom - top + 1;
    let mut out = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            out.put_pixel(x, y, *img.get_pixel(left + x, top + y));
        }
    }
    out
}

/// Like [`analyze`], but also returns the pixel geometry of the detected board.
pub fn analyze_detailed(img: &RgbImage) -> Result<(QueensPuzzle, BoardGeometry), String> {
    let (w, h) = img.dimensions();
    if w < 150 || h < 150 {
        return Err("image is too small".to_string());
    }

    let mut last_err: Option<String> = None;
    for strategy in [detect_by_projection, detect_by_gradient] {
        if let Some(grid) = strategy(img) {
            match build_puzzle(img, &grid) {
                Ok((puzzle, regions, colors)) => {
                    let cell_centers = (0..grid.size)
                        .map(|row| {
                            (0..grid.size)
                                .map(|col| {
                                    let (x0, x1) = grid.x_runs[col];
                                    let (y0, y1) = grid.y_runs[row];
                                    (((x0 + x1) / 2) as u32, ((y0 + y1) / 2) as u32)
                                })
                                .collect()
                        })
                        .collect();
                    let left = grid.x_runs.first().map(|&(a, _)| a as u32).unwrap_or(0);
                    let right = grid.x_runs.last().map(|&(_, b)| b as u32).unwrap_or(0);
                    let top = grid.y_runs.first().map(|&(a, _)| a as u32).unwrap_or(0);
                    let bottom = grid.y_runs.last().map(|&(_, b)| b as u32).unwrap_or(0);

                    return Ok((
                        puzzle,
                        BoardGeometry {
                            size: grid.size,
                            left,
                            top,
                            right,
                            bottom,
                            cell_centers,
                            cell_colors: colors,
                            regions,
                        },
                    ));
                }
                Err(e) => last_err = Some(e),
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "could not locate a Queens board in the image".to_string()))
}

/// Result of building a puzzle from a located grid: the puzzle, the region id
/// matrix, and the per-cell sampled colors.
type BuiltPuzzle = (QueensPuzzle, Vec<Vec<usize>>, Vec<Vec<Rgb<u8>>>);

/// Sample every cell and cluster by color; returns the region matrix and per-cell colors.
fn build_puzzle(img: &RgbImage, grid: &Grid) -> Result<BuiltPuzzle, String> {
    let n = grid.size;
    let mut samples: Vec<Vec<Sample>> = Vec::with_capacity(n);
    for row in 0..n {
        let mut row_samples = Vec::with_capacity(n);
        for col in 0..n {
            row_samples.push(sample_cell(
                img,
                grid.x_runs[col].0 as u32,
                grid.y_runs[row].0 as u32,
                grid.x_runs[col].1 as u32,
                grid.y_runs[row].1 as u32,
            ));
        }
        samples.push(row_samples);
    }

    let flat: Vec<Sample> = samples.iter().flatten().copied().collect();
    let ids = cluster_colors(&flat, COLOR_DISTANCE_THRESHOLD);

    let mut region_cells: Vec<Vec<Cell>> = Vec::new();
    let mut color_to_region: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    let mut regions = vec![vec![0usize; n]; n];

    for (i, &cid) in ids.iter().enumerate() {
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
        regions[row][col] = region;
        region_cells[region].push(Cell { row, col });
    }

    if region_cells.len() != n {
        return Err(format!(
            "expected {n} distinct regions, found {}",
            region_cells.len()
        ));
    }

    let colors: Vec<Vec<Rgb<u8>>> = samples
        .iter()
        .map(|r| r.iter().map(|s| s.rgb).collect())
        .collect();
    Ok((QueensPuzzle::new(region_cells), regions, colors))
}

/// 采样格子中心区域，忽略低色度像素（残留网格边、抗锯齿）。
fn sample_cell(img: &RgbImage, left: u32, top: u32, right: u32, bottom: u32) -> Sample {
    let w = (right - left + 1) as f32;
    let h = (bottom - top + 1) as f32;
    let cx = (left + right) as f32 / 2.0;
    let cy = (top + bottom) as f32 / 2.0;
    let half_w = w * SAMPLE_RATIO / 2.0;
    let half_h = h * SAMPLE_RATIO / 2.0;

    let x0 = (cx - half_w).max(left as f32) as u32;
    let x1 = (cx + half_w).min(right as f32) as u32;
    let y0 = (cy - half_h).max(top as f32) as u32;
    let y1 = (cy + half_h).min(bottom as f32) as u32;

    let mut sr = 0u64;
    let mut sg = 0u64;
    let mut sb = 0u64;
    let mut count = 0u64;

    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = img.get_pixel(x, y);
            let max = p[0].max(p[1]).max(p[2]);
            let min = p[0].min(p[1]).min(p[2]);
            if max - min >= SAMPLE_MIN_CHROMA {
                sr += p[0] as u64;
                sg += p[1] as u64;
                sb += p[2] as u64;
                count += 1;
            }
        }
    }

    let rgb = match (
        sr.checked_div(count),
        sg.checked_div(count),
        sb.checked_div(count),
    ) {
        (Some(r), Some(g), Some(b)) => Rgb([r as u8, g as u8, b as u8]),
        // count == 0: 中心区域没有彩色像素，退回单点采样。
        _ => {
            let p = img.get_pixel((left + right) / 2, (top + bottom) / 2);
            Rgb([p[0], p[1], p[2]])
        }
    };

    Sample {
        rgb,
        hsv: rgb_to_hsv(rgb),
    }
}

fn rgb_to_hsv(rgb: Rgb<u8>) -> Hsv {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;

    let h = if d < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        let mut h = 60.0 * ((g - b) / d);
        if h < 0.0 {
            h += 360.0;
        }
        h
    } else if (max - g).abs() < 1e-6 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };

    let s = if max <= 1e-6 { 0.0 } else { d / max };
    Hsv { h, s, v: max }
}

/// 色相为主的距离；色相是环形量，取最短弧。
fn hsv_distance(a: Hsv, b: Hsv) -> f32 {
    let mut dh = (a.h - b.h).abs();
    if dh > 180.0 {
        dh = 360.0 - dh;
    }
    let dh = dh / 180.0;
    let ds = a.s - b.s;
    let dv = a.v - b.v;
    (dh * dh * 0.68 + ds * ds * 0.20 + dv * dv * 0.12).sqrt()
}

/// 增量式聚类；色相用向量平均避免 0°/360° 环绕问题。
fn cluster_colors(samples: &[Sample], threshold: f32) -> Vec<usize> {
    let mut centers: Vec<Sample> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut ids = Vec::with_capacity(samples.len());

    for &sample in samples {
        let mut best_id = None;
        let mut best_dist = f32::INFINITY;
        for (i, &center) in centers.iter().enumerate() {
            let d = hsv_distance(sample.hsv, center.hsv);
            if d < best_dist {
                best_dist = d;
                best_id = Some(i);
            }
        }

        let id = match best_id {
            Some(i) if best_dist <= threshold => i,
            _ => {
                let i = centers.len();
                centers.push(sample);
                counts.push(0);
                i
            }
        };
        counts[id] += 1;
        ids.push(id);

        let n = counts[id] as f32;
        let old = centers[id];

        let old_h = old.hsv.h.to_radians();
        let new_h = sample.hsv.h.to_radians();
        let sum_x = old_h.cos() * (n - 1.0) + new_h.cos();
        let sum_y = old_h.sin() * (n - 1.0) + new_h.sin();
        let avg_h = sum_y.atan2(sum_x).to_degrees().rem_euclid(360.0);

        let rgb = Rgb([
            (((old.rgb[0] as f32) * (n - 1.0) + sample.rgb[0] as f32) / n).round() as u8,
            (((old.rgb[1] as f32) * (n - 1.0) + sample.rgb[1] as f32) / n).round() as u8,
            (((old.rgb[2] as f32) * (n - 1.0) + sample.rgb[2] as f32) / n).round() as u8,
        ]);

        centers[id] = Sample {
            rgb,
            hsv: Hsv {
                h: avg_h,
                s: ((old.hsv.s * (n - 1.0) + sample.hsv.s) / n).clamp(0.0, 1.0),
                v: ((old.hsv.v * (n - 1.0) + sample.hsv.v) / n).clamp(0.0, 1.0),
            },
        };
    }

    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_cropped_screenshot_with_white_grid() {
        let path = std::path::PathBuf::from("../puzzles/screenshot-1.png");
        let puzzle = try_from_path(&path).expect("should parse cropped screenshot");
        assert_eq!(puzzle.n(), 8);
    }

    #[test]
    fn analyze_full_screen_screenshot() {
        let path = std::path::PathBuf::from("../puzzles/screenshot-full.png");
        let puzzle = try_from_path(&path).expect("should parse full-screen screenshot");
        assert_eq!(puzzle.n(), 8);
    }

    #[test]
    fn analyze_full_screen_screenshot_10x10() {
        let path = std::path::PathBuf::from("../puzzles/screenshot-10x10.png");
        let puzzle = try_from_path(&path).expect("should parse 10x10 full-screen screenshot");
        assert_eq!(puzzle.n(), 10);
    }
}
