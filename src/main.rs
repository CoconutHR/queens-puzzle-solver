use clap::{Parser, Subcommand, ValueEnum};
use colored::*;
use queens_puzzle_core::grid::Cell;
use queens_puzzle_core::puzzle::{region_color, QueensPuzzle, State};
use queens_puzzle_core::{io, solver};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::PathBuf;

/// Solver for the LinkedIn Queens puzzle.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// Colored board + difficulty — for humans at a terminal
    Human,
    /// Structured JSON envelope — for scripts and other programs
    Json,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Solve a puzzle from a file and rate its difficulty
    Solve {
        /// Path to the puzzle file
        file: PathBuf,

        /// Parse the file as archived JSON (see README) instead of the text grid format
        #[arg(long)]
        archived: bool,

        /// When reading archived JSON, the id of the puzzle to solve
        /// (defaults to the lowest id in the file)
        #[arg(long)]
        id: Option<u32>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },

    /// Analyze a screenshot and emit JSON — for programmatic use (e.g. from Python)
    ///
    /// Reads image bytes from stdin when no path is given (or when the path is "-"),
    /// so callers can pass an in-memory image without writing a temp file.
    ///
    /// On success writes a JSON envelope `{ ok: true, data: ... }` to stdout and
    /// exits 0. On failure writes `{ ok: false, error: { kind, message } }` to
    /// stdout and exits 1, so callers can branch on `error.kind` without parsing
    /// free-form text.
    Analyze {
        /// Path to the image. Omit (or pass "-") to read image bytes from stdin.
        file: Option<PathBuf>,

        /// Only look inside this box, as "left,top,right,bottom" in image pixels.
        ///
        /// Useful when the app layout is fixed: it narrows the search and makes
        /// detection deterministic. The box does NOT need to match the board
        /// exactly — detection still runs inside it. Out-of-range values are
        /// clamped to the image. Returned coordinates stay in original-image
        /// space, so they can be used for clicks directly.
        #[arg(long, value_name = "L,T,R,B")]
        crop: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
}

fn main() {
    // 让 stdout 被提前关闭时像其它 Unix 程序一样安静退出，而不是 panic
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    let (format, result) = match cli.command {
        Command::Solve {
            file,
            archived,
            id,
            format,
        } => (format, run_solve(file, archived, id, format)),
        Command::Analyze { file, crop, format } => (format, run_analyze(file, crop, format)),
    };

    match result {
        Ok(Some(value)) => {
            // JSON 模式：把成功的 data 包进信封
            println!(
                "{}",
                json!({
                    "ok": true,
                    "data": value,
                })
            );
        }
        Ok(None) => {
            // Human 模式：run_* 已经直接打印过了，什么都不用做
        }
        Err(err) => {
            match format {
                OutputFormat::Json => {
                    // 机器可读：结构化错误信封打到 stdout，退出码仍为非零
                    println!(
                        "{}",
                        json!({
                            "ok": false,
                            "error": {
                                "kind": err.kind,
                                "message": err.message,
                            }
                        })
                    );
                }
                OutputFormat::Human => {
                    eprintln!("Error: {}", err.message);
                }
            }
            std::process::exit(1);
        }
    }
}

/// 应用层错误：带一个稳定的 kind，供 Python 侧 switch。
struct AppError {
    kind: &'static str,
    message: String,
}

impl AppError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// solve
// ---------------------------------------------------------------------------

fn run_solve(
    file: PathBuf,
    archived: bool,
    id: Option<u32>,
    format: OutputFormat,
) -> Result<Option<Value>, AppError> {
    let puzzle = if archived {
        io::archived_queens::read(file, id).map_err(|e| {
            AppError::new("parse_error", format!("failed to read archived puzzle: {e}"))
        })?
    } else if is_image(&file) {
        io::image::try_from_path(&file).map_err(|e| {
            AppError::new("detect_failed", format!("failed to analyze image: {e}"))
        })?
    } else {
        io::text::read_puzzle_text(file).map_err(|e| {
            AppError::new("parse_error", format!("failed to read puzzle text: {e}"))
        })?
    };

    // rate_puzzle 只在内部 clone 上求解，不写回传入的 puzzle
    let difficulty = solver::rate_puzzle(&mut puzzle.clone());
    let solved = solve_queens(&puzzle);

    match format {
        OutputFormat::Human => {
            // 原始盘面
            println!("{}", format_board(&puzzle));

            // 叠加解后再打一次
            let mut display = puzzle.clone();
            for cell in &solved {
                display.set_cell_state(*cell, State::Queen);
            }
            println!("{}", format_board(&display));

            match &difficulty {
                Some(d) => println!("Difficulty: {d}"),
                None => println!("Difficulty: unrated (no unique solution)"),
            }
            Ok(None)
        }
        OutputFormat::Json => Ok(Some(json!({
            "size": puzzle.n(),
            "regions": regions_matrix(&puzzle),
            "solution": cells_to_json(&solved),
            "difficulty": difficulty.as_ref().map(|d| d.to_string()),
        }))),
    }
}

// ---------------------------------------------------------------------------
// analyze
// ---------------------------------------------------------------------------

