use clap::ValueEnum;
use std::collections::HashMap;
use std::fmt;

use crate::stats::TokenStats;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum LiteralEncoding {
    /// A literal is encoded as 8 single bit tokens.
    Bits1,
    /// A literal is encoded as 4 2-bit tokens.
    Bits2,
    /// A literal is encoded as 2 4-bit tokens.
    Bits4,
    /// All bytes are tokens, so there's no need to encode anything.
    All,
    /// A single token stands for an unknown byte, and its cost is 2 normal tokens.
    Dist2,
    /// A single token stands for an unknown byte, and its cost is 4 normal tokens.
    Dist4,
    /// A single token stands for an unknown byte, and its cost is 8 normal tokens.
    Dist8,
    /// An unknown byte is encoded as '\x10' and two hexadecimal digits.
    Hex,
}

impl fmt::Display for LiteralEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match *self {
                LiteralEncoding::All => "all",
                LiteralEncoding::Bits1 => "bits1",
                LiteralEncoding::Bits2 => "bits2",
                LiteralEncoding::Bits4 => "bits4",
                LiteralEncoding::Dist2 => "dist2",
                LiteralEncoding::Dist4 => "dist4",
                LiteralEncoding::Dist8 => "dist8",
                LiteralEncoding::Hex => "hex",
            }
        )
    }
}

impl LiteralEncoding {
    fn literal_cost(self) -> u64 {
        match self {
            LiteralEncoding::All => 256, // Shouldn't be used
            LiteralEncoding::Bits1 => 8,
            LiteralEncoding::Bits2 => 4,
            LiteralEncoding::Bits4 => 2,
            LiteralEncoding::Dist2 => 2,
            LiteralEncoding::Dist4 => 4,
            LiteralEncoding::Dist8 => 8,
            LiteralEncoding::Hex => 3,
        }
    }

    pub fn reserved_tokens(self) -> usize {
        match self {
            LiteralEncoding::All => 0,
            LiteralEncoding::Bits1 => 2,
            LiteralEncoding::Bits2 => 4,
            LiteralEncoding::Bits4 => 16,
            LiteralEncoding::Dist2 => 1,
            LiteralEncoding::Dist4 => 1,
            LiteralEncoding::Dist8 => 1,
            LiteralEncoding::Hex => 0,
        }
    }

