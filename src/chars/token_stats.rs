use super::tokens::{CharsToken, CharsTokenSet};
use serde_json::json;
use std::collections::HashMap;

const MAX_UNICODE: usize = 0x110000;

pub struct CharsTokenStats {
    pub token_set: CharsTokenSet,
    total_tokens_count: u64,
    literals_count: u64,
    initial_size: Option<u64>,
    pub(super) pair_counts: HashMap<(u16, u16), u64>,
}

impl CharsTokenStats {
    pub fn new(token_set: CharsTokenSet, initial_size: Option<u64>) -> Self {
        CharsTokenStats {
            token_set,
            total_tokens_count: 0,
            literals_count: 0,
            initial_size,
            pair_counts: HashMap::new(),
        }
    }

    pub fn ntokens(&self) -> usize {
        self.token_set.ntokens()
    }

    pub fn merge(&mut self, other: &CharsTokenStats) {
        self.total_tokens_count += other.total_tokens_count;
        self.literals_count += other.literals_count;
        for (&pair, &count) in other.pair_counts.iter() {
            *self.pair_counts.entry(pair).or_insert(0) += count;
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_tokens_count
    }

    pub fn total_literals(&self) -> u64 {
        self.literals_count
    }

    // Count a token
    pub fn count_token(&mut self, idx: usize) {
        self.total_tokens_count += 1;
        let token = &self.token_set.tokens[idx];
        if let CharsToken::Char(_) = token {
            self.literals_count += 1;
        }
    }

    // Count a literal which is _not_ covered by a single token.
    pub fn count_literal(&mut self, ch: char) {
        let cost = self.token_set.char_cost(ch);
        self.total_tokens_count += cost as u64;
        self.literals_count += 1;
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut j = json!({
            "type": "chars",
            "tokens": self.token_set.tokens_to_json(),
            "encodings": self.token_set.encodings_to_json(),
            "stats": {
                "ntokens": self.token_set.ntokens(),
                "total_tokens": self.total_tokens_count,
            }
        });

        if let Some(s) = self.initial_size {
            j["stats"]["initial_size"] = s.into();
            j["stats"]["bytes_per_token"] = (s as f64 / self.total_tokens_count as f64).into();
        }

        j
    }
}
