# compress-test

An experiment in building a lossless compressor from scratch in Rust, tuned for
PCM audio. It evolves from a do-nothing stub into **lzss-v10** — an LZSS core
with Huffman entropy coding, multi-stride delta filters, and audio-specific
prediction. A write-up is linked here later.

Ratio is `compressed / original`, so **lower is better**. Charts are generated
from the benchmark CSVs with `uv run make_charts.py`.

## Charts

**Algorithm evolution** — compression ratio across each version, from the stub
to lzss-v10, on the training set.

![Compression ratio across versions](charts/evolution.png)

**Overall ratio by tool** — lzss-v10 against gzip, bzip2, xz, zstd, lz4 and
brotli on the holdout set.

![Overall ratio by tool](charts/overall_ratio.png)

**Speed vs. ratio** — the trade-off between compression time and ratio across
all tools on the holdout set.

![Speed vs ratio](charts/speed_vs_ratio.png)

**Ratio by file type** — mean compression ratio per file type (audio, text,
data, image, …) for each tool on the holdout set.

![Mean ratio by file type](charts/ratio_by_type.png)
