use clap::ValueEnum;
use serde::Serialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::fmt;

use super::processing::Processing;

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    /// Ext tokens 0 and 1 are used to encode bytes bit by bit. (≥2 tokens)
    Bits1,
    /// Ext tokens 0..4 are used to encode bytes. (≥4 tokens)
    Bits2,
    /// Ext tokens 0..16 are used to encode bytes. (≥16 tokens)
    Bits4,
    /// All bytes have their own tokens. (≥256 tokens)
    Bytes,
    /// Missing bytes are represented as sequences of ext tokens, based on
    /// their frequency. (≥3 tokens)
    BytesHuff,
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TokenType::Bits1 => "bits1",
                TokenType::Bits2 => "bits2",
                TokenType::Bits4 => "bits4",
                TokenType::Bytes => "bytes",
                TokenType::BytesHuff => "byteshuff",
            }
        )
    }
}

/// A single token that will be part of the final tokenization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Token {
    /// A token representing a string of bytes.
    Str(Vec<u8>),
    /// A token which is used to represent bytes/characters that aren't
    /// covered by `Str` tokens.
    Ext(u8),
}

fn bytes_to_json(bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.into(),
        Err(_) => json!(bytes),
    }
}

pub fn show_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => format!("{:?}", s),
        Err(_) => format!("{:?}", bytes),
    }
}

impl Token {
    fn to_json(&self) -> Value {
        match self {
            Token::Ext(n) => (*n).into(),
            Token::Str(bytes) => bytes_to_json(bytes),
        }
    }
}

impl Ord for Token {
    fn cmp(&self, other: &Token) -> Ordering {
        match (self, other) {
            (Token::Ext(x), Token::Ext(y)) => x.cmp(y),
            (Token::Ext(_), Token::Str(_)) => Ordering::Less,
            (Token::Str(_), Token::Ext(_)) => Ordering::Greater,
            (Token::Str(x), Token::Str(y)) => x.cmp(y),
        }
    }
}

impl PartialOrd for Token {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A substring of text covered by one token or a sequence of tokens in such
/// a way that it couldn't be subdivided into smaller strings
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sequence {
    pub string: Vec<u8>,
    /// Sequence of one or more token indices that encode this string.
    pub tokens: Vec<usize>,
}

impl Sequence {
    fn to_json(&self, token_set: &TokenSet) -> Value {
        let seq = self
            .tokens
            .iter()
            .map(|idx| (&token_set.tokens[*idx]).to_json())
            .collect::<Vec<Value>>();
        json!({
            "string": bytes_to_json(self.string.as_slice()),
            "tokens": seq,
        })
    }
}

impl Ord for Sequence {
    fn cmp(&self, other: &Self) -> Ordering {
        self.string.cmp(&other.string)
    }
}

impl PartialOrd for Sequence {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug)]
pub struct TokenSet {
    pub n_ext_tokens: usize,
    /// The type of the token set, specifying how it encodes bytes or characters
    /// that don't have specific tokens associated with them.
    pub token_type: TokenType,
    /// Type of pre-processing that should be done to the text before tokenization.
    pub processing: Processing,
    /// If true, the tokens can span accross paragraphs, i.e. a token can't have
    /// any non '\n' characters after "\n\n".
    pub split_paragraphs: bool,

    pub tokens: Vec<Token>,
    pub sequences: Vec<Sequence>,
}

impl TokenSet {
    pub fn new(
        n_ext_tokens: usize,
        processing: Processing,
        token_type: TokenType,
        split_paragraphs: bool,
    ) -> Self {
        assert!(n_ext_tokens < 256);
        let tokens = (0..n_ext_tokens)
            .map(|i| Token::Ext(i as u8))
            .collect::<Vec<_>>();

        TokenSet {
            n_ext_tokens,
            token_type,
            processing,
            tokens,
            sequences: Vec::new(),
            split_paragraphs,
        }
    }

    pub fn new_bits1(processing: Processing, split_paragraphs: bool) -> Self {
        let mut token_set = Self::new(2, processing, TokenType::Bits1, split_paragraphs);
        for c in 0..256 {
            let mut rem = c;
            let mut bits = Vec::new();
            for _ in 0..8 {
                bits.push(rem & 1);
                rem >>= 1;
            }
            bits.reverse();
            token_set.add_sequence(vec![c as u8], bits);
        }

        token_set
    }

    pub fn new_bits2(processing: Processing, split_paragraphs: bool) -> Self {
        let mut token_set = Self::new(4, processing, TokenType::Bits2, split_paragraphs);
        for c in 0..256 {
            let mut rem = c;
            let mut digits = Vec::new();
            for _ in 0..4 {
                digits.push(rem & 3);
                rem >>= 2;
            }
            digits.reverse();
            token_set.add_sequence(vec![c as u8], digits);
        }

        token_set
    }

