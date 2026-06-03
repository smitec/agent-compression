#!/usr/bin/env bash
# Benchmark lzss-v10 and standard compression tools against the held-out sample set.
# Writes results-holdout.csv (one summary row per tool, same schema as results.csv) and
# debug-holdout.csv (one row per tool × file, accumulates across all tools).
#
# Unlike benchmark.sh / benchmark_external.sh, a single file timing out does NOT abort
# the run — it records -1s for that file in the debug CSV and continues.  This keeps
# the debug CSV fully populated even when brotli times out on large WAV files.
set -euo pipefail

BINARY="./target/release/compress-test"
SAMPLES_DIR="samples-holdout"
RESULTS_CSV="results-holdout.csv"
DEBUG_CSV="debug-holdout.csv"
TIMEOUT_SECS=300

ms() { python3 -c "import time; print(int(time.time() * 1000))"; }

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
    trap "rm -f '$_timed_sentinel'" EXIT
fi

compress_file() {
    local tool="$1" input="$2" output="$3"
    case "$tool" in
        lzss-v10) timed "$BINARY" compress  "$input"   "$output" ;;
        gzip)     timed gzip   -c           "$input" > "$output" ;;
        bzip2)    timed bzip2  -c           "$input" > "$output" ;;
        xz)       timed xz     -c           "$input" > "$output" ;;
        zstd)     timed zstd   -q -c        "$input" > "$output" ;;
        lz4)      timed lz4    -f -q        "$input"   "$output" ;;
        brotli)   timed brotli -c           "$input" > "$output" ;;
        *) echo "Unknown tool: $tool" >&2; return 1 ;;
    esac
}

decompress_file() {
    local tool="$1" input="$2" output="$3"
    case "$tool" in
        lzss-v10) timed "$BINARY" decompress "$input"   "$output" ;;
        gzip)     timed gzip   -dc           "$input" > "$output" ;;
        bzip2)    timed bzip2  -dc           "$input" > "$output" ;;
        xz)       timed xz     -dc           "$input" > "$output" ;;
        zstd)     timed zstd   -q -dc        "$input" > "$output" ;;
        lz4)      timed lz4    -d -f -q      "$input"   "$output" ;;
        brotli)   timed brotli -dc           "$input" > "$output" ;;
        *) echo "Unknown tool: $tool" >&2; return 1 ;;
    esac
}

