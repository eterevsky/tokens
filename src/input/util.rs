fn is_char_start(b: u8) -> bool {
    b < 128 || b >= 192
}

pub fn extract_valid_utf8_slice<'a>(data: &'a [u8]) -> &'a [u8] {
    let mut start = 0;
    while start < data.len() && !is_char_start(data[start]) {
        start += 1;
    }
    let mut end = data.len();
    if data[end - 1] >= 128 {
        end -= 1;
        while end > start && !is_char_start(data[end]) {
            end -= 1;
        }
    }
    &data[start..end]
}

pub fn find_paragraph_end(data: &[u8], end: usize) -> usize {
    let mut pos = end;
    while pos >= 2 && (data[pos - 1] != 10 || data[pos - 2] != 10) {
        pos -= 1;
    }

    if pos < 2 { end } else { pos }
}
