#!/usr/bin/env bash
set -euo pipefail

BINARY="./target/release/compress-test"
SAMPLES_DIR="samples"
OUTPUTS_DIR="outputs"

cargo test >&2
cargo build --release >&2

mkdir -p "$OUTPUTS_DIR"

tmpdir=$(mktemp -d)
trap "rm -rf '$tmpdir'" EXIT

total_ratio=0
count=0

for input_file in "$SAMPLES_DIR"/*; do
    [ -f "$input_file" ] || continue

    filename=$(basename "$input_file")
    compressed_file="$OUTPUTS_DIR/${filename}_compressed"
    decompressed_file="$tmpdir/decompressed"

    "$BINARY" compress "$input_file" "$compressed_file" >&2
    "$BINARY" decompress "$compressed_file" "$decompressed_file" >&2

    if ! cmp -s "$input_file" "$decompressed_file"; then
        echo "Content mismatch for $filename" >&2
        exit 1
    fi

    original_size=$(wc -c < "$input_file" | awk '{print $1}')

    if [ "$original_size" -eq 0 ]; then
        echo "Skipping empty file $filename" >&2
        continue
    fi

    compressed_size=$(wc -c < "$compressed_file" | awk '{print $1}')

    ratio=$(awk "BEGIN { print $compressed_size / $original_size }")
    total_ratio=$(awk "BEGIN { print $total_ratio + $ratio }")
    count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
    echo "No sample files found in $SAMPLES_DIR" >&2
    exit 1
fi

awk "BEGIN { printf \"%.6f\n\", $total_ratio / $count }"
