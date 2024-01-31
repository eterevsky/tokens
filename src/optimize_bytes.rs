use std::cmp::min;
use std::mem;

use crate::stats2::TokenStats;
use crate::tokenset::{Token, TokenSet, TokenType};
use crate::processing::Processing;


pub trait BytesOptimizer {
    fn optimize_bytes(token_stats: &TokenStats, n_byte_tokens: usize) -> TokenSet;
}

pub struct SimpleBytesOptimizer {}

impl BytesOptimizer for SimpleBytesOptimizer {
    fn optimize_bytes(stats: &TokenStats, n_byte_ext_tokens: usize) -> TokenSet {
        let n_byte_tokens = n_byte_ext_tokens - stats.token_set.n_ext_tokens;

        let mut byte_counts: [i64; 256] = [0; 256];
        let mut single_byte_tokens = Vec::new();

        for (token_id, token) in stats.token_set.tokens.iter().enumerate() {
            if let Token::Str(s) = token {
                if s.len() == 1 {
                    byte_counts[s[0] as usize] = stats.token_counts[token_id] as i64;
                    single_byte_tokens.push(s.clone());
                }
            }
        }

        for (seq_id, seq) in stats.token_set.sequences.iter().enumerate() {
            if seq.string.len() == 1 {
                byte_counts[seq.string[0] as usize] = stats.seq_counts[seq_id] as i64;
            }
        }

        let mut bytes = (0..=255).collect::<Vec<u8>>();
        bytes.sort_by_key(|&i| -byte_counts[i as usize]);
        let selected_bytes = &bytes[..n_byte_tokens];

        let mut new_token_set = match stats.token_set.token_type {
            TokenType::Bits1 => {
                TokenSet::new_bits1(stats.token_set.processing, stats.token_set.split_paragraphs)
            }
            TokenType::Bits2 => {
                TokenSet::new_bits2(stats.token_set.processing, stats.token_set.split_paragraphs)
            }
            TokenType::Bits4 => {
                TokenSet::new_bits4(stats.token_set.processing, stats.token_set.split_paragraphs)
            }
            _ => panic!("BytesOptimizer only works for Bits* TokenSet's"),
        };

        for &byte in selected_bytes.iter() {
            new_token_set.add_token(&[byte]);
        }

        for token in stats.token_set.tokens.iter() {
            if let Token::Str(s) = token {
                if s.len() > 1 {
                    new_token_set.add_token(s);
                }
            }
        }

        new_token_set
    }
}

pub struct NoopBytesOptimizer {}

impl BytesOptimizer for NoopBytesOptimizer {
    fn optimize_bytes(token_stats: &TokenStats, _n_byte_tokens: usize) -> TokenSet {
        token_stats.token_set.clone()
    }
}

pub struct HuffOptimizer {}

impl BytesOptimizer for HuffOptimizer {
    fn optimize_bytes(token_stats: &TokenStats, n_byte_ext_tokens: usize) -> TokenSet {
        let mut counts = (0..=255).map(|i| (i, 1)).collect::<Vec<(u8, u64)>>();
        for (token, count) in token_stats.token_set.tokens.iter().zip(token_stats.token_counts.iter()) {
            if let Token::Str(s) = token {
                if s.len() == 1 {
                    counts[s[0] as usize] = (s[0], count + 1);
                }
            }
        }

        for (seq, count) in token_stats.token_set.sequences.iter().zip(token_stats.seq_counts.iter()) {
            assert_eq!(seq.string.len(), 1);
            counts[seq.string[0] as usize] = (seq.string[0], count + 1);
        }

        let mut best_token_set = None;
        let mut best_total_tokens = None;
    
        let max_ext_tokens = min(n_byte_ext_tokens - 1, 8);
    
        for n_ext_tokens in 2..=max_ext_tokens {
            let n_byte_tokens = n_byte_ext_tokens - n_ext_tokens;
    
            let token_set = optimize_bytes_tokenset(&counts, n_byte_tokens, n_ext_tokens, token_stats.token_set.processing);
    
            let mut total = 0;

            for token in token_set.tokens.iter() {
                if let Token::Str(s) = token {
                    assert_eq!(s.len(), 1);
                    total += counts[s[0] as usize].1
                }
            }

            for seq in token_set.sequences.iter() {
                assert_eq!(seq.string.len(), 1);
                total += counts[seq.string[0] as usize].1 * seq.tokens.len() as u64;
            }
    
            if best_total_tokens.is_none() || total < best_total_tokens.unwrap() {
                best_token_set = Some(token_set);
                best_total_tokens = Some(total);
            }
        }
    
        let mut best_token_set = best_token_set.unwrap();

        // Adding multi-byte tokens from the input tokenset into the new
        // tokenset.
        for token in token_stats.token_set.tokens.iter() {
            if let Token::Str(s) = token {
                if s.len() > 1 {
                    best_token_set.add_token(s);
                }
            }
        }

        best_token_set
    }
}


#[derive(Debug)]
struct CharsSplit {
    /// First character in the range
    lo: u8,
    /// The character in the range with the highest count
    top: u8,
    /// The count for the top character
    top_count: u64,
}

