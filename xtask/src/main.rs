use anyhow::{bail, Result};
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => bench(),
        Some("dist") => dist(),
        Some(other) => bail!("unknown xtask: {other}"),
        None => {
            eprintln!("usage: cargo xtask <bench|dist>");
            Ok(())
        }
    }
}

fn bench() -> Result<()> {
    // Placeholder — v0.1 wires this up after the release binary stabilises.
    println!("xtask bench: coming in v0.1 after initial release-baseline capture");
    Ok(())
}

fn dist() -> Result<()> {
    println!("xtask dist: see `cargo dist` once cargo-dist is initialised in v0.1 final");
    Ok(())
}
