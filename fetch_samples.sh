#!/usr/bin/env bash
# Downloads public-domain sample files for compression benchmarking.
# Only fetches files not already present in samples/.  Safe to re-run.
#
# No single file exceeds ~250 MB.  All URLs verified via Wikimedia/Archive APIs.
#
# What each category tells you about the algorithm:
#   text  — high compressibility; plain UTF-8 text compresses very well
#   image — JPEG is already lossy-compressed;   expect ratio ≈ 1.0
#   audio — OGG is already lossy-compressed;    expect ratio ≈ 1.0
#           FLAC is losslessly compressed;       expect ratio somewhat < 1.0
#   video — H.264 MP4 is already compressed;    expect ratio ≈ 1.0
set -euo pipefail

SAMPLES_DIR="${1:-samples}"
FETCH_PAUSE=3   # seconds to sleep after each download (skipped files don't count)
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

# ── Text — Project Gutenberg (public domain, stable) ───────────────────────────────────────────
fetch "text_war_and_peace.txt"        "https://www.gutenberg.org/files/2600/2600-0.txt"                 #  ~3 MB
fetch "text_shakespeare.txt"          "https://www.gutenberg.org/files/100/100-0.txt"                   #  ~6 MB
fetch "text_moby_dick.txt"            "https://www.gutenberg.org/files/2701/2701-0.txt"                 #  ~1 MB
fetch "text_bible_kjv.txt"            "https://www.gutenberg.org/files/10/10-0.txt"                     #  ~4 MB
fetch "text_don_quixote.txt"          "https://www.gutenberg.org/files/996/996-0.txt"                   #  ~2 MB
fetch "text_les_miserables.txt"       "https://www.gutenberg.org/files/135/135-0.txt"                   #  ~3 MB

# ── Images — Wikimedia Commons (public domain NASA photographs, JPEG) ──────────────────────────
# Already lossy-compressed; expect ratio ≈ 1.0 on these.
fetch "img_blue_marble.jpg"           "https://upload.wikimedia.org/wikipedia/commons/9/97/The_Earth_seen_from_Apollo_17.jpg"              #  ~8 MB
fetch "img_earthrise.jpg"             "https://upload.wikimedia.org/wikipedia/commons/a/a8/NASA-Apollo8-Dec24-Earthrise.jpg"               #  ~1 MB
fetch "img_buzz_aldrin.jpg"           "https://upload.wikimedia.org/wikipedia/commons/9/9c/Aldrin_Apollo_11.jpg"                           #  ~3 MB
fetch "img_hubble_pillars.jpg"        "https://upload.wikimedia.org/wikipedia/commons/6/68/Pillars_of_creation_2014_HST_WFC3-UVIS_full-res_denoised.jpg"  # ~47 MB
fetch "img_mars_north_pole.jpg"       "https://upload.wikimedia.org/wikipedia/commons/6/62/Martian_north_polar_cap.jpg"                   #  ~1 MB

# ── Audio — Wikimedia Commons (public domain recordings) ───────────────────────────────────────
# OGG (lossy): already compressed; expect ratio ≈ 1.0
# FLAC (lossless): internally compressed but residuals may compress further
fetch "audio_beethoven_5_mov1.ogg"    "https://upload.wikimedia.org/wikipedia/commons/5/5b/Ludwig_van_Beethoven_-_Symphonie_5_c-moll_-_1._Allegro_con_brio.ogg"   #  ~7 MB
fetch "audio_beethoven_5_mov2.ogg"    "https://upload.wikimedia.org/wikipedia/commons/6/6b/Ludwig_van_Beethoven_-_Symphonie_5_c-moll_-_2._Andante_con_moto.ogg"    # ~13 MB
fetch "audio_beethoven_5_mov4.ogg"    "https://upload.wikimedia.org/wikipedia/commons/3/3b/Ludwig_van_Beethoven_-_Symphonie_5_c-moll_-_4._Allegro.ogg"              # ~14 MB
fetch "audio_gettysburg.ogg"          "https://upload.wikimedia.org/wikipedia/commons/a/a1/Gettysburg_by_Britton.ogg"
fetch "audio_gettysburg_librivox.ogg" "https://upload.wikimedia.org/wikipedia/commons/3/30/LibriVox_-_Everrett_Copy_of_the_Gettysburg_Address_-_Michael_Scherer.ogg"
fetch "audio_bach_toccata.flac"       "https://upload.wikimedia.org/wikipedia/commons/2/20/PDP-CH_-_Philadelphia_Orchestra%2C_Leopold_Stokowski_-_Toccata_and_Fugue_in_D_minor%2C_BWV_565_-_Bach_-_Hmv-d1428-5-0761.flac"  # ~166 MB

# ── Video — Internet Archive (CC-licensed short films, H.264 MP4) ──────────────────────────────
# Already compressed; expect ratio ≈ 1.0 on these.
fetch "video_elephants_dream.mp4"     "https://archive.org/download/ElephantsDream/ed_1024_512kb.mp4"          # ~47 MB  (CC BY)
fetch "video_big_buck_bunny.mp4"      "https://archive.org/download/BigBuckBunny_328/BigBuckBunny_512kb.mp4"   # ~41 MB  (CC BY)

# ── Derived — less-compressed versions of the above ─────────────────────────────────────────────
# Kept alongside originals so the same content appears at different entropy levels.
# Requires ffmpeg.  Each conversion is skipped if the output already exists or the source is missing.
# Extra arguments after the two filenames are passed straight through to ffmpeg.
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
    convert_sample "img_blue_marble.jpg"          "img_blue_marble.bmp"
    convert_sample "img_earthrise.jpg"            "img_earthrise.bmp"
    convert_sample "img_buzz_aldrin.jpg"          "img_buzz_aldrin.bmp"
    convert_sample "img_hubble_pillars.jpg"       "img_hubble_pillars.bmp"
    convert_sample "img_mars_north_pole.jpg"      "img_mars_north_pole.bmp"

    # JPEG → PNG  (lossless deflate — between JPEG and raw)
    convert_sample "img_blue_marble.jpg"          "img_blue_marble.png"
    convert_sample "img_earthrise.jpg"            "img_earthrise.png"
    convert_sample "img_buzz_aldrin.jpg"          "img_buzz_aldrin.png"
    convert_sample "img_hubble_pillars.jpg"       "img_hubble_pillars.png"
    convert_sample "img_mars_north_pole.jpg"      "img_mars_north_pole.png"

    # OGG/FLAC → WAV  (raw PCM, zero compression)
    convert_sample "audio_beethoven_5_mov1.ogg"   "audio_beethoven_5_mov1.wav"
    convert_sample "audio_beethoven_5_mov2.ogg"   "audio_beethoven_5_mov2.wav"
    convert_sample "audio_beethoven_5_mov4.ogg"   "audio_beethoven_5_mov4.wav"
    convert_sample "audio_gettysburg.ogg"         "audio_gettysburg.wav"
    convert_sample "audio_gettysburg_librivox.ogg" "audio_gettysburg_librivox.wav"
    convert_sample "audio_bach_toccata.flac"      "audio_bach_toccata.wav"

    # MP4 → WAV  (extract audio track only — full uncompressed video would be impractically large)
    convert_sample "video_elephants_dream.mp4"    "video_elephants_dream_audio.wav"  -vn -acodec pcm_s16le
    convert_sample "video_big_buck_bunny.mp4"     "video_big_buck_bunny_audio.wav"   -vn -acodec pcm_s16le
fi