enum HuffmanNodeContent {
    Leaf(u8),
    Internal(Box<HuffmanNode>, Box<HuffmanNode>),
}

struct HuffmanNode {
    count: u64,
    content: HuffmanNodeContent,
}

fn node_to_split(node: &HuffmanNode) -> CharsSplit {
    match &node.content {
        HuffmanNodeContent::Leaf(ch) => CharsSplit {
            lo: *ch,
            top: *ch,
            top_count: node.count,
        },
        HuffmanNodeContent::Internal(first, second) => {
            let first_split = node_to_split(first);
            let second_split = node_to_split(second);

            let (top, top_count) = if first_split.top_count > second_split.top_count {
                (first_split.top, first_split.top_count)
            } else {
                (second_split.top, second_split.top_count)
            };

            CharsSplit {
                lo: min(first_split.lo, second_split.lo),
                top,
                top_count,
            }
        }
    }
}


/// Finds an optimal split of an interval of characters into a given number of
/// parts.
fn optimize_splits(counts: &[(u8, u64)], parts: usize) -> Vec<CharsSplit> {
    let mut nodes = Vec::new();
    for (ch, count) in counts {
        nodes.push(HuffmanNode {
            count: *count,
            content: HuffmanNodeContent::Leaf(*ch),
        });
    }

    while nodes.len() > parts {
        let mut min_pair_count = None;
        let mut best_idx = None;
        for i in 0..(nodes.len() - 1) {
            let pair_count = nodes[i].count + nodes[i + 1].count;
            if min_pair_count.is_none() || pair_count < min_pair_count.unwrap() {
                min_pair_count = Some(pair_count);
                best_idx = Some(i);
            }
        }
        let best_idx = best_idx.unwrap();
        let second = nodes.remove(best_idx + 1);
        let dummy = HuffmanNode {
            count: 0,
            content: HuffmanNodeContent::Leaf(0),
        };
        let first = mem::replace(&mut nodes[best_idx], dummy);
        let new_node = HuffmanNode {
            count: first.count + second.count,
            content: HuffmanNodeContent::Internal(Box::new(first), Box::new(second)),
        };
        nodes[best_idx] = new_node;
    }

    let mut splits = Vec::new();
    for node in nodes.iter() {
        splits.push(node_to_split(node));
    }

    splits
}

/// Optimizes the sequence suffixes for an interval of bytes.
fn optimize_ext_encoding(counts: &[(u8, u64)], n_ext_tokens: usize) -> Vec<(u8, Vec<usize>)> {
    if counts.len() <= n_ext_tokens {
        return counts
            .iter()
            .enumerate()
            .map(|(i, &(ch, _))| (ch, vec![i]))
            .collect::<Vec<_>>();
    }

    let splits = optimize_splits(counts, n_ext_tokens);

    let mut encodings = Vec::new();

    for (i, split) in splits.iter().enumerate() {
        encodings.push((split.top, vec![i]));

        let counts_lo = counts.binary_search(&(split.lo, 0)).unwrap_err();
        let counts_hi = if i == splits.len() - 1 {
            counts.len()
        } else {
            counts.binary_search(&(splits[i + 1].lo, 0)).unwrap_err()
        };

        if counts_hi == counts_lo + 1 {
            continue;
        }

        let mut sub_counts = counts[counts_lo..counts_hi].to_vec();

        let top_idx = sub_counts.binary_search(&(split.top, 0)).unwrap_err();
        sub_counts.remove(top_idx);

        let sub_encs = optimize_ext_encoding(sub_counts.as_slice(), n_ext_tokens);
        for (ch, mut enc) in sub_encs {
            enc.insert(0, i);
            encodings.push((ch, enc));
        }
    }

    encodings
}

fn optimize_bytes_tokenset(
    counts: &[(u8, u64)],
    n_char_tokens: usize,
    n_ext_tokens: usize,
    processing: Processing,
) -> TokenSet {
    let mut token_set = TokenSet::new(n_ext_tokens, processing, TokenType::BytesHuff, true);

    let top_splits = optimize_splits(counts, n_char_tokens);

    for (i, split) in top_splits.iter().enumerate() {
        let top_token_id = token_set.add_token(&[split.top]);

        let counts_lo = counts.binary_search(&(split.lo, 0)).unwrap_err();
        let counts_hi = if i == top_splits.len() - 1 {
            counts.len()
        } else {
            counts
                .binary_search(&(top_splits[i + 1].lo, 0))
                .unwrap_err()
        };

        if counts_hi == counts_lo + 1 {
            continue;
        }

        let mut sub_counts = counts[counts_lo..counts_hi].to_vec();

        let top_idx = sub_counts.binary_search(&(split.top, 0)).unwrap_err();
        sub_counts.remove(top_idx);

        let encs = optimize_ext_encoding(sub_counts.as_slice(), n_ext_tokens);
        for (ch, mut enc) in encs {
            enc.insert(0, top_token_id);
            token_set.add_sequence(vec![ch], enc);
        }
    }

    token_set
}
