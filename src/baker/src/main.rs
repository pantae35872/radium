use std::{
    env,
    fs::File,
    io::{BufWriter, Write},
};

use addr2line::Context;
use bakery::Bakery;
use object::{Object, ObjectSymbol, SymbolKind};

fn main() {
    let mut args = env::args().skip(1);
    let target_binary: String = args.next().expect("No elf file provided");
    let output_file: String = args.next().expect("Output path provided");
    let binary_data = std::fs::read(target_binary).expect("Failed to read binary");
    let obj_file = object::File::parse(&*binary_data).expect("Failed to parse object file");
    let context = Context::new(&obj_file).unwrap();
    let output_file = File::create(output_file).unwrap();
    let mut output = BufWriter::new(output_file);
    let mut sorted = Vec::new();
    let mut bakery = Bakery::new();

    for symbol in obj_file.symbols().filter(|e| e.kind() == SymbolKind::Text) {
        let addr = symbol.address();
        let name = match symbol.name() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let mut location_str = "unknown";
        let mut line_num = 0;
        if let Ok(mut frames) = context.find_frames(addr).skip_all_loads() {
            if let Ok(Some(frame)) = frames.next() {
                if let Some(loc) = frame.location {
                    if let Some(file) = loc.file {
                        location_str = file;
                    }
                    if let Some(line) = loc.line {
                        line_num = line;
                    }
                }
            }
        }

        let name = rustc_demangle::demangle(name).to_string();
        sorted.push((addr, addr + symbol.size(), line_num, name, location_str));
    }

    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    sorted
        .iter()
        .for_each(|(addr, end, line_num, name, location)| {
            bakery.push(*addr, *end, *line_num, name, location)
        });

    output
        .write_all(&bakery.bake())
        .expect("Failed to save to output file");
}