    pub fn tokens_in_literal(self) -> u64 {
        match self {
            LiteralEncoding::All => 1,
            LiteralEncoding::Bits1 => 8,
            LiteralEncoding::Bits2 => 4,
            LiteralEncoding::Bits4 => 2,
            LiteralEncoding::Dist2 => 1,
            LiteralEncoding::Dist4 => 1,
            LiteralEncoding::Dist8 => 1,
            LiteralEncoding::Hex => 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TokenIdx {
    Token(u32),
    Literal(u8),
    None,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub string: Vec<u8>,
    pub is_mandatory: bool,
    // The longest other token or literal which is a suffix of this one.
    pub suffix: TokenIdx,
}

impl Token {
    fn new(string: &[u8], is_mandatory: bool) -> Self {
        Token {
            string: string.to_vec(),
            is_mandatory,
            suffix: TokenIdx::None,
        }
    }
}

#[derive(Clone)]
pub struct TokenSet {
    pub tokens: Vec<Token>,
    pub tokens_by_string: HashMap<Vec<u8>, u32>,
    pub literal_encoding: LiteralEncoding,

    // Number of tokens that are not added to the token set since they don't
    // have a string representation.
    // reserved_tokens: usize,

    // Number of each literal in the latest tokenization. Smoothed by +1 for
    // all non-token literals.
    // literal_count: [u64; 256],
}

impl TokenSet {
    pub fn new(literal_encoding: LiteralEncoding) -> Self {
        let mut token_set = TokenSet {
            tokens: Vec::new(),
            tokens_by_string: HashMap::new(),
            literal_encoding,
            // literal_count: [0; 256],
        };

        match literal_encoding {
            LiteralEncoding::Hex => {
                token_set.add_mandatory_token(&[0x10]);
                for i in ('0' as u8)..=('9' as u8) {
                    token_set.add_mandatory_token(&[i]);
                }
                for i in ('a' as u8)..=('f' as u8) {
                    token_set.add_mandatory_token(&[i]);
                }
            }
            LiteralEncoding::All => {
                for i in 0..=255 {
                    token_set.add_mandatory_token(&[i])
                }
            }
            _ => (),
        }

        token_set
    }

    pub fn literal_cost(&self) -> u64 {
        self.literal_encoding.literal_cost()
    }

    pub fn ntokens(&self) -> usize {
        self.tokens.len() + self.literal_encoding.reserved_tokens()
    }

    pub fn has_dist_fallback(&self) -> bool {
        match self.literal_encoding {
            LiteralEncoding::Dist2 | LiteralEncoding::Dist4 | LiteralEncoding::Dist8 => true,
            _ => false,
        }
    }

    pub fn dist_entropy(&self, stats: &TokenStats) -> f64 {
        if !self.has_dist_fallback() {
            return 0.0;
        }
        let total_literals: u64 = stats.literal_count.iter().sum();
        if total_literals == 0 {
            return 0.0;
        }

        let mut entropy: f64 = 0.0;

        for b in 0..256 {
            if stats.literal_count[b] > 0 {
                let fraction = stats.literal_count[b] as f64 / total_literals as f64;
                entropy -= fraction * fraction.log2();
            }
        }

        entropy
    }

    // pub fn update_stats(&mut self, stats: &TokenStats) {
    //     self.literal_count = stats.literal_count;
    //     // let total_literals: u64 = self.literal_count.iter().sum();
    //     // let total_tokens: u64 = stats.token_count.iter().sum();

    //     for b in 0..=255 {
    //         if !self.tokens_by_string.contains_key(&vec![b]) {
    //             self.literal_count[b as usize] += 1;
    //         }
    //     }

    //     // if !self.has_dist_fallback() {
    //     //     return;
    //     // }

    //     // if total_tokens == 0 {
    //     //     self.literal_cost = 8.0;
    //     //     return;
    //     // }

    //     // // Suppose 1 byte has entropy 1 bit
    //     // // Then 1 token = 1 / log2(ntokens) bits of entropy

    //     // let bytes_per_token =
    //     //     (stats.scanned_bytes - total_literals) as f64 / (total_tokens as f64 + 1.0);

    //     // self.literal_cost = 1.0 + self.dist_entropy() / bytes_per_token
    // }

    pub fn reserved_tokens(&self) -> usize {
        self.literal_encoding.reserved_tokens()
    }

    fn add_mandatory_token(&mut self, string: &[u8]) {
        assert!(!self.tokens_by_string.contains_key(string));
        let index = self.tokens.len();
        let token = Token::new(string, true);
        self.tokens_by_string
            .insert(token.string.clone(), index as u32);
        self.tokens.push(token);
    }

    pub fn add_token(&mut self, string: &[u8]) {
        if let Some(&existing) = self.tokens_by_string.get(string) {
            let existing = &self.tokens[existing as usize];
            assert!(existing.is_mandatory);
            return;
        }

        let index = self.tokens.len();
        let token = Token::new(string, false);
        self.tokens_by_string
            .insert(token.string.clone(), index as u32);
        self.tokens.push(token);
    }

    pub fn remove_token(&mut self, token_str: &[u8]) {
        let token_id = *self.tokens_by_string.get(token_str).unwrap() as usize;

        assert!(!self.tokens[token_id].is_mandatory);
        self.tokens.remove(token_id);

        self.tokens_by_string.clear();
        for i in 0..self.tokens.len() {
            let token = &self.tokens[i];
            self.tokens_by_string.insert(token.string.clone(), i as u32);
        }
    }

    pub fn from_json(filename: &str) -> Self {
        let contents = std::fs::read_to_string(filename).unwrap();
        let parsed = json::parse(&contents).unwrap();

        let literal_encoding = match parsed["type"].as_str().unwrap() {
            "str_with_fallback_bits" | "fallback_bits" => {
                match parsed["fallback_bits"].as_usize().unwrap() {
                    1 => LiteralEncoding::Bits1,
                    2 => LiteralEncoding::Bits2,
                    4 => LiteralEncoding::Bits4,
                    _ => unreachable!(),
                }
            }
            "fallback16" => LiteralEncoding::Hex,
            "fallback_distribution" => LiteralEncoding::Dist8,
            "all_tokens" => LiteralEncoding::All,
            _ => unreachable!(),
        };

        let mut token_set = Self::new(literal_encoding);

        for token_str in parsed["tokens"].members() {
            if token_str.is_string() {
                token_set.add_token(token_str.as_str().unwrap().as_bytes());
            } else if token_str.is_array() {
                let mut s = vec![];
                for b in token_str.members() {
                    s.push(b.as_u8().unwrap());
                }
                token_set.add_token(&s);
            } else if token_str.is_number() {
                let v = token_str.as_usize().unwrap();
                assert!(v < literal_encoding.reserved_tokens())
            }
        }

        token_set
    }

    pub fn generate_suffixes(&mut self) {
        for token in self.tokens.iter_mut() {
            if token.string.len() == 1 {
                token.suffix = TokenIdx::None;
                continue;
            }

            token.suffix = TokenIdx::Literal(token.string[token.string.len() - 1]);

            for start in 1..token.string.len() {
                let suffix = &token.string[start..];
                if let Some(&idx) = self.tokens_by_string.get(suffix) {
                    token.suffix = TokenIdx::Token(idx as u32);
                    break;
                }
            }
        }
    }

    pub fn to_json(&self, stats: &TokenStats, initial_size: u64) -> json::JsonValue {
        let mut out = json::object! {
            tokens: [],
            stats: stats.to_json(initial_size, self.literal_encoding.tokens_in_literal(),
                                 self.dist_entropy(stats), self.literal_encoding.reserved_tokens()),
        };

        let mut token_strs = vec![];

        for token in self.tokens.iter() {
            token_strs.push(token.string.clone());
        }

        token_strs.sort_unstable();

        for x in 0..self.literal_encoding.reserved_tokens() {
            out["tokens"].push(x).unwrap();
        }

        for token_str in token_strs.iter() {
            let value: json::JsonValue = match std::str::from_utf8(&token_str) {
                Ok(s) => s.into(),
                Err(_) => token_str.as_slice().into(),
            };

            out["tokens"].push(value).unwrap();
        }

        match self.literal_encoding {
            LiteralEncoding::Hex => {
                out["type"] = "fallback16".into();
            }
            LiteralEncoding::All => {
                out["type"] = "all_tokens".into();
            }
            LiteralEncoding::Bits1 => {
                out["type"] = "fallback_bits".into();
                out["fallback_bits"] = 1.into();
            }
            LiteralEncoding::Bits2 => {
                out["type"] = "fallback_bits".into();
                out["fallback_bits"] = 2.into();
            }
            LiteralEncoding::Bits4 => {
                out["type"] = "fallback_bits".into();
                out["fallback_bits"] = 4.into();
            }
            LiteralEncoding::Dist2 | LiteralEncoding::Dist4 | LiteralEncoding::Dist8 => {
                out["type"] = "fallback_distribution".into();
                out["literal_count"] = json::JsonValue::new_array();
                for &count in stats.literal_count.iter() {
                    out["literal_count"].push(count).unwrap();
                }
            }
        }

        out
    }
}
