# cargo-debug

A subcommand for cargo that launches the specified debugger on the output of a provided subcommand.

## Usage

Install with `cargo install --locked --git https://github.com/DrChat/cargo-debug.git`

- `cargo debug` to run your crate in the debugger.
- `cargo debug windbg` to run your crate in a specific debugger (this one being `windbg`).
- `cargo debug --bin my-bin` to run a specific binary in a workspace.

## Status

[![GitHub tag](https://img.shields.io/github/tag/DrChat/cargo-debug.svg)](https://github.com/DrChat/cargo-debug)
![Build Status](https://github.com/DrChat/cargo-debug/actions/workflows/ci.yaml/badge.svg)

[Open Issues](https://github.com/DrChat/cargo-debug/issues)

