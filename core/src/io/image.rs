use std::collections::HashMap;
use std::path::Path;

use image::{ImageReader, Rgb, RgbImage};

use crate::grid::Cell;
use crate::puzzle::QueensPuzzle;

/// Read a Queens puzzle from an image file (PNG / JPEG / WebP) and return the corresponding puzzle.
///
/// The screenshot must show a clean grid of colored cells separated by grid lines.
/// Works for any grid line color (white, black, or otherwise) because line detection
/// is based on color gradients rather than a fixed color threshold.
///
/// # Arguments
/// * `path` - Path to the image file
///
/// # Returns
/// A `QueensPuzzle` whose regions are the colored cells detected in the screenshot,
/// or an error message describing why the image could not be parsed.
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

/// Analyze a Queens puzzle screenshot and return the corresponding puzzle.
///
/// Algorithm:
/// 1. Compute a 1D gradient profile for rows and columns. The profile measures
///    color change between adjacent rows/columns; peaks correspond to grid lines
///    and the transitions between the board and its margin.
/// 2. Find the significant peaks in each profile. The first and last peak mark
///    the board boundaries; the peaks in between are the internal grid lines.
/// 3. Use the median spacing between internal peaks to determine the cell size,
///    and derive n from the board span and cell size.
/// 4. Crop to the board, sample the center of each cell, and cluster by color.
pub fn analyze(img: &RgbImage) -> Result<QueensPuzzle, String> {
    let (w, h) = img.dimensions();
    let h_profile = gradient_rows(img);
    let v_profile = gradient_cols(img);

    let (h_start, h_end, h_cell) = find_board_and_cell_size(&h_profile, h)
        .ok_or_else(|| "could not determine horizontal grid".to_string())?;
    let (v_start, v_end, v_cell) = find_board_and_cell_size(&v_profile, w)
        .ok_or_else(|| "could not determine vertical grid".to_string())?;

    // Cells should be square.
    let cell_size = h_cell.min(v_cell);
    let cell_diff = h_cell.max(v_cell).saturating_sub(cell_size);
    if cell_diff > cell_size / 4 {
        return Err(format!("cells are not square: {h_cell} x {v_cell}"));
    }

    let board_w = v_end - v_start;
    let board_h = h_end - h_start;
    let n_w = (board_w as f64 / cell_size as f64).round() as usize;
    let n_h = (board_h as f64 / cell_size as f64).round() as usize;
    if n_w != n_h {
        return Err(format!("grid is not square: {n_h} rows x {n_w} cols"));
    }
    let n = n_w;

    if !(4..=16).contains(&n) {
        return Err(format!("grid size {n} is out of supported range [4, 16]"));
    }

    // Crop to the board and sample each cell center.
    let cropped = crop(img, (v_start, h_start, v_end, h_end));
    let cell = (
        cropped.width() as f64 / n as f64,
        cropped.height() as f64 / n as f64,
    );
    let mut cells = vec![[0u8; 3]; n * n];
    for row in 0..n {
        for col in 0..n {
            let cy = ((row as f64 + 0.5) * cell.1) as u32;
            let cx = ((col as f64 + 0.5) * cell.0) as u32;
            let p = cropped.get_pixel(cx, cy);
            cells[row * n + col] = [p[0], p[1], p[2]];
        }
    }

    // Cluster cells by color and build the puzzle.
    let mut region_grid = vec![0u8; n * n];
    let mut color_to_id: HashMap<[u8; 3], u8> = HashMap::new();
    let mut next_id: u8 = 0;
    for (i, &color) in cells.iter().enumerate() {
        let key = quantize(color);
        let id = *color_to_id.entry(key).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        region_grid[i] = id;
    }

    if next_id as usize != n {
        return Err(format!("expected {n} distinct regions, found {next_id}"));
    }

    let mut region_cells: Vec<Vec<Cell>> = vec![vec![]; n];
    for (i, &region_id) in region_grid.iter().enumerate() {
        let row = i / n;
        let col = i % n;
        region_cells[region_id as usize].push(Cell { row, col });
    }

    Ok(QueensPuzzle::new(region_cells))
}

fn color_dist_sq(a: &Rgb<u8>, b: &Rgb<u8>) -> u64 {
    let dr = a[0] as i64 - b[0] as i64;
    let dg = a[1] as i64 - b[1] as i64;
    let db = a[2] as i64 - b[2] as i64;
    (dr * dr + dg * dg + db * db) as u64
}

/// Sum of squared color distances between each pixel in row y and the pixel directly below it.
/// Peaks mark rows where the color changes sharply — i.e. grid lines and board edges.
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

/// Same idea as [`gradient_rows`] but for columns.
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

/// Locate the board's start and end positions and the cell size in one direction.
///
/// The first and last significant peaks mark the transitions between the board and
/// its margin. The internal peaks are the grid lines; the median spacing between
/// consecutive internal peaks is the cell size.
fn find_board_and_cell_size(profile: &[u64], dim: u32) -> Option<(u32, u32, u32)> {
    if profile.is_empty() || dim == 0 {
        return None;
    }

    let mut sorted: Vec<u64> = profile.to_vec();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    // Threshold well above noise so we only catch real lines.
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
    let spacings: Vec<u32> = internal.windows(2).map(|w| w[1] - w[0]).collect();
    if spacings.is_empty() {
        return None;
    }
    let mut sorted = spacings;
    sorted.sort_unstable();
    let cell_size = sorted[sorted.len() / 2];
    if cell_size == 0 {
        return None;
    }

    Some((start, end, cell_size))
}

fn crop(img: &RgbImage, bbox: (u32, u32, u32, u32)) -> RgbImage {
    let (min_x, min_y, max_x, max_y) = bbox;
    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    let mut out = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            out.put_pixel(x, y, *img.get_pixel(min_x + x, min_y + y));
        }
    }
    out
}

/// Snap each channel to the nearest multiple of 8 to absorb minor color noise.
fn quantize(c: [u8; 3]) -> [u8; 3] {
    [c[0] / 8 * 8, c[1] / 8 * 8, c[2] / 8 * 8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_screenshot_with_white_grid() {
        // Real screenshot provided by the user: 8x8 board with thin white grid lines
        // on a beige background. Gradient-based detection locates the board and grid
        // lines purely from color transitions, so it works for any line or margin color.
        let path = std::path::PathBuf::from("../puzzles/screenshot-1.png");
        let puzzle = try_from_path(&path).expect("should parse white-grid screenshot");
        assert_eq!(puzzle.n(), 8);
    }
}
