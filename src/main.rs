mod ast;
mod env;
mod error;
mod interpreter;
mod lexer;
mod parser;
mod token;
mod value;

use std::fs;

use interpreter::Interpreter;
use parser::Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // argv[1] = script file
    let mut args = std::env::args();
    let prog_name = args.next().unwrap(); // argv[0]

    let filename = match args.next() {
        Some(f) => f,
        None => {
            eprintln!("Usage: {} <script-file>", prog_name);
            std::process::exit(1);
        }
    };

    let src = fs::read_to_string(&filename).map_err(|e| {
        eprintln!("Could not read file '{}': {}", filename, e);
        e
    })?;

    let mut p = Parser::new(&src)?;
    let prog = p.parse_program()?;

    let mut i = Interpreter::new();
    if let Err(e) = i.run(prog) {
        eprintln!("{}", e);
    }
    Ok(())
}
