use std::collections::HashMap;

use super::stats2::TokenStats;
use super::tokenset::{Token, TokenSet};

enum SpanContent {
    None, // Empty span
    Token(usize),
    Sequence(usize),
}

struct Span {
    content: SpanContent,

    string: Vec<u8>,

    // Index of another span that is the longest suffix of this span.
    suffix_span: usize,

    // 1 for tokens, number of tokens for sequences
    cost: u64,
}

#[derive(Debug)]
struct SuffixState {
    suffix: Vec<u8>,
    span_idx: usize,
    next: [usize; 256],
}

impl SuffixState {
    fn new(suffix: Vec<u8>, span_idx: usize) -> Self {
        SuffixState {
            suffix,
            span_idx,
            next: [0; 256],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CostState {
    cost: u64,
    span: usize,
}

// A synchronous tokenizer,
pub struct FragmentTokenizer {
    pub token_set: TokenSet,
    spans: Vec<Span>,
    suffix_states: Vec<SuffixState>,
}

impl FragmentTokenizer {
    pub fn new(token_set: TokenSet) -> Self {
        let (spans, span_by_str) = Self::create_spans(&token_set);
        let suffix_states = Self::create_suffix_states(&spans, &span_by_str);
        FragmentTokenizer {
            token_set,
            spans,
            suffix_states,
        }
    }

    fn create_spans(token_set: &TokenSet) -> (Vec<Span>, HashMap<Vec<u8>, usize>) {
        let mut spans = Vec::new();
        let mut span_by_str: HashMap<Vec<u8>, usize> = HashMap::new();

        span_by_str.insert(Vec::new(), 0);
        spans.push(Span {
            content: SpanContent::None,
            string: Vec::new(),
            suffix_span: 0,
            cost: 0,
        });

        // Populate tokens
        for (idx, token) in token_set.tokens.iter().enumerate() {
            if let Token::Str(string) = token {
                assert!(span_by_str.insert(string.clone(), spans.len()).is_none());
                spans.push(Span {
                    content: SpanContent::Token(idx),
                    string: string.clone(),
                    suffix_span: 0,
                    cost: 1,
                })
            }
        }

        // Populate sequences
        for (idx, seq) in token_set.sequences.iter().enumerate() {
            assert!(span_by_str
                .insert(seq.string.clone(), spans.len())
                .is_none());
            spans.push(Span {
                content: SpanContent::Sequence(idx),
                string: seq.string.clone(),
                suffix_span: 0,
                cost: seq.tokens.len() as u64,
            })
        }

        // Populate suffix spans
        for span in spans[1..].iter_mut() {
            for start in 1..=(span.string.len() - 1) {
                let suffix = &span.string[start..];
                if let Some(&idx) = span_by_str.get(suffix) {
                    span.suffix_span = idx;
                    break;
                }
            }
        }

        (spans, span_by_str)
    }

    fn create_suffix_states(
        spans: &[Span],
        span_by_str: &HashMap<Vec<u8>, usize>,
    ) -> Vec<SuffixState> {
        let mut suffix_states = Vec::new();
        let mut state_by_str: HashMap<Vec<u8>, usize> = HashMap::new();

        suffix_states.push(SuffixState::new(Vec::new(), 0));

        state_by_str.insert(Vec::new(), 0);

        for span in spans.iter() {
            for end in 1..=span.string.len() {
                // The suffix is a token prefix
                let span_prefix = span.string[..end].to_vec();

                if state_by_str.contains_key(&span_prefix) {
                    continue;
                }

                let mut suffix_span = 0;

                for start in 0..span_prefix.len() {
                    if let Some(&idx) = span_by_str.get(&span_prefix[start..]) {
                        suffix_span = idx;
                        break;
                    }
                }

                assert!(suffix_span > 0);

                let suffix_state = SuffixState::new(span_prefix, suffix_span);

                state_by_str.insert(suffix_state.suffix.clone(), suffix_states.len());
                suffix_states.push(suffix_state);
            }
        }

        for state in suffix_states.iter_mut() {
            let mut suffix = state.suffix.to_vec();

            for last_byte in 0..=255 {
                suffix.push(last_byte);

                let mut suffix_id = 0;

                for start in 0..suffix.len() {
                    let suffix_suffix = &suffix[start..];

                    if let Some(&id) = state_by_str.get(suffix_suffix) {
                        suffix_id = id;
                        break;
                    }
                }

                assert!(suffix_id > 0);
                state.next[last_byte as usize] = suffix_id;

                suffix.pop();
            }
        }

        suffix_states
    }

    pub fn process_slice(&self, bytes: &[u8], stats: &mut TokenStats, cost_state: &mut Vec<CostState>) {
        // let mut cost_state = vec![CostState { cost: 0, span: 0 }];
        cost_state.clear();
        cost_state.push(CostState { cost: 0, span: 0 });
        let mut state = &self.suffix_states[0];

        for &byte in bytes.iter() {
            state = &self.suffix_states[state.next[byte as usize]];

            let mut best_cost_state: Option<CostState> = None;
            let mut span_idx = state.span_idx;

            while span_idx != 0 {
                let span = &self.spans[span_idx];
                let prev_cost = cost_state[cost_state.len() - span.string.len()].cost;
                let cost = prev_cost + span.cost;
                if best_cost_state.is_none() || best_cost_state.unwrap().cost > cost {
                    best_cost_state = Some(CostState {
                        cost,
                        span: span_idx,
                    });
                }

                span_idx = span.suffix_span;
            }

            cost_state.push(best_cost_state.unwrap());
        }

        self.update_stats(cost_state, bytes, stats);
    }

    fn update_stats(&self, cost_state: &Vec<CostState>, bytes: &[u8], stats: &mut TokenStats) {
        stats.total_tokens += cost_state.last().unwrap().cost;
        stats.scanned_bytes += bytes.len() as u64;

        let ntokens = stats.token_set.ntokens();

        let mut span_counts = vec![0; self.spans.len()];
        let mut next_token = None;
        let mut pos = bytes.len();

        while pos > 0 {
            let span_idx = cost_state[pos].span;
            span_counts[span_idx] += 1;

            let span = &self.spans[span_idx];
            next_token = if let SpanContent::Token(token) = span.content {
                if let Some(next) = next_token {
                    let pair_id = token * ntokens + next;
                    stats.pair_counts[pair_id] += 1;
                }

                Some(token)
            } else {
                None
            };

            pos -= span.string.len();
        }

        for span_idx in 1..self.spans.len() {
            let count = span_counts[span_idx];
            let span = &self.spans[span_idx];
            match span.content {
                SpanContent::Sequence(seq_id) => {
                    stats.seq_counts[seq_id] += count;
                    let seq = &stats.token_set.sequences[seq_id];
                    for &token_id in seq.tokens.iter() {
                        stats.token_counts[token_id] += count;
                    }
                },
                SpanContent::Token(token_id) => {
                    stats.token_counts[token_id] += count
                },
                SpanContent::None => {
                    dbg!(cost_state);
                    dbg!(span_idx);
                    unreachable!()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::Processing;

    #[test]
    fn tokenize() {
        let mut token_set = TokenSet::new_bits1(Processing::Raw, true);
        token_set.add_token("a".as_bytes());
        token_set.add_token("ab".as_bytes());
        token_set.add_token("bc".as_bytes());

        let tokenizer = FragmentTokenizer::new(token_set.clone());
        let mut stats = TokenStats::new(token_set, Some(3));
        let mut buffer = Vec::new();

        tokenizer.process_slice("abc".as_bytes(), &mut stats, &mut buffer);
        assert_eq!(stats.total_tokens, 2);
    }

    #[test]
    fn tokenize_seq() {
        let mut token_set = TokenSet::new_bits4(Processing::Raw, true);
        token_set.add_token("ab".as_bytes());
        token_set.add_token("b".as_bytes());
        token_set.add_token("c".as_bytes());
        token_set.add_token("d".as_bytes());
        token_set.add_token("e".as_bytes());
        token_set.add_token("bcde".as_bytes());

        let tokenizer = FragmentTokenizer::new(token_set.clone());
        let mut stats = TokenStats::new(token_set, Some(3));
        let mut buffer = Vec::new();

        tokenizer.process_slice("abcde".as_bytes(), &mut stats, &mut buffer);
        assert_eq!(stats.total_tokens, 3);
    }
}
