use std::io::{self, Read, Seek, SeekFrom, Write};

// Block-based LZSS with two improvements over v1:
//
// 1. Cross-block sliding window: the last WINDOW_SIZE bytes of raw output are carried
//    as a history prefix into the next block's LZ search window. This lets the
//    compressor find matches that straddle block boundaries.
//
// 2. Stride-2 delta filter (block type 2): before compressing, each byte is replaced
//    by its difference from the byte two positions earlier — treating even/odd positions
//    as independent streams. For 16-bit PCM audio this collapses high bytes to near-zero
//    and reduces low-byte variance dramatically. Compressor tries both the raw and the
//    delta-filtered path and picks whichever produces the smaller output.
//
// File format:
//   [4B magic "LZC2"] [8B original_size u64 LE]
//   blocks until EOF:
//     [1B type: 0=stored, 1=lzss+history, 2=delta2+lzss]
//     [4B raw_len u32 LE] [4B comp_len u32 LE] [comp_len bytes]
//
// LZSS token format (types 1 and 2):
//   cmd 0x01..=0x80: literal run of `cmd` bytes follows (1..=128 literals)
//   cmd 0x81..=0xFF: back-reference; length = (cmd−0x81)+MIN_MATCH,
//                    next 2 bytes = (offset−1) as u16 LE

const MAGIC: [u8; 4] = *b"LZC2";
const BLOCK_SIZE: usize = 65536;
const WINDOW_SIZE: usize = 65536;
const HASH_BITS: usize = 16;
const HASH_SIZE: usize = 1 << HASH_BITS;
const MAX_MATCH: usize = 130;
const MIN_MATCH: usize = 4;
const MAX_CHAIN: usize = 32;

const BLOCK_STORED: u8 = 0;
const BLOCK_LZSS: u8 = 1;
const BLOCK_DELTA2_LZSS: u8 = 2;

pub fn compress<S: Read + Seek, W: Write + Seek>(mut input: S, mut output: W) -> io::Result<()> {
    let original_size = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;

    output.write_all(&MAGIC)?;
    output.write_all(&original_size.to_le_bytes())?;

    let mut in_buf = vec![0u8; BLOCK_SIZE];
    let mut history: Vec<u8> = Vec::new();

    loop {
        let n = read_full(&mut input, &mut in_buf)?;
        if n == 0 {
            break;
        }
        let block = &in_buf[..n];

        // Option A: LZSS with cross-block history
        let comp_lzss = lzss_compress(&history, block);

        // Option B: stride-2 delta filter + LZSS (self-contained, no cross-block history)
        let delta_buf = delta2_encode(block);
        let comp_delta = lzss_compress(&[], &delta_buf);

        let (flag, data): (u8, &[u8]) =
            if comp_lzss.len() <= comp_delta.len() && comp_lzss.len() < block.len() {
                (BLOCK_LZSS, &comp_lzss)
            } else if comp_delta.len() < block.len() {
                (BLOCK_DELTA2_LZSS, &comp_delta)
            } else {
                (BLOCK_STORED, block)
            };

        output.write_all(&[flag])?;
        output.write_all(&(block.len() as u32).to_le_bytes())?;
        output.write_all(&(data.len() as u32).to_le_bytes())?;
        output.write_all(data)?;

        push_history(&mut history, block);
    }

    Ok(())
}

