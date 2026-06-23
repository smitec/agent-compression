"""Generate comparison charts from the benchmark CSVs for the write-up.

Reads the benchmark results captured during development and renders four
publication-quality PNGs into ``charts/``:

    evolution.png       compression ratio improving across lzss versions
    speed_vs_ratio.png  speed/ratio trade-off vs. external tools (holdout)
    ratio_by_type.png   mean ratio per file type, lzss-v10 vs. external tools
    overall_ratio.png   overall holdout ratio ranked across tools

Run with ``uv run make_charts.py`` (or ``python make_charts.py``).

Note on direction: ratio = compressed / original, so *lower is better*.
"""

from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
import seaborn as sns

ROOT = Path(__file__).resolve().parent
CHARTS = ROOT / "charts"
DPI = 150

# Our tool gets a bold, distinct colour; external tools stay muted.
OURS = "lzss-v10"
HIGHLIGHT = "#d62728"
MUTED = "#4c72b0"
TOOL_ORDER = [OURS, "brotli", "xz", "bzip2", "gzip", "zstd", "lz4"]


def _palette(tools):
    """Map tool name -> colour, highlighting our tool."""
    base = sns.color_palette("muted", n_colors=len(tools))
    palette = {}
    i = 0
    for tool in tools:
        if tool == OURS:
            palette[tool] = HIGHLIGHT
        else:
            palette[tool] = base[i]
            i += 1
    return palette


def save(fig, name):
    """Write ``fig`` to ``charts/<name>.png`` and report the path."""
    CHARTS.mkdir(exist_ok=True)
    out = CHARTS / f"{name}.png"
    fig.savefig(out, dpi=DPI, bbox_inches="tight")
    plt.close(fig)
    print(f"  wrote {out.relative_to(ROOT)}")
    return out


def chart_evolution():
    """Compression ratio improving across lzss versions (training set)."""
    df = pd.read_csv(ROOT / "results.csv")

    fig, ax = plt.subplots(figsize=(9, 5.5))
    x = range(len(df))
    ax.plot(x, df["compressible_ratio"], marker="o", color=HIGHLIGHT,
            linewidth=2, label="compressible files")
    ax.plot(x, df["overall_ratio"], marker="s", color=MUTED,
            linewidth=2, label="all files")

    ax.set_xticks(list(x))
    ax.set_xticklabels(df["branch"], rotation=45, ha="right")
    ax.set_ylabel("compression ratio  (lower = better)")
    ax.set_xlabel("")
    ax.set_title("Algorithm evolution: ratio improves from stub to lzss-v10")
    ax.set_ylim(0.4, 1.03)
    ax.legend(frameon=False)

    # Annotate the journey's endpoints on the compressible line.
    first, last = df.iloc[0], df.iloc[-1]
    ax.annotate(f"{first['compressible_ratio']:.2f}",
                (0, first["compressible_ratio"]),
                textcoords="offset points", xytext=(14, -4), ha="left")
    ax.annotate(f"{last['compressible_ratio']:.2f}",
                (len(df) - 1, last["compressible_ratio"]),
                textcoords="offset points", xytext=(0, -22), ha="center",
                color=HIGHLIGHT, fontweight="bold")

    fig.tight_layout()
    return save(fig, "evolution")


def chart_speed_vs_ratio():
    """Speed vs. ratio trade-off against external tools (holdout set)."""
    df = pd.read_csv(ROOT / "results-holdout.csv")
    palette = _palette(df["branch"].tolist())

    fig, ax = plt.subplots(figsize=(9, 6))
    for _, row in df.iterrows():
        tool = row["branch"]
        is_ours = tool == OURS
        ax.scatter(row["avg_compress_ms"], row["compressible_ratio"],
                   s=220 if is_ours else 140,
                   color=palette[tool],
                   edgecolor="black", linewidth=1.2 if is_ours else 0.6,
                   zorder=3)
        ax.annotate(tool,
                    (row["avg_compress_ms"], row["compressible_ratio"]),
                    textcoords="offset points", xytext=(8, 6),
                    fontweight="bold" if is_ours else "normal",
                    color=HIGHLIGHT if is_ours else "black")

    ax.set_xscale("log")
    ax.set_xlabel("avg compress time per file (ms, log scale)")
    ax.set_ylabel("compression ratio  (lower = better)")
    ax.set_title("Speed vs. ratio on the holdout set\n"
                 "bottom-left is best: small output, fast")
    ax.grid(True, which="both", axis="x", alpha=0.3)
    fig.tight_layout()
    return save(fig, "speed_vs_ratio")


def chart_ratio_by_type():
    """Mean compression ratio per file type, across all tools (holdout)."""
    df = pd.read_csv(ROOT / "debug-holdout.csv")
    df = df[df["ratio"] > 0]  # drop -1 failure/timeout rows

    means = (df.groupby(["type", "tool"])["ratio"].mean()
               .reset_index())

    # Lead with the types where compression actually does something.
    type_order = ["text", "data", "audio", "image", "binary", "video", "random"]
    type_order = [t for t in type_order if t in means["type"].unique()]
    tools = [t for t in TOOL_ORDER if t in means["tool"].unique()]
    palette = _palette(tools)

    fig, ax = plt.subplots(figsize=(12, 6))
    sns.barplot(data=means, x="type", y="ratio", hue="tool",
                order=type_order, hue_order=tools, palette=palette, ax=ax)
    ax.axhline(1.0, color="grey", linestyle="--", linewidth=1, alpha=0.6)
    ax.set_ylabel("mean compression ratio  (lower = better)")
    ax.set_xlabel("")
    ax.set_title("Where lzss-v10 wins: mean ratio by file type (holdout)")
    ax.legend(title="tool", frameon=False, loc="upper left",
              bbox_to_anchor=(1.01, 1.0))
    fig.tight_layout()
    return save(fig, "ratio_by_type")


def chart_overall_ratio():
    """Overall holdout ratio ranked across tools."""
    df = pd.read_csv(ROOT / "results-holdout.csv")
    df = df.sort_values("overall_ratio")  # best (lowest) first
    colors = [HIGHLIGHT if t == OURS else MUTED for t in df["branch"]]

    fig, ax = plt.subplots(figsize=(9, 5))
    bars = ax.barh(df["branch"], df["overall_ratio"], color=colors)
    ax.invert_yaxis()  # best at top
    ax.set_xlabel("overall compression ratio  (lower = better)")
    ax.set_title("Overall ratio on the holdout set")
    ax.set_xlim(0, 1.0)
    for bar, val in zip(bars, df["overall_ratio"]):
        ax.text(val + 0.01, bar.get_y() + bar.get_height() / 2,
                f"{val:.3f}", va="center")
    fig.tight_layout()
    return save(fig, "overall_ratio")


def main():
    sns.set_theme(style="whitegrid", context="talk")
    plt.rcParams["axes.titleweight"] = "bold"
    print("Generating charts...")
    chart_evolution()
    chart_speed_vs_ratio()
    chart_ratio_by_type()
    chart_overall_ratio()
    print("Done.")


if __name__ == "__main__":
    main()
