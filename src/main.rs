use clap::{Parser, Subcommand};
use colored::*;
use queens_puzzle_core::grid::Cell;
use queens_puzzle_core::puzzle::{region_color, QueensPuzzle, State};
use queens_puzzle_core::{io, solver};
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

    let mut working = puzzle.clone();
    let difficulty = solver::rate_puzzle(&mut working);

    println!("{}", format_board(&working));
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
