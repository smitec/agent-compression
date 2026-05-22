use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::io::{self, Read, Seek, SeekFrom, Write};

const MAGIC: [u8; 4] = *b"LZC3";
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
const BLOCK_LZSS_HUF: u8 = 3;
const BLOCK_DELTA2_LZSS_HUF: u8 = 4;
const BLOCK_DELTA1_LZSS: u8 = 5;
const BLOCK_DELTA1_LZSS_HUF: u8 = 6;
const BLOCK_LZSS_HUF3: u8 = 7;
const BLOCK_DELTA1_LZSS_HUF3: u8 = 8;
const BLOCK_DELTA2_LZSS_HUF3: u8 = 9;
const BLOCK_DELTA3_LZSS: u8 = 10;
const BLOCK_DELTA3_LZSS_HUF: u8 = 11;
const BLOCK_DELTA3_LZSS_HUF3: u8 = 12;
const BLOCK_DELTA4_LZSS: u8 = 13;
const BLOCK_DELTA4_LZSS_HUF: u8 = 14;
const BLOCK_DELTA4_LZSS_HUF3: u8 = 15;

// ── BitWriter ────────────────────────────────────────────────────────────────

struct BitWriter {
    buf: Vec<u8>,
    accumulator: u64,
    bits_in_acc: u8,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter { buf: Vec::new(), accumulator: 0, bits_in_acc: 0 }
    }

    fn write_bits(&mut self, value: u32, count: u8) {
        debug_assert!(count <= 15 && count > 0);
        let masked = (value as u64) & ((1u64 << count) - 1);
        self.accumulator |= masked << (64 - self.bits_in_acc - count);
        self.bits_in_acc += count;
        while self.bits_in_acc >= 8 {
            self.buf.push((self.accumulator >> 56) as u8);
            self.accumulator <<= 8;
            self.bits_in_acc -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits_in_acc > 0 {
            self.buf.push((self.accumulator >> 56) as u8);
        }
        self.buf
    }
}

// ── BitReader ────────────────────────────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    accumulator: u64,
    bits_in_acc: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, byte_pos: 0, accumulator: 0, bits_in_acc: 0 }
    }

    fn fill(&mut self) {
        while self.bits_in_acc <= 56 && self.byte_pos < self.data.len() {
            self.accumulator |= (self.data[self.byte_pos] as u64) << (56 - self.bits_in_acc);
            self.byte_pos += 1;
            self.bits_in_acc += 8;
        }
    }

    fn peek_bits(&mut self, n: u8) -> u32 {
        self.fill();
        (self.accumulator >> (64 - n)) as u32
    }

    fn consume_bits(&mut self, n: u8) {
        self.accumulator <<= n;
        self.bits_in_acc = self.bits_in_acc.saturating_sub(n);
    }
}

// ── Huffman ──────────────────────────────────────────────────────────────────

fn compute_huffman_lengths(freq: &[u32; 256]) -> [u8; 256] {
    let mut lengths = [0u8; 256];

    let active: Vec<usize> = (0..256).filter(|&i| freq[i] > 0).collect();
    match active.len() {
        0 => return lengths,
        1 => { lengths[active[0]] = 1; return lengths; }
        _ => {}
    }

    struct Node { freq: u32, left: usize, right: usize, symbol: u8, is_leaf: bool }

    let mut nodes: Vec<Node> = active.iter().map(|&s| Node {
        freq: freq[s], left: usize::MAX, right: usize::MAX, symbol: s as u8, is_leaf: true,
    }).collect();

    let mut heap: BinaryHeap<Reverse<(u32, usize)>> =
        (0..nodes.len()).map(|i| Reverse((nodes[i].freq, i))).collect();

    while heap.len() > 1 {
        let Reverse((f1, i1)) = heap.pop().unwrap();
        let Reverse((f2, i2)) = heap.pop().unwrap();
        let new_idx = nodes.len();
        nodes.push(Node { freq: f1 + f2, left: i1, right: i2, symbol: 0, is_leaf: false });
        heap.push(Reverse((f1 + f2, new_idx)));
    }

    let root = heap.pop().unwrap().0.1;
    let mut stack: Vec<(usize, u8)> = vec![(root, 0)];
    while let Some((idx, depth)) = stack.pop() {
        let node = &nodes[idx];
        if node.is_leaf {
            lengths[node.symbol as usize] = depth.min(15);
        } else {
            if node.left != usize::MAX  { stack.push((node.left,  depth + 1)); }
            if node.right != usize::MAX { stack.push((node.right, depth + 1)); }
        }
    }

    // Verify Kraft sum; fall back to uniform lengths if clamping violated it.
    let kraft: u32 = active.iter()
        .map(|&s| 1u32 << (15u32.saturating_sub(lengths[s] as u32)))
        .sum();
    if kraft > 32768 {
        let bits = (active.len() as f32).log2().ceil() as u8;
        let uniform = bits.max(1);
        for &s in &active { lengths[s] = uniform; }
    }

    lengths
}

