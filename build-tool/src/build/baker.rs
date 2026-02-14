use addr2line::Context;
use bakery::Bakery;
use object::{Object, ObjectSymbol, SymbolKind};

pub fn bake(target_binary: &[u8]) -> Vec<u8> {
    let obj_file = object::File::parse(target_binary).expect("Failed to parse object file");
    let context = Context::new(&obj_file).unwrap();
    let mut sorted = Vec::new();
    let mut bakery = Bakery::new();

    for symbol in obj_file.symbols().filter(|e| e.kind() == SymbolKind::Text) {
        let addr = symbol.address();
        let Ok(name) = symbol.name() else {
            continue;
        };
        let mut location_str = "unknown";
        let mut line_num = 0;
        if let Ok(Some(frame)) = context.find_frames(addr).skip_all_loads().and_then(|mut f| f.next())
            && let Some((Some(file), Some(line))) = frame.location.map(|loc| (loc.file, loc.line))
        {
            location_str = file;
            line_num = line;
        }

        let name = rustc_demangle::demangle(name).to_string();
        sorted.push((addr, addr + symbol.size(), line_num, name, location_str));
    }

    sorted.sort_by_key(|e| e.0);

    sorted.iter().for_each(|(addr, end, line_num, name, location)| bakery.push(*addr, *end, *line_num, name, location));

    bakery.bake()
}
