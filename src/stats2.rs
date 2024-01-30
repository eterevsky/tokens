use serde_json::{json, Value};

use super::tokenset::TokenSet;

#[derive(Debug)]
pub struct TokenStats {
    pub token_set: TokenSet,
    pub total_tokens: u64,
    pub initial_size: Option<u64>,
    pub scanned_bytes: u64,
    pub token_counts: Vec<u64>,
    pub seq_counts: Vec<u64>,
    /// Counts for pairs of tokens (token1, token2). Indexed by
    /// token1_id * ntokens + token2_id
    pub pair_counts: Vec<u64>,
}

impl TokenStats {
    pub fn new(token_set: TokenSet, initial_size: Option<u64>) -> Self {
        let ntokens = token_set.tokens.len();
        let nseqs = token_set.sequences.len();
        TokenStats {
            token_set,
            total_tokens: 0,
            initial_size,
            scanned_bytes: 0,
            token_counts: vec![0; ntokens],
            seq_counts: vec![0; nseqs],
            pair_counts: vec![0; ntokens*ntokens],
        }
    }

    pub fn ntokens(&self) -> usize {
        self.token_set.ntokens()
    }

    pub fn bytes_per_token(&self) -> f64 {
        self.scanned_bytes as f64 / self.total_tokens as f64
    }

    pub fn to_json(&self) -> Value {
        let mut result = self.token_set.to_json();

        let mut stats = json!({
            "ntokens": self.ntokens(),
            "total_tokens": self.total_tokens,
            "scanned_bytes": self.scanned_bytes,
        });
        if let Some(s) = self.initial_size {
            stats["initial_size"] = s.into();
            stats["bytes_per_token"] = (s as f64 / self.total_tokens as f64).into();
        }

        result["stats"] = stats;

        result
    }

    pub fn merge(&mut self, other: &TokenStats) {
        self.total_tokens += other.total_tokens;
        self.scanned_bytes += other.scanned_bytes;
        for i in 0..self.token_counts.len() {
            self.token_counts[i] += other.token_counts[i];
        }
        for i in 0..self.seq_counts.len() {
            self.seq_counts[i] += other.seq_counts[i];
        }
    }
}