benchmark_tool() {
    local tool="$1"
    echo "=== $tool ===" >&2

    local tmpdir
    tmpdir=$(mktemp -d)

    local total_orig_bytes=0
    local total_comp_bytes=0
    local compressible_orig_bytes=0
    local compressible_comp_bytes=0
    local already_comp_orig_bytes=0
    local already_comp_comp_bytes=0
    local total_compress_ms=0
    local total_decompress_ms=0
    local count=0
    local failed_count=0

    for input_file in "$SAMPLES_DIR"/*; do
        [ -f "$input_file" ] || continue

        local filename original_size
        filename=$(basename "$input_file")
        local compressed_file="$tmpdir/${filename}.compressed"
        local decompressed_file="$tmpdir/${filename}.decompressed"

        original_size=$(wc -c < "$input_file" | awk '{print $1}')
        [ "$original_size" -eq 0 ] && continue

        echo "  $filename" >&2

        local t0 t1 t2 exit_code=0
        t0=$(ms)
        compress_file "$tool" "$input_file" "$compressed_file" 2>/dev/null || exit_code=$?
        if [ "$exit_code" -ne 0 ]; then
            [ "$exit_code" -eq 124 ] \
                && echo "    compress timed out (${TIMEOUT_SECS}s)" >&2 \
                || echo "    compress failed (exit $exit_code)" >&2
            rm -f "$compressed_file"
            echo "$tool,$filename,$original_size,-1,-1,-1,-1" >> "$DEBUG_CSV"
            failed_count=$((failed_count + 1))
            continue
        fi
        t1=$(ms)

        exit_code=0
        decompress_file "$tool" "$compressed_file" "$decompressed_file" 2>/dev/null || exit_code=$?
        if [ "$exit_code" -ne 0 ]; then
            [ "$exit_code" -eq 124 ] \
                && echo "    decompress timed out (${TIMEOUT_SECS}s)" >&2 \
                || echo "    decompress failed (exit $exit_code)" >&2
            rm -f "$compressed_file" "$decompressed_file"
            echo "$tool,$filename,$original_size,-1,-1,-1,-1" >> "$DEBUG_CSV"
            failed_count=$((failed_count + 1))
            continue
        fi
        t2=$(ms)

        local compressed_size compress_ms decompress_ms ratio
        compressed_size=$(wc -c < "$compressed_file" | awk '{print $1}')
        compress_ms=$((t1 - t0))
        decompress_ms=$((t2 - t1))
        ratio=$(awk "BEGIN { printf \"%.6f\", $compressed_size / $original_size }")

        echo "$tool,$filename,$original_size,$compressed_size,$ratio,$compress_ms,$decompress_ms" >> "$DEBUG_CSV"

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
        total_compress_ms=$((total_compress_ms + compress_ms))
        total_decompress_ms=$((total_decompress_ms + decompress_ms))
        count=$((count + 1))

        rm -f "$compressed_file" "$decompressed_file"
    done

    rm -rf "$tmpdir"

    local datetime
    datetime=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    if [ "$count" -eq 0 ]; then
        echo "  all files failed — recording -1 row" >&2
        echo "$tool,$datetime,-1,-1,-1,-1,-1" >> "$RESULTS_CSV"
        return
    fi

    local overall_ratio compressible_ratio already_ratio avg_compress_ms avg_decompress_ms
    overall_ratio=$(awk "BEGIN { printf \"%.6f\", $total_comp_bytes / $total_orig_bytes }")
    compressible_ratio=$([ "$compressible_orig_bytes" -gt 0 ] \
        && awk "BEGIN { printf \"%.6f\", $compressible_comp_bytes / $compressible_orig_bytes }" \
        || echo "N/A")
    already_ratio=$([ "$already_comp_orig_bytes" -gt 0 ] \
        && awk "BEGIN { printf \"%.6f\", $already_comp_comp_bytes / $already_comp_orig_bytes }" \
        || echo "N/A")
    avg_compress_ms=$(awk "BEGIN { printf \"%.3f\", $total_compress_ms / $count }")
    avg_decompress_ms=$(awk "BEGIN { printf \"%.3f\", $total_decompress_ms / $count }")

    if [ "$failed_count" -gt 0 ]; then
        echo "  $failed_count file(s) timed out; metrics from $count successful files" >&2
    fi
    echo "  overall=$overall_ratio compressible=$compressible_ratio already=$already_ratio compress=${avg_compress_ms}ms decompress=${avg_decompress_ms}ms" >&2
    echo "$tool,$datetime,$overall_ratio,$compressible_ratio,$already_ratio,$avg_compress_ms,$avg_decompress_ms" >> "$RESULTS_CSV"
}

# ── Setup ───────────────────────────────────────────────────────────────────────────────────────

cargo test >&2
cargo build --release >&2

mkdir -p "$SAMPLES_DIR"
[ -f "$SAMPLES_DIR/random_1mb.bin"   ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_1mb.bin"   bs=1048576 count=1   2>/dev/null
[ -f "$SAMPLES_DIR/random_10mb.bin"  ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_10mb.bin"  bs=1048576 count=10  2>/dev/null
[ -f "$SAMPLES_DIR/random_100mb.bin" ] || dd if=/dev/urandom of="$SAMPLES_DIR/random_100mb.bin" bs=1048576 count=100 2>/dev/null
[ -f "$SAMPLES_DIR/binary_compress_test" ] || cp "$BINARY" "$SAMPLES_DIR/binary_compress_test"

# Initialise output files
echo "tool,filename,original_bytes,compressed_bytes,ratio,compress_ms,decompress_ms" > "$DEBUG_CSV"
[ -f "$RESULTS_CSV" ] || echo "branch,datetime,overall_ratio,compressible_ratio,already_compressed_ratio,avg_compress_ms,avg_decompress_ms" > "$RESULTS_CSV"

# ── Run all tools ───────────────────────────────────────────────────────────────────────────────
for tool in lzss-v10 gzip bzip2 xz zstd lz4 brotli; do
    benchmark_tool "$tool"
done
