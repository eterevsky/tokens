use std::cmp::max;
use std::{collections::HashMap, fmt};
use serde_json::json;

#[derive(Clone, Debug)]
pub(super) enum CharsToken {
    /// Used as extensions after an Ext token to specify characters for which
    /// there are no dedicated tokens.
    Ext(u8),
    /// A single character token. If followed by a Char or Str token, it
    /// indicates the `ch` character. Otherwise, it is combined with a number
    /// of following Ext tokens to indicate a character in the range from
    /// `from` to the `from` character of the next Char token.
    Char(char),
    /// Tokens indicating a given string.
    Str(String),
}

impl CharsToken {
    pub fn bytes_len(&self) -> usize {
        match self {
            CharsToken::Ext(_) => unreachable!(),
            CharsToken::Char(ch) => ch.len_utf8(),
            CharsToken::Str(s) => s.as_bytes().len(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            CharsToken::Ext(n) => (*n).into(),
            CharsToken::Char(ch) => ch.to_string().into(),
            CharsToken::Str(s) => s.as_str().into(),
        }
    }

    pub fn to_string(&self) -> Option<String> {
        match self {
            CharsToken::Ext(_) => None,
            CharsToken::Char(ch) => Some(ch.to_string()),
            CharsToken::Str(s) => Some(s.clone()),
        }
    }
}

impl fmt::Display for CharsToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharsToken::Ext(idx) => write!(f, "{}", idx),
            CharsToken::Char(ch) => write!(f, "{:?}", *ch),
            CharsToken::Str(s) => write!(f, "{:?}", s),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CharsTokenIdx(u32);

impl CharsTokenIdx {
    pub fn id(self) -> usize {
        self.0 as usize
    }
}

pub const HI_CHAR_THRESHOLD: usize = 256;

#[derive(Clone, Debug)]
pub struct CharsTokenSet {
    pub(super) tokens: Vec<CharsToken>,
    lo_chars_enc: [Vec<CharsTokenIdx>; HI_CHAR_THRESHOLD],
    hi_chars_enc: HashMap<char, Vec<CharsTokenIdx>>,
    tokens_by_str: HashMap<String, CharsTokenIdx>,
}

impl CharsTokenSet {
    pub fn new(num_ext: usize) -> Self {
        let mut tokens = Vec::new();
        for i in 0..num_ext {
            tokens.push(CharsToken::Ext(i as u8));
        }

        CharsTokenSet {
            tokens,
            lo_chars_enc: [(); HI_CHAR_THRESHOLD].map(|_| Vec::new()),
            hi_chars_enc: HashMap::new(),
            tokens_by_str: HashMap::new(),
        }
    }

    pub fn ntokens(&self) -> usize {
        self.tokens.len()
    }

    pub fn add_char_token(&mut self, ch: char) -> CharsTokenIdx {
        let idx = CharsTokenIdx(self.tokens.len() as u32);
        self.tokens.push(CharsToken::Char(ch));
        self.tokens_by_str.insert(ch.to_string(), idx);

        self.add_encoding(ch, vec![idx]);

        idx
    }

    pub fn add_encoding(&mut self, ch: char, enc: Vec<CharsTokenIdx>) {
        if (ch as usize) < HI_CHAR_THRESHOLD {
            self.lo_chars_enc[ch as usize] = enc;
        } else {
            self.hi_chars_enc.insert(ch, enc);
        }
    }

    pub fn add_string(&mut self, string: &str) -> CharsTokenIdx {
        let idx = CharsTokenIdx(self.tokens.len() as u32);
        self.tokens.push(CharsToken::Str(string.to_string()));
        self.tokens_by_str.insert(string.to_string(), idx);

        idx
    }

    pub fn ext_token(idx: u32) -> CharsTokenIdx {
        CharsTokenIdx(idx)
    }

    pub fn char_encoding<'a>(&'a self, ch: char) -> &'a [CharsTokenIdx] {
        // TODO: fix for missing chars
        if (ch as usize) < HI_CHAR_THRESHOLD {
            &self.lo_chars_enc[ch as usize]
        } else {
            self.hi_chars_enc.get(&ch).unwrap()
        }
    }

    pub fn char_cost(&self, ch: char) -> u8 {
        if (ch as usize) < HI_CHAR_THRESHOLD {
            self.lo_chars_enc[ch as usize].len() as u8
        } else {
            match self.hi_chars_enc.get(&ch) {
                Some(enc) => enc.len() as u8,
                None => 32, // TODO: calculate
            }
        }
    }

    pub fn token_by_str(&self, s: &str) -> Option<CharsTokenIdx> {
        self.tokens_by_str.get(s).map(|&idx| idx)
    }

    pub fn max_bytes_in_token(&self) -> usize {
        let mut max_bytes = 4; // Maximum size of a UTF-8 char
        for token in self.tokens.iter() {
            let nbytes = match token {
                CharsToken::Ext(_) => continue,
                CharsToken::Char(ch) => ch.to_string().as_bytes().len(),
                CharsToken::Str(s) => s.as_bytes().len(),
            };
            max_bytes = max(max_bytes, nbytes);
        }
        max_bytes
    }

    pub fn tokens_to_json(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();

        for token in self.tokens.iter() {
            let value = match token {
                CharsToken::Ext(n) => (*n).into(),
                CharsToken::Char(ch) => ch.to_string().into(),
                CharsToken::Str(s) => s.as_str().into(),
            };
            out.push(value);
        }

        out
    }

    pub fn encodings_to_json(&self) -> serde_json::Value {
        let mut out = json!({});

        for (ch, enc) in self.lo_chars_enc.iter().enumerate() {
            if enc.len() <= 1 {
                continue;
            };
            let ch = char::from_u32(ch as u32).unwrap();
            let encoding: Vec<serde_json::Value> = 
                enc.iter()
                    .map(|t| self.tokens[t.id()].to_json())
                    .collect::<Vec<_>>();
            out[ch.to_string().as_str()] = json!(encoding);
        }

        let mut keys: Vec<_> = self.hi_chars_enc.keys().collect();
        keys.sort();

        for ch in keys {
            let enc = self.hi_chars_enc.get(ch).unwrap();
            if enc.len() <= 1 {
                continue;
            };
            let encoding: Vec<serde_json::Value> = 
                enc.iter()
                    .map(|t| self.tokens[t.id()].to_json())
                    .collect::<Vec<_>>();
            out[ch.to_string().as_str()] = json!(encoding);
        }

        out
    }
}

impl fmt::Display for CharsTokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, token) in self.tokens.iter().enumerate() {
            writeln!(f, "{}  {}", i, token)?;
        }

        writeln!(f)?;

        for (ch, enc) in self.lo_chars_enc.iter().enumerate() {
            if !enc.is_empty() {
                let ch = char::from_u32(ch as u32).unwrap();
                write!(f, "{:?} ", ch)?;
                for &CharsTokenIdx(idx) in enc {
                    write!(f, " {}", self.tokens[idx as usize])?;
                }
                writeln!(f)?;
            }
        }

        let mut keys: Vec<_> = self.hi_chars_enc.keys().collect();
        keys.sort();

        for ch in keys {
            let enc = self.hi_chars_enc.get(ch).unwrap();
            write!(f, "{:?} ", ch)?;
            for &CharsTokenIdx(idx) in enc {
                write!(f, " {}", self.tokens[idx as usize])?;
            }
            writeln!(f)?;
        }

        Ok(())
    }
}
