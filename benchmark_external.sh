#!/usr/bin/env bash
# Benchmark standard compression tools against the same sample set and metrics as benchmark.sh.
# Appends one row per tool to results.csv using the tool name in the "branch" column.
set -euo pipefail

SAMPLES_DIR="samples"
RESULTS_CSV="results.csv"
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
        gzip)   timed gzip   -c       "$input" > "$output" ;;
        bzip2)  timed bzip2  -c       "$input" > "$output" ;;
        xz)     timed xz     -c       "$input" > "$output" ;;
        zstd)   timed zstd   -q -c    "$input" > "$output" ;;
        lz4)    timed lz4    -f -q    "$input"   "$output" ;;
        brotli) timed brotli -c       "$input" > "$output" ;;
        *) echo "Unknown tool: $tool" >&2; return 1 ;;
    esac
}

decompress_file() {
    local tool="$1" input="$2" output="$3"
    case "$tool" in
        gzip)   timed gzip   -dc      "$input" > "$output" ;;
        bzip2)  timed bzip2  -dc      "$input" > "$output" ;;
        xz)     timed xz     -dc      "$input" > "$output" ;;
        zstd)   timed zstd   -q -dc   "$input" > "$output" ;;
        lz4)    timed lz4    -d -f -q "$input"   "$output" ;;
        brotli) timed brotli -dc      "$input" > "$output" ;;
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
    local failed=false
    local fail_reason=""

    for input_file in "$SAMPLES_DIR"/*; do
        [ -f "$input_file" ] || continue

        local filename
        filename=$(basename "$input_file")
        local compressed_file="$tmpdir/${filename}.compressed"
        local decompressed_file="$tmpdir/${filename}.decompressed"

        local original_size
        original_size=$(wc -c < "$input_file" | awk '{print $1}')
        [ "$original_size" -eq 0 ] && continue

        echo "  $filename" >&2

        local t0 t1 t2 exit_code=0
        t0=$(ms)
        compress_file "$tool" "$input_file" "$compressed_file" 2>/dev/null || exit_code=$?
        if [ "$exit_code" -ne 0 ]; then
            [ "$exit_code" -eq 124 ] \
                && fail_reason="compress timed out (${TIMEOUT_SECS}s) on $filename" \
                || fail_reason="compress failed (exit $exit_code) on $filename"
            failed=true
            break
        fi
        t1=$(ms)

        exit_code=0
        decompress_file "$tool" "$compressed_file" "$decompressed_file" 2>/dev/null || exit_code=$?
        if [ "$exit_code" -ne 0 ]; then
            [ "$exit_code" -eq 124 ] \
                && fail_reason="decompress timed out (${TIMEOUT_SECS}s) on $filename" \
                || fail_reason="decompress failed (exit $exit_code) on $filename"
            failed=true
            break
        fi
        t2=$(ms)

        local compressed_size
        compressed_size=$(wc -c < "$compressed_file" | awk '{print $1}')

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

    rm -rf "$tmpdir"

    local datetime
    datetime=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    if $failed; then
        echo "  FAILED: $fail_reason" >&2
        echo "$tool,$datetime,-1,-1,-1,-1,-1" >> "$RESULTS_CSV"
        return
    fi

    if [ "$count" -eq 0 ]; then
        echo "  No sample files found in $SAMPLES_DIR" >&2
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

    echo "$tool,$datetime,$overall_ratio,$compressible_ratio,$already_ratio,$avg_compress_ms,$avg_decompress_ms" >> "$RESULTS_CSV"
    echo "  overall=$overall_ratio compressible=$compressible_ratio already=$already_ratio compress=${avg_compress_ms}ms decompress=${avg_decompress_ms}ms" >&2
}

for tool in gzip bzip2 xz zstd lz4 brotli; do
    benchmark_tool "$tool"
done