fn canonical_codes_from_lengths(lengths: &[u8; 256]) -> [u32; 256] {
    let mut codes = [0u32; 256];
    let mut sorted: Vec<(u8, usize)> = (0..256)
        .filter(|&i| lengths[i] > 0)
        .map(|i| (lengths[i], i))
        .collect();
    sorted.sort_unstable();

    let mut code = 0u32;
    let mut prev_len = 0u8;
    for (len, sym) in sorted {
        code <<= len - prev_len;
        codes[sym] = code;
        code += 1;
        prev_len = len;
    }
    codes
}

fn build_decode_table(lengths: &[u8; 256]) -> Vec<(u8, u8)> {
    let mut table = vec![(0u8, 0u8); 32768];
    let codes = canonical_codes_from_lengths(lengths);
    for s in 0..256usize {
        let len = lengths[s];
        if len == 0 { continue; }
        let base = (codes[s] << (15 - len)) as usize;
        let step = 1usize << (15 - len);
        for i in 0..step {
            table[base + i] = (s as u8, len);
        }
    }
    table
}

// Layout: [4B lzss_len u32 LE][256B code_lengths][bit-packed stream]
fn huffman_encode(lzss_data: &[u8]) -> Vec<u8> {
    if lzss_data.is_empty() { return Vec::new(); }

    let mut freq = [0u32; 256];
    for &b in lzss_data { freq[b as usize] += 1; }

    let lengths = compute_huffman_lengths(&freq);
    let codes = canonical_codes_from_lengths(&lengths);

    let mut out = Vec::with_capacity(lzss_data.len() + 260);
    out.extend_from_slice(&(lzss_data.len() as u32).to_le_bytes());
    out.extend_from_slice(&lengths);

    let mut bw = BitWriter::new();
    for &b in lzss_data {
        bw.write_bits(codes[b as usize], lengths[b as usize]);
    }
    out.extend(bw.finish());
    out
}

fn huffman_decode(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < 260 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "huffman block too short"));
    }
    let lzss_len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let lengths: [u8; 256] = data[4..260].try_into().unwrap();

    let active_count = lengths.iter().filter(|&&l| l > 0).count();
    if active_count == 0 {
        if lzss_len == 0 { return Ok(Vec::new()); }
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty huffman table"));
    }

    // Single-symbol edge case: all output bytes are that one symbol.
    if active_count == 1 {
        let sym = lengths.iter().position(|&l| l > 0).unwrap() as u8;
        return Ok(vec![sym; lzss_len]);
    }

    let table = build_decode_table(&lengths);
    let mut br = BitReader::new(&data[260..]);
    let mut out = Vec::with_capacity(lzss_len);

    while out.len() < lzss_len {
        let idx = br.peek_bits(15) as usize;
        let (sym, consumed) = table[idx];
        if consumed == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "huffman decode error"));
        }
        out.push(sym);
        br.consume_bits(consumed);
    }
    Ok(out)
}

// ── Delta filters ────────────────────────────────────────────────────────────

fn delta_n_encode(input: &[u8], stride: usize) -> Vec<u8> {
    let n = input.len();
    let mut out = vec![0u8; n];
    for i in 0..stride.min(n) { out[i] = input[i]; }
    for i in stride..n { out[i] = input[i].wrapping_sub(input[i - stride]); }
    out
}

