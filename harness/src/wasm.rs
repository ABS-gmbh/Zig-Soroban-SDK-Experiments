//! Wasm section sizes.

pub struct Sections {
    pub code: usize,
    pub data: usize,
    /// The `contractspecv0` custom section: the contract's interface.
    pub spec: usize,
    /// Everything else: headers, types, imports, exports, other metadata.
    pub other: usize,
}

pub fn sections(wasm: &[u8]) -> Sections {
    let mut s = Sections { code: 0, data: 0, spec: 0, other: 8 };
    let mut i = 8;
    while i < wasm.len() {
        let id = wasm[i];
        let (size, body) = leb128(wasm, i + 1);
        let total = body - i + size;
        match id {
            10 => s.code += total,
            11 => s.data += total,
            0 => {
                let (name_len, name_start) = leb128(wasm, body);
                if &wasm[name_start..name_start + name_len] == b"contractspecv0" {
                    s.spec += total;
                } else {
                    s.other += total;
                }
            }
            _ => s.other += total,
        }
        i = body + size;
    }
    s
}

fn leb128(b: &[u8], mut i: usize) -> (usize, usize) {
    let (mut v, mut shift) = (0usize, 0);
    loop {
        let x = b[i];
        i += 1;
        v |= ((x & 0x7f) as usize) << shift;
        shift += 7;
        if x < 0x80 {
            return (v, i);
        }
    }
}
