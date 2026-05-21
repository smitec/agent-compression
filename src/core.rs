use std::io::{self, Read, Seek, SeekFrom, Write};

// Block-based LZSS compressor.
//
// File format:
//   [4B magic] [8B original_size: u64 LE]
//   then blocks until EOF:
//     [1B flag: 0=stored, 1=lzss] [4B raw_len: u32 LE] [4B comp_len: u32 LE] [comp_len bytes]
//
// LZSS token format inside a compressed block:
//   cmd <= 0x80: literal run of `cmd` bytes (1..=128) follows
//   cmd >= 0x81: back-reference; length = (cmd - 0x81) + MIN_MATCH,
//                next 2 bytes = (offset - 1) as u16 LE

const MAGIC: [u8; 4] = *b"LZC1";
const BLOCK_SIZE: usize = 65536;
const WINDOW_SIZE: usize = 65536;
const HASH_BITS: usize = 16;
const HASH_SIZE: usize = 1 << HASH_BITS;
const MAX_MATCH: usize = 130; // 0x81 + 126 = 0xFF, so max (cmd - 0x81) = 126, length = 130
const MIN_MATCH: usize = 4;
const MAX_CHAIN: usize = 32;

pub fn compress<S: Read + Seek, W: Write + Seek>(mut input: S, mut output: W) -> io::Result<()> {
    let original_size = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;

    output.write_all(&MAGIC)?;
    output.write_all(&original_size.to_le_bytes())?;

    let mut in_buf = vec![0u8; BLOCK_SIZE];
    loop {
        let n = read_full(&mut input, &mut in_buf)?;
        if n == 0 {
            break;
        }
        let block = &in_buf[..n];
        let compressed = lzss_compress(block);

        let (flag, data): (u8, &[u8]) = if compressed.len() < block.len() {
            (1, &compressed)
        } else {
            (0, block)
        };

        output.write_all(&[flag])?;
        output.write_all(&(block.len() as u32).to_le_bytes())?;
        output.write_all(&(data.len() as u32).to_le_bytes())?;
        output.write_all(data)?;
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
    // original_size is available here if needed for pre-allocation

    let mut flag_buf = [0u8; 1];
    let mut len_buf = [0u8; 4];

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

        if flag_buf[0] == 0 {
            output.write_all(&data)?;
        } else {
            let decompressed = lzss_decompress(&data, raw_len)?;
            output.write_all(&decompressed)?;
        }
    }

    Ok(())
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

fn lzss_compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut output = Vec::with_capacity(n / 2 + 16);

    if n < MIN_MATCH {
        if n > 0 {
            output.push(n as u8);
            output.extend_from_slice(input);
        }
        return output;
    }

    // head[h] = most recent position with hash h; prev[p] = previous position with same hash
    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut prev = vec![u32::MAX; n];

    let mut pos = 0;
    let mut lit_start = 0;
    let mut lit_len: usize = 0;

    while pos < n {
        let (match_offset, match_len) = if pos + MIN_MATCH <= n {
            let h = hash4(input, pos);

            let mut best_len = 0usize;
            let mut best_offset = 0usize;
            let mut chain_pos = head[h];
            let mut depth = 0;

            while chain_pos != u32::MAX && depth < MAX_CHAIN {
                let cp = chain_pos as usize;
                if pos - cp >= WINDOW_SIZE {
                    break;
                }
                let max_len = (n - pos).min(MAX_MATCH);
                let mut ml = 0;
                while ml < max_len && input[cp + ml] == input[pos + ml] {
                    ml += 1;
                }
                if ml > best_len {
                    best_len = ml;
                    best_offset = pos - cp;
                    if best_len == MAX_MATCH {
                        break;
                    }
                }
                chain_pos = prev[cp];
                depth += 1;
            }

            prev[pos] = head[h];
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
            flush_literals(&mut output, input, lit_start, lit_len);
            lit_len = 0;

            // cmd byte 0x81..=0xFF encodes lengths MIN_MATCH..=MAX_MATCH
            let cmd = 0x81u8 + (match_len - MIN_MATCH) as u8;
            let off = (match_offset - 1) as u16;
            output.push(cmd);
            output.push(off as u8);
            output.push((off >> 8) as u8);

            // Insert skipped positions into hash so they can anchor future matches
            for i in 1..match_len {
                let p = pos + i;
                if p + MIN_MATCH <= n {
                    let h = hash4(input, p);
                    prev[p] = head[h];
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

    flush_literals(&mut output, input, lit_start, lit_len);
    output
}

fn flush_literals(output: &mut Vec<u8>, input: &[u8], start: usize, len: usize) {
    let mut remaining = len;
    let mut offset = start;
    while remaining > 0 {
        let chunk = remaining.min(128);
        output.push(chunk as u8);
        output.extend_from_slice(&input[offset..offset + chunk]);
        offset += chunk;
        remaining -= chunk;
    }
}

fn lzss_decompress(input: &[u8], expected_size: usize) -> io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(expected_size);
    let mut pos = 0;

    while pos < input.len() {
        let cmd = input[pos];
        pos += 1;

        if cmd <= 0x80 {
            let count = cmd as usize;
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "zero literal count"));
            }
            if pos + count > input.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated literal run"));
            }
            output.extend_from_slice(&input[pos..pos + count]);
            pos += count;
        } else {
            if pos + 2 > input.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated backref"));
            }
            let length = (cmd - 0x81) as usize + MIN_MATCH;
            let offset = (input[pos] as usize) | ((input[pos + 1] as usize) << 8);
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

    if output.len() != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("size mismatch: got {} expected {}", output.len(), expected_size),
        ));
    }

    Ok(output)
}