fn delta_n_decode(mut data: Vec<u8>, stride: usize) -> Vec<u8> {
    for i in stride..data.len() {
        let prev = data[i - stride];
        data[i] = data[i].wrapping_add(prev);
    }
    data
}

fn delta1_decode(data: Vec<u8>) -> Vec<u8> { delta_n_decode(data, 1) }
fn delta2_decode(data: Vec<u8>) -> Vec<u8> { delta_n_decode(data, 2) }

fn estimate_entropy(data: &[u8]) -> f32 {
    let mut freq = [0u32; 256];
    for &b in data { freq[b as usize] += 1; }
    let n = data.len() as f32;
    let mut h = 0.0f32;
    for &f in &freq {
        if f > 0 {
            let p = f as f32 / n;
            h -= p * p.log2();
        }
    }
    h
}

// ── Utility ──────────────────────────────────────────────────────────────────

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

fn push_history(history: &mut Vec<u8>, block: &[u8]) {
    if block.len() >= WINDOW_SIZE {
        history.clear();
        history.extend_from_slice(&block[block.len() - WINDOW_SIZE..]);
    } else {
        let total = history.len() + block.len();
        if total > WINDOW_SIZE { history.drain(..total - WINDOW_SIZE); }
        history.extend_from_slice(block);
    }
}

// ── LZSS ─────────────────────────────────────────────────────────────────────

// Insert `pos` into hash table, then find the best match. Returns (offset, length).
fn find_best_match_at(
    combined: &[u8],
    pos: usize,
    combined_len: usize,
    head: &mut Vec<u32>,
    chain: &mut Vec<u32>,
) -> (usize, usize) {
    if pos + MIN_MATCH > combined_len { return (0, 0); }
    let h = hash4(combined, pos);
    let old_head = head[h];
    chain[pos] = old_head;
    head[h] = pos as u32;

    let mut best_len = 0usize;
    let mut best_off = 0usize;
    let mut cur = old_head;
    let mut depth = 0;

    while cur != u32::MAX && depth < MAX_CHAIN {
        let cp = cur as usize;
        if pos - cp >= WINDOW_SIZE { break; }
        let max_len = (combined_len - pos).min(MAX_MATCH);
        let mut ml = 0;
        while ml < max_len && combined[cp + ml] == combined[pos + ml] { ml += 1; }
        if ml > best_len {
            best_len = ml;
            best_off = pos - cp;
            if best_len == MAX_MATCH { break; }
        }
        cur = chain[cp];
        depth += 1;
    }

    if best_len >= MIN_MATCH { (best_off, best_len) } else { (0, 0) }
}

