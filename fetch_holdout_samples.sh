#!/usr/bin/env bash
# Downloads a held-out set of public-domain sample files for compression benchmarking.
# Same file types as fetch_samples.sh but entirely different source material.
# Only fetches files not already present.  Safe to re-run.
set -euo pipefail

SAMPLES_DIR="${1:-samples-holdout}"
FETCH_PAUSE=3
mkdir -p "$SAMPLES_DIR"

fetch() {
    local name="$1" url="$2"
    local dest="$SAMPLES_DIR/$name"
    if [ -f "$dest" ]; then
        echo "exists:   $name" >&2
        return
    fi
    echo "fetching: $name" >&2
    if ! curl -L --fail --silent --show-error -o "$dest" "$url"; then
        echo "  failed: $name — removing partial file" >&2
        rm -f "$dest"
    fi
    sleep "$FETCH_PAUSE"
}

# ── Text — Project Gutenberg (different books from fetch_samples.sh) ────────────────────────────
fetch "text_count_monte_cristo.txt"    "https://www.gutenberg.org/files/1184/1184-0.txt"    # ~2.7 MB
fetch "text_great_expectations.txt"   "https://www.gutenberg.org/files/1400/1400-0.txt"    # ~1.0 MB
fetch "text_crime_and_punishment.txt" "https://www.gutenberg.org/files/2554/2554-0.txt"    # ~1.1 MB
fetch "text_huckleberry_finn.txt"     "https://www.gutenberg.org/files/76/76-0.txt"        # ~0.6 MB
fetch "text_sherlock_holmes.txt"      "https://www.gutenberg.org/files/1661/1661-0.txt"    # ~0.6 MB
fetch "text_middlemarch.txt"          "https://www.gutenberg.org/files/145/145-0.txt"      # ~1.9 MB

# ── Images — Wikimedia Commons (different NASA/HST photographs from fetch_samples.sh) ──────────
fetch "img_saturn_cassini.jpg"        "https://upload.wikimedia.org/wikipedia/commons/e/e3/Saturn_from_Cassini_Orbiter_%282004-10-06%29.jpg"  # ~5 MB
fetch "img_pluto_newhorizons.jpg"     "https://upload.wikimedia.org/wikipedia/commons/e/ef/Pluto_in_True_Color_-_High-Res.jpg"               # ~3 MB
fetch "img_crab_nebula_hst.jpg"       "https://upload.wikimedia.org/wikipedia/commons/0/00/Crab_Nebula.jpg"                                  # ~59 MB
fetch "img_andromeda_hst.jpg"         "https://upload.wikimedia.org/wikipedia/commons/9/98/Andromeda_Galaxy_%28with_h-alpha%29.jpg"          # ~3 MB
fetch "img_jupiter_grs.jpg"           "https://upload.wikimedia.org/wikipedia/commons/2/2b/Jupiter_and_its_shrunken_Great_Red_Spot.jpg"      # ~1 MB

# ── Audio — Wikimedia Commons (Beethoven 6 & 8, different from Beethoven 5 used originally) ────
# OGG (lossy): already compressed; expect ratio ≈ 1.0
fetch "audio_beethoven_6_mov1.ogg"    "https://upload.wikimedia.org/wikipedia/commons/d/d5/Ludwig_van_Beethoven_-_symphony_no._6_in_f_major_%27pastoral%27%2C_op._68_-_i._allegro_non_troppo.ogg"  # ~9 MB
fetch "audio_beethoven_6_mov4.ogg"    "https://upload.wikimedia.org/wikipedia/commons/1/15/Ludwig_van_Beethoven_-_symphony_no._6_in_f_major_%27pastoral%27%2C_op._68_-_iv._allegro.ogg"             # ~4 MB
fetch "audio_beethoven_8_mov4.ogg"    "https://upload.wikimedia.org/wikipedia/commons/2/2d/Ludwig_van_Beethoven_-_symphony_no._8_in_f_major%2C_op._93_-_iv._allegro_vivace.ogg"                     # ~9 MB
# Speech OGGs — different from Gettysburg Address used in fetch_samples.sh
fetch "audio_lincoln_inaugural.ogg"   "https://upload.wikimedia.org/wikipedia/commons/a/a5/Abraham_Lincoln%27s_Second_Inaugural_Address%3B_Read_by_Winston_Tharp_for_LibriVox.oga"
fetch "audio_federalist_10.ogg"       "https://upload.wikimedia.org/wikipedia/commons/5/56/LibriVox_-_The_Federalist_Papers-No._10.ogg"                                                              # ~13 MB
# FLAC (lossless): different piece from Bach Toccata used originally
fetch "audio_beethoven_egmont.flac"   "https://upload.wikimedia.org/wikipedia/commons/8/85/Beethoven_-_Egmont_Overture%2C_Op._84_%28Musopen_Symphony%29.flac"                                        # ~90 MB