    pub fn new_bits4(processing: Processing, split_paragraphs: bool) -> Self {
        let mut token_set = Self::new(16, processing, TokenType::Bits4, split_paragraphs);
        for c in 0..256 {
            token_set.add_sequence(vec![c as u8], vec![c >> 4, c & 15]);
        }

        token_set
    }

    pub fn new_bytes(processing: Processing) -> Self {
        let mut token_set = Self::new(0, processing, TokenType::Bytes, true);
        for c in 0..256 {
            token_set.add_token(&[c as u8]);
        }

        token_set
    }

    pub fn from_json(value: Value) -> Self {
        let n_ext_tokens = value["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v.is_number())
            .count();
        let processing = match value["processing"].as_str() {
            Some("raw") => Processing::Raw,
            Some("capswords") => Processing::CapsWords,
            _ => panic!("Unexpected processing type."),
        };
        let token_type = match value["type"].as_str() {
            Some("fallback_bits") => match value["fallback_bits"].as_i64().unwrap() {
                1 => TokenType::Bits1,
                2 => TokenType::Bits2,
                4 => TokenType::Bits4,
                _ => unreachable!(),
            },
            Some("all_tokens") => TokenType::Bytes,
            Some("bits1") => TokenType::Bits1,
            Some("bits2") => TokenType::Bits2,
            Some("bits4") => TokenType::Bits4,
            Some("bytes") => TokenType::Bytes,
            Some("byteshuff") => TokenType::BytesHuff,
            _ => panic!("Unknown token type"),
        };
        let split_paragraphs = match value.get("split_paragraph") {
            None => false,
            Some(&Value::Bool(v)) => v,
            _ => panic!("Can't parse split_paragraphs field."),
        };

        let mut token_set = match token_type {
            TokenType::Bits1 => {
                assert_eq!(n_ext_tokens, 2);
                TokenSet::new_bits1(processing, split_paragraphs)
            }
            TokenType::Bits2 => {
                assert_eq!(n_ext_tokens, 4);
                TokenSet::new_bits2(processing, split_paragraphs)
            }
            TokenType::Bits4 => {
                assert_eq!(n_ext_tokens, 16);
                TokenSet::new_bits4(processing, split_paragraphs)
            }
            TokenType::Bytes => {
                assert_eq!(n_ext_tokens, 0);
                TokenSet::new(0, processing, TokenType::Bytes, split_paragraphs)
            }
            other => TokenSet::new(n_ext_tokens, processing, other, split_paragraphs),
        };
        for token in value["tokens"].as_array().unwrap().iter() {
            let token = match &token {
                &Value::Array(v) => v
                    .iter()
                    .map(|b| b.as_i64().unwrap() as u8)
                    .collect::<Vec<_>>(),
                &Value::String(s) => s.as_bytes().to_vec(),
                &Value::Number(_) => continue,
                _ => panic!("Unexpected token"),
            };

            token_set.add_token(&token);
        }

        token_set
    }

    pub fn name(&self) -> String {
        format!(
            "tokens{}_{}_{}",
            self.ntokens(),
            self.processing,
            self.token_type
        )
    }

    /// Returns the minimum number of Ext and single-byte tokens that a 
    /// tokenset of this type can have.
    pub fn min_bytes_ext_tokens(&self) -> usize {
        match self.token_type {
            TokenType::Bits1 => 2,
            TokenType::Bits2 => 4,
            TokenType::Bits4 => 16,
            TokenType::Bytes => 256,
            TokenType::BytesHuff => 3,
        }
    }

    pub fn add_sequence(&mut self, string: Vec<u8>, tokens: Vec<usize>) {
        let sequence = Sequence { string, tokens };
        self.sequences.push(sequence);
    }

    pub fn add_token(&mut self, token: &[u8]) -> usize {
        assert!(!token.is_empty());
        self.sequences.retain(|s| s.string != token);
        let token = Token::Str(token.to_vec());
        let idx = self.tokens.len();
        self.tokens.push(token);
        idx
    }

    pub fn remove_token(&mut self, token_idx: usize) {
        for seq in self.sequences.iter() {
            assert!(!seq.tokens.contains(&token_idx));
        }

        let last_idx = self.tokens.len() - 1;

        if token_idx == last_idx {
            self.tokens.pop();
        } else {
            let last_token = self.tokens.pop().unwrap();
            self.tokens[token_idx] = last_token;

            for seq in self.sequences.iter_mut() {
                for tok in seq.tokens.iter_mut() {
                    if *tok == last_idx {
                        *tok = token_idx
                    }
                }
            }
        }
    }

    pub fn find_token(&self, s: &[u8]) -> Option<usize> {
        self.tokens.iter().position(|token| {
            if let Token::Str(x) = token {
                x == s
            } else {
                false
            }
        })
    }

    pub fn ntokens(&self) -> usize {
        self.tokens.len()
    }

    pub fn n_long_tokens(&self) -> usize {
        self.tokens
            .iter()
            .filter(|t| {
                if let Token::Str(s) = t {
                    s.len() > 1
                } else {
                    false
                }
            })
            .count()
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "type": self.token_type,
            "processing": self.processing,
            "tokens": self.tokens.iter().map(|t| t.to_json()).collect::<Vec<_>>(),
            "split_paragraphs": self.split_paragraphs,
        });
        let sequences = self
            .sequences
            .iter()
            .filter(|s| s.tokens.len() > 1)
            .map(|s| s.to_json(self))
            .collect::<Vec<_>>();
        if !sequences.is_empty() {
            value["sequences"] = json!(sequences);
        }
        value
    }

    pub fn sort(&mut self) {
        let mut token_idxs = (0..self.tokens.len()).collect::<Vec<usize>>();
        token_idxs.sort_by_key(|&id| &self.tokens[id]);

        let mut new_indices = vec![0; self.tokens.len()];
        for (new_pos, &current_pos) in token_idxs.iter().enumerate() {
            new_indices[current_pos] = new_pos;
        }

        for seq in self.sequences.iter_mut() {
            for i in 0..seq.tokens.len() {
                // println!("{} {}", i, seq.tokens[i], );
                seq.tokens[i] = new_indices[seq.tokens[i]];
            }
        }

        self.tokens.sort();
        self.sequences.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn token_to_json() {
        assert_eq!(Token::Ext(127).to_json(), Value::Number(127.into()));
        assert_eq!(
            Token::Str(vec![0xd0, 0xb0, 0xd0, 0xb1]).to_json(),
            Value::String("аб".to_string())
        );
        assert_eq!(
            Token::Str(vec![0xb0, 0xff]).to_json(),
            Value::Array(vec![Value::Number(0xb0.into()), Value::Number(0xff.into())])
        );
    }

    #[test]
    fn token_set_to_json() {
        let mut token_set = TokenSet::new_bits4(Processing::Raw, true);
        token_set.add_token("a".as_bytes());
        token_set.add_token("b".as_bytes());
        token_set.add_token("c".as_bytes());

        let value = token_set.to_json();

        assert_eq!(value["type"], "bits4");
        assert_eq!(value["tokens"].as_array().unwrap().len(), 19);
        assert_eq!(value["sequences"].as_array().unwrap().len(), 253);

        let _new_token_set = TokenSet::from_json(value);
    }

    #[test]
    fn token_set_name() {
        let mut token_set = TokenSet::new_bits4(Processing::Raw, true);
        token_set.add_token("a".as_bytes());
        token_set.add_token("b".as_bytes());
        token_set.add_token("c".as_bytes());

        assert_eq!(token_set.name(), "tokens19_raw_bits4");
    }

    #[test]
    fn sort() {
        let mut token_set = TokenSet::new(2, Processing::Raw, TokenType::BytesHuff, true);
        token_set.add_token("b".as_bytes()); // 2
        token_set.add_token("a".as_bytes()); // 3
        token_set.add_token("c".as_bytes()); // 4

        token_set.add_sequence("e".as_bytes().to_vec(), vec![3, 0]); // "a", 0
        token_set.add_sequence("d".as_bytes().to_vec(), vec![2, 3, 1]); // "b", "a", 1

        token_set.sort();

        assert_eq!(token_set.tokens[0], Token::Ext(0));
        assert_eq!(token_set.tokens[1], Token::Ext(1));
        assert_eq!(token_set.tokens[2], Token::Str("a".as_bytes().to_vec()));
        assert_eq!(token_set.tokens[3], Token::Str("b".as_bytes().to_vec()));
        assert_eq!(token_set.tokens[4], Token::Str("c".as_bytes().to_vec()));

        assert_eq!(
            token_set.sequences[0],
            Sequence {
                string: "d".as_bytes().to_vec(),
                tokens: vec![3, 2, 1]
            }
        );
        assert_eq!(
            token_set.sequences[1],
            Sequence {
                string: "e".as_bytes().to_vec(),
                tokens: vec![2, 0]
            }
        );
    }

    #[test]
    fn remove_token() {
        let mut token_set = TokenSet::new(2, Processing::Raw, TokenType::BytesHuff, true);
        token_set.add_token("a".as_bytes()); // 2
        token_set.add_token("bc".as_bytes()); // 3
        token_set.add_token("c".as_bytes()); // 4

        token_set.add_sequence("d".as_bytes().to_vec(), vec![2, 0]); // "a", 0
        token_set.add_sequence("e".as_bytes().to_vec(), vec![4, 2, 1]); // "c", "a", 1

        token_set.remove_token(3);

        assert_eq!(token_set.tokens.len(), 4);
        assert_eq!(token_set.tokens[3], Token::Str("c".as_bytes().to_vec()));

        assert_eq!(
            token_set.sequences[0],
            Sequence {
                string: "d".as_bytes().to_vec(),
                tokens: vec![2, 0]
            }
        );
        assert_eq!(
            token_set.sequences[1],
            Sequence {
                string: "e".as_bytes().to_vec(),
                tokens: vec![3, 2, 1]
            }
        );
    }
}