fn run_analyze(
    file: Option<PathBuf>,
    crop: Option<String>,
    format: OutputFormat,
) -> Result<Option<Value>, AppError> {
    let bytes = match file.as_deref() {
        Some(p) if p.as_os_str() != "-" => std::fs::read(p)
            .map_err(|e| AppError::new("io_error", format!("failed to read {}: {e}", p.display())))?,
        _ => {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
                .map_err(|e| AppError::new("io_error", format!("failed to read stdin: {e}")))?;
            buf
        }
    };

    let img = image::load_from_memory(&bytes)
        .map_err(|e| AppError::new("decode_error", format!("failed to decode image: {e}")))?
        .to_rgb8();

    let crop = crop.as_deref().map(parse_crop).transpose()?;

    let (puzzle, geo) = io::image::analyze_cropped(&img, crop)
        .map_err(|e| AppError::new("detect_failed", format!("failed to detect board: {e}")))?;

    match format {
        OutputFormat::Human => {
            println!("{}", format_board(&puzzle));
            let solved = solve_queens(&puzzle);
            let mut display = puzzle.clone();
            for cell in &solved {
                display.set_cell_state(*cell, State::Queen);
            }
            println!("{}", format_board(&display));
            Ok(None)
        }
        OutputFormat::Json => Ok(Some(analyze_to_json(&puzzle, &geo))),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("png")
            || ext.eq_ignore_ascii_case("jpg")
            || ext.eq_ignore_ascii_case("jpeg")
            || ext.eq_ignore_ascii_case("webp")
    )
}

fn format_board(puzzle: &QueensPuzzle) -> String {
    let n = puzzle.n();
    let mut out = String::new();

    for row in 0..n {
        for col in 0..n {
            let cell = Cell { row, col };
            let cell_text = match puzzle[cell] {
                State::Queen => " ♛ ",
                State::Empty => " x ",
                _ => "   ",
            }
            .white();
            let region_index = puzzle
                .all_regions_iter()
                .position(|(region, _)| region.contains(&cell));

            let cell_text = match region_index {
                Some(index) => colorize_region(cell_text, index),
                None => cell_text,
            };
            write!(out, "{} ", cell_text).unwrap();
        }
        writeln!(out).unwrap();
    }

    out
}

fn colorize_region(cell: ColoredString, region_index: usize) -> ColoredString {
    let color = region_color(region_index);
    let color = Color::TrueColor {
        r: color.r,
        g: color.g,
        b: color.b,
    };
    cell.on_color(color)
}

/// 求解并返回皇后位置：先试逻辑推导，不行再退回暴力搜索。
fn solve_queens(puzzle: &QueensPuzzle) -> Vec<Cell> {
    let mut solved = puzzle.clone();
    if solver::solve_and_rate_puzzle(&mut solved).is_some() {
        return solved.queens();
    }

    let mut solutions = Vec::new();
    solver::brute_force::solve(&mut solved, &mut solutions);
    if solutions.len() == 1 {
        return solutions.remove(0).queens();
    }
    Vec::new()
}

/// 把 puzzle 转成 n×n 的区域编号矩阵，索引即区域 id。
fn regions_matrix(puzzle: &QueensPuzzle) -> Vec<Vec<usize>> {
    let n = puzzle.n();
    let regions: Vec<_> = puzzle.all_regions_iter().collect();
    (0..n)
        .map(|row| {
            (0..n)
                .map(|col| {
                    let cell = Cell { row, col };
                    regions
                        .iter()
                        .position(|(r, _)| r.contains(&cell))
                        .unwrap_or(0)
                })
                .collect()
        })
        .collect()
}

fn cells_to_json(cells: &[Cell]) -> Vec<Value> {
    cells.iter().map(|c| json!([c.row, c.col])).collect()
}

/// Parse "left,top,right,bottom" into a Crop box.
fn parse_crop(spec: &str) -> Result<io::image::Crop, AppError> {
    let parts: Vec<&str> = spec.split(',').map(|p| p.trim()).collect();
    if parts.len() != 4 {
        return Err(AppError::new(
            "invalid_argument",
            format!(
                "--crop expects 4 comma-separated numbers (left,top,right,bottom), got {spec:?}"
            ),
        ));
    }
    let nums: Result<Vec<u32>, _> = parts.iter().map(|p| p.parse::<u32>()).collect();
    let nums = nums.map_err(|_| {
        AppError::new("invalid_argument", format!("--crop contains a non-number: {spec:?}"))
    })?;
    Ok(io::image::Crop {
        left: nums[0],
        top: nums[1],
        right: nums[2],
        bottom: nums[3],
    })
}

/// 构造 analyze 的 data 部分（外层信封由 main 负责）。
fn analyze_to_json(puzzle: &QueensPuzzle, geo: &io::image::BoardGeometry) -> Value {
    let n = geo.size;

    let mut cells = Vec::with_capacity(n);
    for row in 0..n {
        let mut row_cells = Vec::with_capacity(n);
        for col in 0..n {
            let (x, y) = geo.cell_centers[row][col];
            let c = geo.cell_colors[row][col];
            row_cells.push(json!({
                "row": row,
                "col": col,
                "region": geo.regions[row][col],
                "x": x,
                "y": y,
                "rgb": [c[0], c[1], c[2]],
            }));
        }
        cells.push(row_cells);
    }

    // Region id -> representative color.
    let palette: Vec<Value> = (0..n)
        .map(|region| {
            let mut representative = None;
            'search: for row in 0..n {
                for col in 0..n {
                    if geo.regions[row][col] == region {
                        representative = Some(geo.cell_colors[row][col]);
                        break 'search;
                    }
                }
            }
            let c = representative.unwrap_or(image::Rgb([0u8, 0, 0]));
            json!([c[0], c[1], c[2]])
        })
        .collect();

    // rate_puzzle 只在内部 clone 上求解，不写回传入的 puzzle
    let difficulty = solver::rate_puzzle(&mut puzzle.clone());
    let solution = cells_to_json(&solve_queens(puzzle));

    json!({
        "size": n,
        "board": {
            "left": geo.left,
            "top": geo.top,
            "right": geo.right,
            "bottom": geo.bottom,
        },
        "regions": geo.regions,
        "cells": cells,
        "palette": palette,
        "solution": solution,
        "difficulty": difficulty.as_ref().map(|d| d.to_string()),
    })
}