# ── Structured data — different time window / format from fetch_samples.sh ──────────────────────
fetch "data_usgs_week.json"           "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_week.geojson"    # ~2 MB
fetch "data_usgs_month.csv"           "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_month.csv"       # ~3 MB

# ── Video — Blender Foundation open movies, different titles from fetch_samples.sh ─────────────
fetch "video_sintel.mp4"              "https://archive.org/download/Sintel/sintel-2048-stereo_512kb.mp4"              # ~74 MB  (CC BY)
fetch "video_tears_of_steel.mp4"      "https://archive.org/download/Tears-of-Steel/tears_of_steel_720p.mp4"           # ~73 MB  (CC BY)

# ── Derived — less-compressed versions of the above ─────────────────────────────────────────────
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "ffmpeg not found — skipping derived samples" >&2
else
    convert_sample() {
        local src="$SAMPLES_DIR/$1" dest="$SAMPLES_DIR/$2"
        shift 2
        if [ -f "$dest" ]; then
            echo "exists:     $(basename "$dest")" >&2
            return
        fi
        if [ ! -f "$src" ]; then
            echo "skipping:   $(basename "$dest") (source $(basename "$src") not present)" >&2
            return
        fi
        echo "converting: $(basename "$src") → $(basename "$dest")" >&2
        if ! ffmpeg -i "$src" "$@" -y "$dest" -loglevel error; then
            echo "  failed:   $(basename "$dest")" >&2
            rm -f "$dest"
        fi
    }

    # JPEG → BMP  (raw RGB, zero compression)
    convert_sample "img_saturn_cassini.jpg"    "img_saturn_cassini.bmp"
    convert_sample "img_pluto_newhorizons.jpg" "img_pluto_newhorizons.bmp"
    convert_sample "img_crab_nebula_hst.jpg"   "img_crab_nebula_hst.bmp"
    convert_sample "img_andromeda_hst.jpg"     "img_andromeda_hst.bmp"
    convert_sample "img_jupiter_grs.jpg"       "img_jupiter_grs.bmp"

    # JPEG → PNG  (lossless deflate)
    convert_sample "img_saturn_cassini.jpg"    "img_saturn_cassini.png"
    convert_sample "img_pluto_newhorizons.jpg" "img_pluto_newhorizons.png"
    convert_sample "img_crab_nebula_hst.jpg"   "img_crab_nebula_hst.png"
    convert_sample "img_andromeda_hst.jpg"     "img_andromeda_hst.png"
    convert_sample "img_jupiter_grs.jpg"       "img_jupiter_grs.png"

    # OGG/FLAC → WAV  (raw PCM)
    convert_sample "audio_beethoven_6_mov1.ogg"  "audio_beethoven_6_mov1.wav"
    convert_sample "audio_beethoven_6_mov4.ogg"  "audio_beethoven_6_mov4.wav"
    convert_sample "audio_beethoven_8_mov4.ogg"  "audio_beethoven_8_mov4.wav"
    convert_sample "audio_lincoln_inaugural.ogg" "audio_lincoln_inaugural.wav"
    convert_sample "audio_federalist_10.ogg"     "audio_federalist_10.wav"
    convert_sample "audio_beethoven_egmont.flac" "audio_beethoven_egmont.wav"

    # MP4 → WAV  (extract audio track only)
    convert_sample "video_sintel.mp4"         "video_sintel_audio.wav"         -vn -acodec pcm_s16le
    convert_sample "video_tears_of_steel.mp4" "video_tears_of_steel_audio.wav" -vn -acodec pcm_s16le
fi
