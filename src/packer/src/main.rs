use std::{
    env,
    fs::{read_dir, File},
    io::{Read, Write},
};

use packery::Packery;

fn main() {
    let mut args = env::args().skip(1);
    let target_directory: String = args.next().expect("Directory provided");
    let output_file: String = args.next().expect("Output path provided");

    let mut packery = Packery::new();
    let mut buf = Vec::new();
    for file in read_dir(target_directory)
        .expect("Target directory dosn't exists")
        .filter_map(|e| e.ok())
    {
        if file.path().extension().is_some_and(|e| e == "so") {
            let mut driver = File::open(file.path()).expect("Failed to open driver file");
            driver
                .read_to_end(&mut buf)
                .expect("Failed to read driver file");

            packery.push(
                file.path()
                    .with_extension("")
                    .file_name()
                    .unwrap()
                    .try_into()
                    .expect("Driver name invalid"),
                &buf,
            );
            buf.clear();
        }
    }

    let mut file = File::create(output_file).expect("Failed to write to output file");
    file.write_all(&packery.pack())
        .expect("Failed to write to output file");
}
