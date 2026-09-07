# CLAUDE.md — Queens Puzzle

## Repository layout

```
queens-puzzle/
├── src/       CLI binary (depends on core; coloured output, clap, env_logger)
├── core/      Library crate — solver, generator, puzzle types, IO
│   └── src/
│       ├── io/json.rs    Canonical JSON format
│       └── io/text.rs    Text/archive formats (used by the CLI)
└── docs/      formats.md (puzzle format spec)
```

The root `Cargo.toml` is simultaneously the workspace root and the CLI binary package.

## Git

The `main` branch is protected — never push directly to it. All changes go through branches and PRs.

## Build commands

```bash
# Build CLI
cargo build --release

# Run Rust tests
cargo test
```

## Puzzle formats

See [docs/formats.md](docs/formats.md) for the full specification of all supported formats.

The canonical JSON format is implemented in `core/src/io/json.rs`. The text and archived JSON
formats used by the CLI are in `core/src/io/text.rs` and `core/src/io/json.rs` respectively.
