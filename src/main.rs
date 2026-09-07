use clap::{Parser, Subcommand};
use colored::*;
use queens_puzzle_core::grid::Cell;
use queens_puzzle_core::puzzle::{region_color, QueensPuzzle, State};
use queens_puzzle_core::{io, solver};
use serde_json::json;
use std::fmt::Write as _;
use std::path::PathBuf;

/// Solver for the LinkedIn Queens puzzle.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Solve a puzzle from a file and rate its difficulty
    Solve {
        /// Path to the puzzle file
        file: PathBuf,

        /// Parse the file as archived JSON (see README) instead of the text grid format
        #[arg(long)]
        json: bool,

        /// When reading JSON, the id of the puzzle to solve (defaults to the lowest id in the file)
        #[arg(long)]
        id: Option<u32>,
    },

    /// Analyze a screenshot and emit JSON — for programmatic use (e.g. from Python)
    ///
    /// Reads image bytes from stdin when no path is given (or when the path is "-"),
    /// so callers can pass an in-memory image without writing a temp file. On
    /// success writes JSON to stdout and exits 0; on failure writes the reason to
    /// stderr and exits 1.
    Analyze {
        /// Path to the image. Omit (or pass "-") to read image bytes from stdin.
        file: Option<PathBuf>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Solve { file, json, id } => {
            let puzzle = if json {
                io::archived_queens::read(file, id)?
            } else if is_image(&file) {
                io::image::try_from_path(&file)?
            } else {
                io::text::read_puzzle_text(file)
            };
            solve_puzzle(&puzzle);
        }
        Command::Analyze { file } => {
            let bytes = match file.as_deref() {
                Some(p) if p.as_os_str() != "-" => {
                    std::fs::read(p).map_err(|e| format!("failed to read {}: {e}", p.display()))?
                }
                _ => {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
                    buf
                }
            };
            let img = image::load_from_memory(&bytes)?.to_rgb8();
            let (puzzle, geo) = io::image::analyze_detailed(&img)?;
            println!("{}", analyze_to_json(&puzzle, &geo));
        }
    }

    Ok(())
}

fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("png")
            || ext.eq_ignore_ascii_case("jpg")
            || ext.eq_ignore_ascii_case("jpeg")
            || ext.eq_ignore_ascii_case("webp")
    )
}

fn solve_puzzle(puzzle: &QueensPuzzle) {
    println!("{}", format_board(puzzle));

    let difficulty = solver::rate_puzzle(&mut puzzle.clone());
    let solved = solve_queens(puzzle);

    // 把解叠加到盘面上再打印（rate_puzzle 不写回传入的 puzzle）。
    let mut display = puzzle.clone();
    for cell in &solved {
        display.set_cell_state(*cell, State::Queen);
    }
    println!("{}", format_board(&display));
    match difficulty {
        Some(d) => println!("Difficulty: {}", d),
        None => println!("Difficulty: unrated (no unique solution)"),
    }
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

/// Build the JSON payload describing the detected board and its solution.
fn analyze_to_json(puzzle: &QueensPuzzle, geo: &io::image::BoardGeometry) -> String {
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
    let palette: Vec<serde_json::Value> = (0..n)
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

    // 注意：solver::rate_puzzle 只在内部 clone 上求解，不会写回传入的 puzzle，
    // 所以这里必须单独解一次才能拿到皇后位置。
    let difficulty = solver::rate_puzzle(&mut puzzle.clone());
    let solution: Vec<serde_json::Value> = solve_queens(puzzle)
        .iter()
        .map(|cell| json!([cell.row, cell.col]))
        .collect();

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
        "solution": solution,
        "difficulty": difficulty.map(|d| d.to_string()),
        "palette": palette,
    })
    .to_string()
}
