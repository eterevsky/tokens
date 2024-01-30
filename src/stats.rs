use std::collections::HashMap;

use json::JsonValue;

#[derive(Clone, Debug)]
pub struct TokenStats {
    literal_cost: u64,
    pub token_count: Vec<u64>,
    // Will work for up to 2^16 tokens
    // pub pair_count: HashMap<(u16, u16), u64>,
    pub pair_count: HashMap<u32, u64>,
    pub literal_count: [u64; 256],
    pub scanned_bytes: u64,
}

impl TokenStats {
    pub fn new(ntokens: usize, literal_cost: u64) -> Self {
        let mut token_count = Vec::new();
        token_count.resize(ntokens, 0);

        TokenStats {
            literal_cost,
            token_count,
            pair_count: HashMap::new(),
            literal_count: [0; 256],
            scanned_bytes: 0,
        }
    }

    pub fn add(&mut self, other: &TokenStats) {
        for i in 0..self.token_count.len() {
            self.token_count[i] += other.token_count[i];
        }
        for (&pair, count) in other.pair_count.iter() {
            *self.pair_count.entry(pair).or_insert(0) += count;
        }
        for i in 0..256 {
            self.literal_count[i] += other.literal_count[i];
        }
        self.scanned_bytes += other.scanned_bytes;
    }

    pub fn total_literals(&self) -> u64 {
        self.literal_count.iter().sum()
    }

    pub fn total_tokens(&self) -> u64 {
        self.token_count.iter().sum()
    }

    pub fn cost(&self) -> u64 {
        self.token_count.iter().sum::<u64>() + self.literal_cost * self.literal_count.iter().sum::<u64>()
    }

    // pub fn average_token_entropy(&self) -> f64 {
    //     let mut total_entropy = 0.0;
    //     let total_tokens = self.total_tokens() as f64;

    //     for token_count in self.token_count.iter() {
    //         let token_count = *token_count as f64;
    //         total_entropy += token_count * (token_count/ total_tokens).log2();
    //     }

    //     - total_entropy / total_tokens
    // }

    pub fn to_json(
        &self,
        initial_size: u64,
        tokens_in_literal: u64,
        literal_dist_entropy: f64,
        reserved_tokens: usize,
    ) -> JsonValue {
        let total_cost = self.cost();
        let final_tokens = self.total_tokens() + tokens_in_literal * self.total_literals();
        json::object! {
            ntokens: self.token_count.len() + reserved_tokens,
            literal_cost: self.literal_cost,
            initial_size: initial_size,
            scanned_bytes: self.scanned_bytes,
            total_cost: total_cost,
            total_tokens: self.total_tokens(),
            total_literals: self.total_literals(),
            final_tokens: final_tokens,
            bytes_per_token: initial_size as f64 / final_tokens as f64,
            literal_dist_entropy: literal_dist_entropy,
            cost_per_byte: total_cost as f64 / initial_size as f64,
        }
    }
}