fn lzss_compress(prefix: &[u8], input: &[u8]) -> Vec<u8> {
    let prefix_len = prefix.len();
    let n = input.len();
    let mut output = Vec::with_capacity(n / 2 + 16);

    if n == 0 { return output; }
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
        let (mo, ml) = find_best_match_at(&combined, pos, combined_len, &mut head, &mut chain);

        if ml >= MIN_MATCH {
            // Lazy matching: peek up to 2 positions ahead for a better match.
            if ml < MAX_MATCH && pos + 1 + MIN_MATCH <= combined_len {
                let (lo, ll) = find_best_match_at(&combined, pos + 1, combined_len, &mut head, &mut chain);
                if ll > ml {
                    // pos+1 has a better match; check pos+2 for an even better one.
                    let mut use_off = lo;
                    let mut use_len = ll;
                    let mut skip = 1usize; // literals to emit before the match

                    if ll < MAX_MATCH && pos + 2 + MIN_MATCH <= combined_len {
                        let (lo2, ll2) = find_best_match_at(&combined, pos + 2, combined_len, &mut head, &mut chain);
                        if ll2 > ll {
                            use_off = lo2;
                            use_len = ll2;
                            skip = 2;
                        }
                        // pos+2 now inserted in the hash table regardless.
                    }

                    if lit_len == 0 { lit_start = pos; }
                    lit_len += skip;
                    pos += skip;

                    flush_literals(&mut output, &combined, lit_start, lit_len);
                    lit_len = 0;

                    let cmd = 0x81u8 + (use_len - MIN_MATCH) as u8;
                    let off = (use_off - 1) as u16;
                    output.push(cmd);
                    output.push(off as u8);
                    output.push((off >> 8) as u8);

                    for i in 1..use_len {
                        let p = pos + i;
                        if p + MIN_MATCH <= combined_len {
                            let h = hash4(&combined, p);
                            chain[p] = head[h];
                            head[h] = p as u32;
                        }
                    }
                    pos += use_len;
                    continue;
                }
                // pos+1 already inserted by the lazy probe; continue with original match.
            }

            if lit_len > 0 {
                flush_literals(&mut output, &combined, lit_start, lit_len);
                lit_len = 0;
            }
            let cmd = 0x81u8 + (ml - MIN_MATCH) as u8;
            let off = (mo - 1) as u16;
            output.push(cmd);
            output.push(off as u8);
            output.push((off >> 8) as u8);

            for i in 1..ml {
                let p = pos + i;
                if p + MIN_MATCH <= combined_len {
                    let h = hash4(&combined, p);
                    chain[p] = head[h];
                    head[h] = p as u32;
                }
            }
            pos += ml;
        } else {
            if lit_len == 0 { lit_start = pos; }
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
            let offset = ((data[pos] as usize) | ((data[pos + 1] as usize) << 8)) + 1;
            pos += 2;

            let out_len = output.len();
            if offset > out_len {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "backref offset out of bounds"));
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

// ── Delta candidate selection ─────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum DeltaMode { None, Delta1, Delta2, Delta3, Delta4 }

fn select_delta(block: &[u8]) -> (DeltaMode, Vec<u8>) {
    let e_raw = estimate_entropy(block);
    let threshold = e_raw - 0.05;

    let candidates: [(DeltaMode, Vec<u8>); 4] = [
        (DeltaMode::Delta1, delta_n_encode(block, 1)),
        (DeltaMode::Delta2, delta_n_encode(block, 2)),
        (DeltaMode::Delta3, delta_n_encode(block, 3)),
        (DeltaMode::Delta4, delta_n_encode(block, 4)),
    ];

    let mut best_entropy = threshold;
    let mut best_idx: Option<usize> = None;

    for i in 0..candidates.len() {
        let e = estimate_entropy(&candidates[i].1);
        if e < best_entropy {
            best_entropy = e;
            best_idx = Some(i);
        }
    }

    match best_idx {
        Some(i) => {
            let mut arr = candidates.into_iter();
            arr.nth(i).unwrap()
        }
        None => (DeltaMode::None, Vec::new()),
    }
}

// ── 3-Stream Huffman ──────────────────────────────────────────────────────────

// Splits the LZSS byte stream into cmd/literal/offset streams and Huffman-codes each.
// Wire format: [4B cmds_huf_len][4B lits_huf_len][cmds_huf][lits_huf][offs_huf]
fn lzss_huf3_encode(lzss_data: &[u8]) -> Vec<u8> {
    if lzss_data.is_empty() { return Vec::new(); }

    let mut cmds = Vec::new();
    let mut lits = Vec::new();
    let mut offs = Vec::new();

    let mut pos = 0;
    while pos < lzss_data.len() {
        let cmd = lzss_data[pos];
        pos += 1;
        cmds.push(cmd);
        if cmd <= 0x80 {
            let count = cmd as usize;
            lits.extend_from_slice(&lzss_data[pos..pos + count]);
            pos += count;
        } else {
            offs.push(lzss_data[pos]);
            offs.push(lzss_data[pos + 1]);
            pos += 2;
        }
    }

    let cmds_huf = huffman_encode(&cmds);
    let lits_huf = if lits.is_empty() { Vec::new() } else { huffman_encode(&lits) };
    let offs_huf = if offs.is_empty() { Vec::new() } else { huffman_encode(&offs) };

    let mut out = Vec::with_capacity(8 + cmds_huf.len() + lits_huf.len() + offs_huf.len());
    out.extend_from_slice(&(cmds_huf.len() as u32).to_le_bytes());
    out.extend_from_slice(&(lits_huf.len() as u32).to_le_bytes());
    out.extend(cmds_huf);
    out.extend(lits_huf);
    out.extend(offs_huf);
    out
}

fn lzss_huf3_decode(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.is_empty() { return Ok(Vec::new()); }
    if data.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "huf3 block too short"));
    }

    let cmds_huf_len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let lits_huf_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;

    if 8 + cmds_huf_len + lits_huf_len > data.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "huf3 section lengths overflow"));
    }

    let cmds_section = &data[8..8 + cmds_huf_len];
    let lits_section = &data[8 + cmds_huf_len..8 + cmds_huf_len + lits_huf_len];
    let offs_section = &data[8 + cmds_huf_len + lits_huf_len..];

    let cmds = huffman_decode(cmds_section)?;
    let lits = if lits_huf_len == 0 { Vec::new() } else { huffman_decode(lits_section)? };
    let offs = if offs_section.is_empty() { Vec::new() } else { huffman_decode(offs_section)? };

    let mut out = Vec::new();
    let mut lits_pos = 0usize;
    let mut offs_pos = 0usize;

    for &cmd in &cmds {
        out.push(cmd);
        if cmd <= 0x80 {
            let count = cmd as usize;
            if lits_pos + count > lits.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "huf3 lits truncated"));
            }
            out.extend_from_slice(&lits[lits_pos..lits_pos + count]);
            lits_pos += count;
        } else {
            if offs_pos + 2 > offs.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "huf3 offs truncated"));
            }
            out.push(offs[offs_pos]);
            out.push(offs[offs_pos + 1]);
            offs_pos += 2;
        }
    }

    Ok(out)
}

