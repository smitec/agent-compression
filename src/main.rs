mod core;

use core::{compress, decompress};
use std::fs::File;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: {} <compress|decompress> <input> <output>", args[0]);
        std::process::exit(1);
    }
    let mode = &args[1];
    let input_path = &args[2];
    let output_path = &args[3];

    let input = File::open(input_path).expect("Failed to open input file");
    let output = File::create(output_path).expect("Failed to create output file");

    match mode.as_str() {
        "compress" => compress(input, output).expect("Compression failed"),
        "decompress" => decompress(input, output).expect("Decompression failed"),
        _ => {
            eprintln!("Unknown mode '{}'. Use 'compress' or 'decompress'.", mode);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_file_roundtrip() {
        let pid = std::process::id();
        let tmp = std::env::temp_dir();
        let input_path = tmp.join(format!("compress_test_input_{}.bin", pid));
        let compressed_path = tmp.join(format!("compress_test_compressed_{}.bin", pid));
        let output_path = tmp.join(format!("compress_test_output_{}.bin", pid));

        let content = b"Hello from the file system!";
        std::fs::write(&input_path, content).unwrap();

        compress(File::open(&input_path).unwrap(), File::create(&compressed_path).unwrap()).unwrap();
        decompress(File::open(&compressed_path).unwrap(), File::create(&output_path).unwrap()).unwrap();

        let result = std::fs::read(&output_path).unwrap();

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&compressed_path);
        let _ = std::fs::remove_file(&output_path);

        assert_eq!(result, content);
    }

    #[test]
    fn test_roundtrip() {
        let input: &[u8] = b"Hello, world!";
        let mut compressed = Cursor::new(Vec::new());
        compress(Cursor::new(input), &mut compressed).unwrap();

        let mut uncompressed = Cursor::new(Vec::new());
        compressed.set_position(0);
        decompress(compressed, &mut uncompressed).unwrap();

        let s = String::from_utf8(uncompressed.into_inner()).expect("Found invalid UTF-8");
        assert_eq!(s, "Hello, world!");
    }
}
