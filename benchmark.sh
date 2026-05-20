#!/usr/bin/env bash
set -euo pipefail

BINARY="./target/release/compress-test"
SAMPLES_DIR="samples"
OUTPUTS_DIR="outputs"

cargo test >&2
cargo build --release >&2

mkdir -p "$OUTPUTS_DIR"

[ -f "$SAMPLES_DIR/random_1mb.bin"   ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_1mb.bin"   bs=1048576 count=1   2>/dev/null
[ -f "$SAMPLES_DIR/random_10mb.bin"  ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_10mb.bin"  bs=1048576 count=10  2>/dev/null
[ -f "$SAMPLES_DIR/random_100mb.bin" ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_100mb.bin" bs=1048576 count=100 2>/dev/null

tmpdir=$(mktemp -d)
trap "rm -rf '$tmpdir'" EXIT

ms() { python3 -c "import time; print(int(time.time() * 1000))"; }

total_ratio=0
total_compress_ms=0
total_decompress_ms=0
count=0

for input_file in "$SAMPLES_DIR"/*; do
    [ -f "$input_file" ] || continue

    filename=$(basename "$input_file")
    compressed_file="$OUTPUTS_DIR/${filename}_compressed"
    decompressed_file="$tmpdir/decompressed"

    original_size=$(wc -c < "$input_file" | awk '{print $1}')
    if [ "$original_size" -eq 0 ]; then
        echo "Skipping empty file $filename" >&2
        continue
    fi

    t0=$(ms)
    "$BINARY" compress "$input_file" "$compressed_file" >&2
    t1=$(ms)
    "$BINARY" decompress "$compressed_file" "$decompressed_file" >&2
    t2=$(ms)

    if ! cmp -s "$input_file" "$decompressed_file"; then
        echo "Content mismatch for $filename" >&2
        exit 1
    fi

    compressed_size=$(wc -c < "$compressed_file" | awk '{print $1}')

    ratio=$(awk "BEGIN { print $compressed_size / $original_size }")
    total_ratio=$(awk "BEGIN { print $total_ratio + $ratio }")
    total_compress_ms=$((total_compress_ms + t1 - t0))
    total_decompress_ms=$((total_decompress_ms + t2 - t1))
    count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
    echo "No sample files found in $SAMPLES_DIR" >&2
    exit 1
fi

awk "BEGIN { printf \"%.6f\n\", $total_ratio / $count }"
awk "BEGIN { printf \"%.3f\n\", $total_compress_ms / $count }"
awk "BEGIN { printf \"%.3f\n\", $total_decompress_ms / $count }"