pub fn decompress<S: Read + Seek, W: Write + Seek>(mut input: S, mut output: W) -> io::Result<()> {
    let mut magic = [0u8; 4];
    input.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid magic"));
    }
    let mut buf8 = [0u8; 8];
    input.read_exact(&mut buf8)?;

    let mut flag_buf = [0u8; 1];
    let mut len_buf = [0u8; 4];
    let mut history: Vec<u8> = Vec::new();

    loop {
        match input.read_exact(&mut flag_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        input.read_exact(&mut len_buf)?;
        let raw_len = u32::from_le_bytes(len_buf) as usize;
        input.read_exact(&mut len_buf)?;
        let comp_len = u32::from_le_bytes(len_buf) as usize;

        let mut data = vec![0u8; comp_len];
        input.read_exact(&mut data)?;

        let block: Vec<u8> = match flag_buf[0] {
            BLOCK_STORED => data,
            BLOCK_LZSS => lzss_decompress(&history, &data, raw_len)?,
            BLOCK_DELTA2_LZSS => {
                let decoded = lzss_decompress(&[], &data, raw_len)?;
                delta2_decode(decoded)
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown block type")),
        };

        push_history(&mut history, &block);
        output.write_all(&block)?;
    }

    Ok(())
}

// Keep the last WINDOW_SIZE raw bytes for cross-block LZ history.
fn push_history(history: &mut Vec<u8>, block: &[u8]) {
    if block.len() >= WINDOW_SIZE {
        history.clear();
        history.extend_from_slice(&block[block.len() - WINDOW_SIZE..]);
    } else {
        let total = history.len() + block.len();
        if total > WINDOW_SIZE {
            history.drain(..total - WINDOW_SIZE);
        }
        history.extend_from_slice(block);
    }
}

// Stride-2 delta: each byte stores its difference from the byte 2 positions earlier.
// Even positions and odd positions are treated as independent byte streams.
// This is highly effective for 16-bit PCM audio where adjacent samples correlate strongly.
fn delta2_encode(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut out = vec![0u8; n];
    if n > 0 {
        out[0] = input[0];
    }
    if n > 1 {
        out[1] = input[1];
    }
    for i in 2..n {
        out[i] = input[i].wrapping_sub(input[i - 2]);
    }
    out
}

fn delta2_decode(mut data: Vec<u8>) -> Vec<u8> {
    for i in 2..data.len() {
        let prev = data[i - 2];
        data[i] = data[i].wrapping_add(prev);
    }
    data
}

fn read_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

#[inline]
fn hash4(b: &[u8], pos: usize) -> usize {
    let v = u32::from_le_bytes([b[pos], b[pos + 1], b[pos + 2], b[pos + 3]]);
    (v.wrapping_mul(0x9E37_79B9) >> (32 - HASH_BITS)) as usize
}

// Compress `input` using LZ77 with a sliding window seeded by `prefix`.
// Back-references can reach into `prefix`, enabling cross-block matches when
// prefix is the tail of the previous raw block.
fn lzss_compress(prefix: &[u8], input: &[u8]) -> Vec<u8> {
    let prefix_len = prefix.len();
    let n = input.len();
    let mut output = Vec::with_capacity(n / 2 + 16);

    if n == 0 {
        return output;
    }
    if n < MIN_MATCH {
        output.push(n as u8);
        output.extend_from_slice(input);
        return output;
    }

    let combined_len = prefix_len + n;
    let mut combined = Vec::with_capacity(combined_len);
    combined.extend_from_slice(prefix);
    combined.extend_from_slice(input);

    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut chain = vec![u32::MAX; combined_len];

    // Seed hash table with prefix positions so the input can match against them.
    for i in 0..prefix_len {
        if i + MIN_MATCH <= combined_len {
            let h = hash4(&combined, i);
            chain[i] = head[h];
            head[h] = i as u32;
        }
    }

    let mut pos = prefix_len;
    let mut lit_start = prefix_len;
    let mut lit_len: usize = 0;

    while pos < combined_len {
        let (match_offset, match_len) = if pos + MIN_MATCH <= combined_len {
            let h = hash4(&combined, pos);

            let mut best_len = 0usize;
            let mut best_offset = 0usize;
            let mut cur = head[h];
            let mut depth = 0;

            while cur != u32::MAX && depth < MAX_CHAIN {
                let cp = cur as usize;
                if pos - cp >= WINDOW_SIZE {
                    break;
                }
                let max_len = (combined_len - pos).min(MAX_MATCH);
                let mut ml = 0;
                while ml < max_len && combined[cp + ml] == combined[pos + ml] {
                    ml += 1;
                }
                if ml > best_len {
                    best_len = ml;
                    best_offset = pos - cp;
                    if best_len == MAX_MATCH {
                        break;
                    }
                }
                cur = chain[cp];
                depth += 1;
            }

            chain[pos] = head[h];
            head[h] = pos as u32;

            if best_len >= MIN_MATCH {
                (best_offset, best_len)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        if match_len >= MIN_MATCH {
            if lit_len > 0 {
                flush_literals(&mut output, &combined, lit_start, lit_len);
                lit_len = 0;
            }
            let cmd = 0x81u8 + (match_len - MIN_MATCH) as u8;
            let off = (match_offset - 1) as u16;
            output.push(cmd);
            output.push(off as u8);
            output.push((off >> 8) as u8);

            for i in 1..match_len {
                let p = pos + i;
                if p + MIN_MATCH <= combined_len {
                    let h = hash4(&combined, p);
                    chain[p] = head[h];
                    head[h] = p as u32;
                }
            }
            pos += match_len;
        } else {
            if lit_len == 0 {
                lit_start = pos;
            }
            lit_len += 1;
            pos += 1;
        }
    }

    if lit_len > 0 {
        flush_literals(&mut output, &combined, lit_start, lit_len);
    }
    output
}

fn flush_literals(output: &mut Vec<u8>, data: &[u8], start: usize, len: usize) {
    let mut remaining = len;
    let mut offset = start;
    while remaining > 0 {
        let chunk = remaining.min(128);
        output.push(chunk as u8);
        output.extend_from_slice(&data[offset..offset + chunk]);
        offset += chunk;
        remaining -= chunk;
    }
}

// Decompress a block. `prefix` (the raw history tail from previous blocks) is
// prepended so that back-references into it resolve correctly.
// Returns only the newly decompressed bytes, not the prefix.
fn lzss_decompress(prefix: &[u8], data: &[u8], expected_size: usize) -> io::Result<Vec<u8>> {
    let prefix_len = prefix.len();
    let mut output = Vec::with_capacity(prefix_len + expected_size);
    output.extend_from_slice(prefix);

    let mut pos = 0;
    while pos < data.len() {
        let cmd = data[pos];
        pos += 1;

        if cmd <= 0x80 {
            let count = cmd as usize;
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "zero literal count"));
            }
            if pos + count > data.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated literal run"));
            }
            output.extend_from_slice(&data[pos..pos + count]);
            pos += count;
        } else {
            if pos + 2 > data.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated backref"));
            }
            let length = (cmd - 0x81) as usize + MIN_MATCH;
            let offset = (data[pos] as usize) | ((data[pos + 1] as usize) << 8);
            let offset = offset + 1;
            pos += 2;

            let out_len = output.len();
            if offset > out_len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "backref offset out of bounds",
                ));
            }
            let start = out_len - offset;
            for i in 0..length {
                let byte = output[start + i];
                output.push(byte);
            }
        }
    }

    let new_len = output.len() - prefix_len;
    if new_len != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("size mismatch: got {} expected {}", new_len, expected_size),
        ));
    }

    Ok(output[prefix_len..].to_vec())
}
