use std::{
    env,
    fs::File,
    io::{BufWriter, Write},
};

use addr2line::Context;
use object::{Object, ObjectSymbol, SymbolKind};

const MAGIC: u32 = u32::from_le_bytes(*b"BAKE");

struct BakerSymbol {
    addr: u64,
    line_num: u32,
    name: String,
    location: String,
}

struct BakerString {
    // Offset in the string table
    offset: u64,
    size: u64,
}

struct BakerSymbolFile {
    addr: u64,
    line_num: u32,
    name: BakerString,
    location: BakerString,
}

impl BakerString {
    pub fn write(&self, writer: &mut impl Write) {
        writer.write_all(&self.offset.to_le_bytes()).unwrap();
        writer.write_all(&self.size.to_le_bytes()).unwrap();
    }
}

impl BakerSymbolFile {
    pub fn write(&self, writer: &mut impl Write) {
        writer.write_all(&self.addr.to_le_bytes()).unwrap();
        writer.write_all(&self.line_num.to_le_bytes()).unwrap();
        self.name.write(writer);
        self.location.write(writer);
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let target_binary: String = args.next().expect("No elf file provided");
    let output_file: String = args.next().expect("Output path provided");
    let binary_data = std::fs::read(target_binary).expect("Failed to read binary");
    let obj_file = object::File::parse(&*binary_data).expect("Failed to parse object file");
    let context = Context::new(&obj_file).unwrap();
    let output_file = File::create(output_file).unwrap();
    let mut output = BufWriter::new(output_file);

    // write the magic ofc
    output
        .write_all(&MAGIC.to_le_bytes())
        .expect("Failed to write to the output file");

    let mut symbols = Vec::new();
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
        symbols.push(BakerSymbol {
            addr,
            line_num,
            name,
            location: location_str.to_string(),
        });
    }

    // next is the length of the symbols
    output.write_all(&symbols.len().to_le_bytes()).unwrap();

    let mut string_table = String::new();
    let mut symbols_file = Vec::new();

    for symbol in symbols {
        let location_offset = string_table.len();
        string_table += &symbol.location;
        let name_offset = string_table.len();
        string_table += &symbol.name;
        symbols_file.push(BakerSymbolFile {
            addr: symbol.addr,
            line_num: symbol.line_num,
            name: BakerString {
                offset: name_offset as u64,
                size: symbol.name.len() as u64,
            },
            location: BakerString {
                offset: location_offset as u64,
                size: symbol.location.len() as u64,
            },
        });
    }

    symbols_file.iter().for_each(|e| e.write(&mut output));

    output.write_all(&string_table.len().to_le_bytes()).unwrap();
    output.write_all(string_table.as_bytes()).unwrap();
}