// ── Public API ───────────────────────────────────────────────────────────────

pub fn compress<S: Read + Seek, W: Write + Seek>(mut input: S, mut output: W) -> io::Result<()> {
    let original_size = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;

    output.write_all(&MAGIC)?;
    output.write_all(&original_size.to_le_bytes())?;

    let mut in_buf = vec![0u8; BLOCK_SIZE];
    let mut history: Vec<u8> = Vec::new();

    loop {
        let n = read_full(&mut input, &mut in_buf)?;
        if n == 0 { break; }
        let block = &in_buf[..n];

        // Candidate A: LZSS with cross-block history (± single-tree Huffman ± 3-stream Huffman).
        let comp_lzss = lzss_compress(&history, block);
        let comp_lzss_huf = huffman_encode(&comp_lzss);
        let comp_lzss_huf3 = lzss_huf3_encode(&comp_lzss);

        // Candidate B: best delta + LZSS (self-contained, ± Huffman variants).
        let (delta_mode, delta_buf) = select_delta(block);
        let (comp_delta, comp_delta_huf, comp_delta_huf3) = if !matches!(delta_mode, DeltaMode::None) {
            let cd = lzss_compress(&[], &delta_buf);
            let cdh = huffman_encode(&cd);
            let cdh3 = lzss_huf3_encode(&cd);
            (cd, cdh, cdh3)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        // Pick the smallest.
        let mut best_len = block.len();
        let mut best_type = BLOCK_STORED;

        macro_rules! consider {
            ($len:expr, $flag:expr) => {
                if $len < best_len { best_len = $len; best_type = $flag; }
            };
        }

        consider!(comp_lzss.len(), BLOCK_LZSS);
        if !comp_lzss_huf.is_empty()  { consider!(comp_lzss_huf.len(),  BLOCK_LZSS_HUF); }
        if !comp_lzss_huf3.is_empty() { consider!(comp_lzss_huf3.len(), BLOCK_LZSS_HUF3); }
        if !comp_delta.is_empty() {
            let (df, dhf, dhf3) = match delta_mode {
                DeltaMode::Delta1 => (BLOCK_DELTA1_LZSS, BLOCK_DELTA1_LZSS_HUF, BLOCK_DELTA1_LZSS_HUF3),
                DeltaMode::Delta2 => (BLOCK_DELTA2_LZSS, BLOCK_DELTA2_LZSS_HUF, BLOCK_DELTA2_LZSS_HUF3),
                DeltaMode::Delta3 => (BLOCK_DELTA3_LZSS, BLOCK_DELTA3_LZSS_HUF, BLOCK_DELTA3_LZSS_HUF3),
                DeltaMode::Delta4 => (BLOCK_DELTA4_LZSS, BLOCK_DELTA4_LZSS_HUF, BLOCK_DELTA4_LZSS_HUF3),
                DeltaMode::None   => unreachable!(),
            };
            consider!(comp_delta.len(), df);
            if !comp_delta_huf.is_empty()  { consider!(comp_delta_huf.len(),  dhf); }
            if !comp_delta_huf3.is_empty() { consider!(comp_delta_huf3.len(), dhf3); }
        }

        let data: &[u8] = match best_type {
            BLOCK_STORED            => block,
            BLOCK_LZSS              => &comp_lzss,
            BLOCK_LZSS_HUF          => &comp_lzss_huf,
            BLOCK_LZSS_HUF3         => &comp_lzss_huf3,
            BLOCK_DELTA1_LZSS       => &comp_delta,
            BLOCK_DELTA2_LZSS       => &comp_delta,
            BLOCK_DELTA3_LZSS       => &comp_delta,
            BLOCK_DELTA4_LZSS       => &comp_delta,
            BLOCK_DELTA1_LZSS_HUF   => &comp_delta_huf,
            BLOCK_DELTA2_LZSS_HUF   => &comp_delta_huf,
            BLOCK_DELTA3_LZSS_HUF   => &comp_delta_huf,
            BLOCK_DELTA4_LZSS_HUF   => &comp_delta_huf,
            BLOCK_DELTA1_LZSS_HUF3  => &comp_delta_huf3,
            BLOCK_DELTA2_LZSS_HUF3  => &comp_delta_huf3,
            BLOCK_DELTA3_LZSS_HUF3  => &comp_delta_huf3,
            BLOCK_DELTA4_LZSS_HUF3  => &comp_delta_huf3,
            _                       => block,
        };

        output.write_all(&[best_type])?;
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
            BLOCK_LZSS_HUF => {
                let lzss_bytes = huffman_decode(&data)?;
                lzss_decompress(&history, &lzss_bytes, raw_len)?
            }
            BLOCK_DELTA2_LZSS => {
                let decoded = lzss_decompress(&[], &data, raw_len)?;
                delta2_decode(decoded)
            }
            BLOCK_DELTA2_LZSS_HUF => {
                let lzss_bytes = huffman_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta2_decode(decoded)
            }
            BLOCK_DELTA1_LZSS => {
                let decoded = lzss_decompress(&[], &data, raw_len)?;
                delta1_decode(decoded)
            }
            BLOCK_DELTA1_LZSS_HUF => {
                let lzss_bytes = huffman_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta1_decode(decoded)
            }
            BLOCK_LZSS_HUF3 => {
                let lzss_bytes = lzss_huf3_decode(&data)?;
                lzss_decompress(&history, &lzss_bytes, raw_len)?
            }
            BLOCK_DELTA1_LZSS_HUF3 => {
                let lzss_bytes = lzss_huf3_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 1)
            }
            BLOCK_DELTA2_LZSS_HUF3 => {
                let lzss_bytes = lzss_huf3_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 2)
            }
            BLOCK_DELTA3_LZSS => {
                let decoded = lzss_decompress(&[], &data, raw_len)?;
                delta_n_decode(decoded, 3)
            }
            BLOCK_DELTA3_LZSS_HUF => {
                let lzss_bytes = huffman_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 3)
            }
            BLOCK_DELTA3_LZSS_HUF3 => {
                let lzss_bytes = lzss_huf3_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 3)
            }
            BLOCK_DELTA4_LZSS => {
                let decoded = lzss_decompress(&[], &data, raw_len)?;
                delta_n_decode(decoded, 4)
            }
            BLOCK_DELTA4_LZSS_HUF => {
                let lzss_bytes = huffman_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 4)
            }
            BLOCK_DELTA4_LZSS_HUF3 => {
                let lzss_bytes = lzss_huf3_decode(&data)?;
                let decoded = lzss_decompress(&[], &lzss_bytes, raw_len)?;
                delta_n_decode(decoded, 4)
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown block type")),
        };

        push_history(&mut history, &block);
        output.write_all(&block)?;
    }

    Ok(())
}
