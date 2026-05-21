#!/usr/bin/env bash
set -euo pipefail

BINARY="./target/release/compress-test"
SAMPLES_DIR="samples"
OUTPUTS_DIR="outputs"
TIMEOUT_SECS=300
EXPANSION_LIMIT=1.5   # fail if compressed output exceeds original by this factor
RESULTS_CSV="results.csv"

RECORD=false
[ "${1:-}" = "--record" ] && RECORD=true

cargo test >&2
cargo build --release >&2

mkdir -p "$OUTPUTS_DIR"

[ -f "$SAMPLES_DIR/random_1mb.bin"   ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_1mb.bin"   bs=1048576 count=1   2>/dev/null
[ -f "$SAMPLES_DIR/random_10mb.bin"  ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_10mb.bin"  bs=1048576 count=10  2>/dev/null
[ -f "$SAMPLES_DIR/random_100mb.bin" ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_100mb.bin" bs=1048576 count=100 2>/dev/null

# Keep a copy of the compiled binary as a structured-binary sample.
[ -f "$SAMPLES_DIR/binary_compress_test" ] || cp "$BINARY" "$SAMPLES_DIR/binary_compress_test"

DEBUG_CSV="debug.csv"
echo "filename,original_bytes,compressed_bytes,ratio,compress_ms,decompress_ms" > "$DEBUG_CSV"

tmpdir=$(mktemp -d)
trap "rm -rf '$tmpdir'" EXIT

ms() { python3 -c "import time; print(int(time.time() * 1000))"; }

# Prefer system timeout, fall back to pure bash watchdog that returns 124 on timeout
if command -v timeout >/dev/null 2>&1; then
    timed() { timeout "$TIMEOUT_SECS" "$@"; }
elif command -v gtimeout >/dev/null 2>&1; then
    timed() { gtimeout "$TIMEOUT_SECS" "$@"; }
else
    _timed_sentinel=$(mktemp)
    timed() {
        echo 0 > "$_timed_sentinel"
        "$@" &
        local pid=$!
        (sleep "$TIMEOUT_SECS" && echo 1 > "$_timed_sentinel" && kill "$pid" 2>/dev/null) &
        local watchdog=$!
        wait "$pid" 2>/dev/null
        local code=$?
        kill "$watchdog" 2>/dev/null
        wait "$watchdog" 2>/dev/null
        [ "$(cat "$_timed_sentinel")" = "1" ] && return 124
        return $code
    }
    trap "rm -f '$_timed_sentinel'; rm -rf '$tmpdir'" EXIT
fi

# Byte accumulators for size-weighted ratios.
# Already-compressed: formats where compression is expected to have no effect (jpg, ogg, flac, mp4, png).
# Compressible: everything else (txt, bmp, wav, bin, csv, json, executables, etc.).
total_orig_bytes=0
total_comp_bytes=0
compressible_orig_bytes=0
compressible_comp_bytes=0
already_comp_orig_bytes=0
already_comp_comp_bytes=0
total_compress_ms=0
total_decompress_ms=0
count=0

decompressed_file="$tmpdir/decompressed"
determinism_file="$tmpdir/determinism"

for input_file in "$SAMPLES_DIR"/*; do
    [ -f "$input_file" ] || continue

    filename=$(basename "$input_file")
    compressed_file="$OUTPUTS_DIR/${filename}_compressed"

    original_size=$(wc -c < "$input_file" | awk '{print $1}')
    if [ "$original_size" -eq 0 ]; then
        echo "Skipping empty file $filename" >&2
        continue
    fi

    t0=$(ms)
    timed "$BINARY" compress "$input_file" "$compressed_file" >&2 || {
        code=$?
        [ $code -eq 124 ] && echo "Compress timed out for $filename (${TIMEOUT_SECS}s limit)" >&2 \
                           || echo "Compress failed for $filename (exit $code)" >&2
        exit 1
    }
    t1=$(ms)

    timed "$BINARY" decompress "$compressed_file" "$decompressed_file" >&2 || {
        code=$?
        [ $code -eq 124 ] && echo "Decompress timed out for $filename (${TIMEOUT_SECS}s limit)" >&2 \
                           || echo "Decompress failed for $filename (exit $code)" >&2
        exit 1
    }
    t2=$(ms)

    if ! cmp -s "$input_file" "$decompressed_file"; then
        echo "Content mismatch for $filename" >&2
        exit 1
    fi

    compressed_size=$(wc -c < "$compressed_file" | awk '{print $1}')

    # Expansion guard: a ratio this high indicates something is badly wrong.
    if awk "BEGIN { exit !($compressed_size > $original_size * $EXPANSION_LIMIT) }"; then
        echo "Output too large for $filename: $original_size → $compressed_size bytes ($(awk "BEGIN { printf \"%.2f\", $compressed_size / $original_size }")×, limit ${EXPANSION_LIMIT}×)" >&2
        exit 1
    fi

    # Determinism check: compress again and verify bit-identical output.
    timed "$BINARY" compress "$input_file" "$determinism_file" >&2 || {
        code=$?
        [ $code -eq 124 ] && echo "Determinism check timed out for $filename (${TIMEOUT_SECS}s limit)" >&2 \
                           || echo "Determinism check failed for $filename (exit $code)" >&2
        exit 1
    }
    if ! cmp -s "$compressed_file" "$determinism_file"; then
        echo "Non-deterministic compression for $filename" >&2
        exit 1
    fi

    ratio=$(awk "BEGIN { print $compressed_size / $original_size }")
    echo "$filename,$original_size,$compressed_size,$ratio,$((t1 - t0)),$((t2 - t1))" >> "$DEBUG_CSV"

    total_orig_bytes=$((total_orig_bytes + original_size))
    total_comp_bytes=$((total_comp_bytes + compressed_size))
    case "$filename" in
        *.jpg|*.ogg|*.flac|*.mp4|*.png)
            already_comp_orig_bytes=$((already_comp_orig_bytes + original_size))
            already_comp_comp_bytes=$((already_comp_comp_bytes + compressed_size))
            ;;
        *)
            compressible_orig_bytes=$((compressible_orig_bytes + original_size))
            compressible_comp_bytes=$((compressible_comp_bytes + compressed_size))
            ;;
    esac

    total_compress_ms=$((total_compress_ms + t1 - t0))
    total_decompress_ms=$((total_decompress_ms + t2 - t1))
    count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
    echo "No sample files found in $SAMPLES_DIR" >&2
    exit 1
fi

overall_ratio=$(awk "BEGIN { printf \"%.6f\", $total_comp_bytes / $total_orig_bytes }")
compressible_ratio=$([ "$compressible_orig_bytes" -gt 0 ] \
    && awk "BEGIN { printf \"%.6f\", $compressible_comp_bytes / $compressible_orig_bytes }" \
    || echo "N/A")
already_ratio=$([ "$already_comp_orig_bytes" -gt 0 ] \
    && awk "BEGIN { printf \"%.6f\", $already_comp_comp_bytes / $already_comp_orig_bytes }" \
    || echo "N/A")
avg_compress_ms=$(awk "BEGIN { printf \"%.3f\", $total_compress_ms   / $count }")
avg_decompress_ms=$(awk "BEGIN { printf \"%.3f\", $total_decompress_ms / $count }")

echo "$overall_ratio"
echo "$compressible_ratio"
echo "$already_ratio"
echo "$avg_compress_ms"
echo "$avg_decompress_ms"

if [ "$RECORD" = "true" ]; then
    branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
    datetime=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    echo "$branch,$datetime,$overall_ratio,$compressible_ratio,$already_ratio,$avg_compress_ms,$avg_decompress_ms" >> "$RESULTS_CSV"
fi
