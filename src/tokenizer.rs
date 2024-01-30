use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::input::sample::{Sample, Sampler};
use crate::stats::TokenStats;
use crate::tokens::{TokenIdx, TokenSet};

#[derive(Debug)]
struct SuffixState {
    suffix: Vec<u8>,
    token_idx: TokenIdx,
    next: [usize; 256],
}

impl SuffixState {
    fn new(suffix: Vec<u8>, token_idx: TokenIdx) -> Self {
        SuffixState {
            suffix,
            token_idx,
            next: [0; 256],
        }
    }
}

struct DynState {
    cost: u64,
    token_id: TokenIdx,
}

struct Tokenizer {
    token_set: TokenSet,
    suffix_states: Vec<SuffixState>,
}

impl Tokenizer {
    fn new(mut token_set: TokenSet) -> Self {
        token_set.generate_suffixes();
        let suffix_states = Self::create_suffix_states(&token_set);

        Tokenizer {
            token_set,
            suffix_states,
        }
    }

    fn create_suffix_states(token_set: &TokenSet) -> Vec<SuffixState> {
        let mut suffix_states = Vec::new();
        let mut state_by_str: HashMap<Vec<u8>, usize> = HashMap::new();

        suffix_states.push(SuffixState::new(Vec::new(), TokenIdx::None));

        state_by_str.insert(Vec::new(), 0);

        for token in token_set.tokens.iter() {
            for end in 1..=token.string.len() {
                // The suffix is a token prefix
                let suffix = token.string[..end].to_vec();

                if state_by_str.contains_key(&suffix) {
                    continue;
                }

                let mut suffix_token = TokenIdx::Literal(suffix[suffix.len() - 1]);

                for token_start in 0..suffix.len() {
                    if let Some(&idx) = token_set.tokens_by_string.get(&suffix[token_start..]) {
                        suffix_token = TokenIdx::Token(idx);
                        break;
                    }
                }

                let suffix_state = SuffixState::new(suffix, suffix_token);

                state_by_str.insert(suffix_state.suffix.clone(), suffix_states.len());
                suffix_states.push(suffix_state);
            }
        }

        // Add literals, not covered by tokens
        for literal in 0..=255 {
            let suffix = vec![literal];
            if state_by_str.contains_key(&suffix) {
                continue;
            }
            let suffix_state = SuffixState::new(suffix, TokenIdx::Literal(literal));
            state_by_str.insert(suffix_state.suffix.clone(), suffix_states.len());
            suffix_states.push(suffix_state);
        }

        for state in suffix_states.iter_mut() {
            let mut suffix = state.suffix.to_vec();

            for last_byte in 0..=255 {
                suffix.push(last_byte);

                let mut suffix_id: Option<usize> = None;

                for start in 0..suffix.len() {
                    let suffix_suffix = &suffix[start..];

                    if let Some(&id) = state_by_str.get(suffix_suffix) {
                        suffix_id = Some(id);
                        break;
                    }
                }

                state.next[last_byte as usize] = suffix_id.unwrap();

                suffix.pop();
            }
        }

        suffix_states
    }

    fn process_slice(&self, bytes: &[u8], cost_array: &mut Vec<DynState>, pair_stats: bool, stats: &mut TokenStats) {
        // let mut cost_array = Vec::with_capacity(bytes.len() + 1);
        cost_array.clear();
        cost_array.push(DynState {
            cost: 0,
            token_id: TokenIdx::None,
        });


        let literal_cost = self.token_set.literal_cost();

        let mut state = &self.suffix_states[0];

        for &byte in bytes.iter() {
            state = &self.suffix_states[state.next[byte as usize]];

            let best_dyn_state = match state.token_idx {
                TokenIdx::Literal(id) => {
                    let prev_cost = cost_array.last().unwrap().cost;
                    let new_cost = prev_cost + literal_cost;

                    DynState {
                        cost: new_cost,
                        token_id: TokenIdx::Literal(id),
                    }
                }
                TokenIdx::Token(id) => {
                    let mut token = &self.token_set.tokens[id as usize];
                    let prev_cost = cost_array[cost_array.len() - token.string.len()].cost;
                    let new_cost = prev_cost + 1;

                    let mut best_dyn_state = DynState {
                        cost: new_cost,
                        token_id: TokenIdx::Token(id),
                    };
                    loop {
                        match token.suffix {
                            TokenIdx::Token(id) => {
                                token = &self.token_set.tokens[id as usize];
                                let prev_cost =
                                    cost_array[cost_array.len() - token.string.len()].cost;
                                let new_cost = prev_cost + 1;

                                if new_cost < best_dyn_state.cost {
                                    best_dyn_state.cost = new_cost;
                                    best_dyn_state.token_id = TokenIdx::Token(id);
                                }
                            }
                            TokenIdx::Literal(id) => {
                                let prev_cost = cost_array[cost_array.len() - 1].cost;
                                let new_cost = prev_cost + literal_cost;

                                if new_cost < best_dyn_state.cost {
                                    best_dyn_state.cost = new_cost;
                                    best_dyn_state.token_id = TokenIdx::Literal(id);
                                }
                                break;
                            }
                            TokenIdx::None => break,
                        }
                    }
                    best_dyn_state
                }
                TokenIdx::None => unreachable!(),
            };

            cost_array.push(best_dyn_state);
        }
        // self.get_stats(&cost_array, pair_stats)
        self.update_stats(&cost_array, pair_stats, stats);
    }

    fn update_stats(&self, cost_array: &[DynState], pair_stats: bool, stats: &mut TokenStats) {
        let mut pos = cost_array.len() - 1;
        stats.scanned_bytes += pos as u64;

        let mut next_token_id = TokenIdx::None;

        while pos > 0 {
            let token_id = cost_array[pos].token_id;
            match token_id {
                TokenIdx::Token(id) => {
                    stats.token_count[id as usize] += 1;

                    if pair_stats {
                        if let TokenIdx::Token(next_id) = next_token_id {
                            // assert!(next_id < 2048);
                            let key = (id << 16) + next_id;
                            // assert!(key & 0xFFFF < 2048);
                            *stats.pair_count.entry(key).or_insert(0) += 1;
                            // *token_stats.pair_count.entry((id as u16, next_id as u16)).or_insert(0) += 1;
                        }
                    }
                    let token = &self.token_set.tokens[id as usize];
                    pos -= token.string.len();
                }
                TokenIdx::Literal(l) => {
                    stats.literal_count[l as usize] += 1;
                    pos -= 1;
                }
                TokenIdx::None => unreachable!(),
            }

            next_token_id = token_id;
        }
    }
}

fn worker(
    tokenizer: &Tokenizer,
    jobs_rx: Arc<Mutex<Receiver<Sample>>>,
    results_tx: Sender<TokenStats>,
    pair_stats: bool,
) {
    let mut buffer = Vec::new();
    // let mut wait = Vec::new();

    let mut stats = TokenStats::new(tokenizer.token_set.tokens.len(), tokenizer.token_set.literal_cost());

    loop {
        // let start = Instant::now();
        let job = jobs_rx.lock().unwrap().recv();
        // wait.push(start.elapsed().as_millis() as u64);
        let data = {
            match job {
                Ok(ref sample) => sample.as_bytes(),
                Err(_) => break,
            }
        };

        // println!("got sample {}", data.len());

        assert!(!data.is_empty());
        tokenizer.process_slice(data, &mut buffer, pair_stats, &mut stats);
        // dbg!(stats.scanned_bytes);
    }

    // dbg!(stats.scanned_bytes);

    results_tx.send(stats).unwrap();
    // println!("wait {:?}", wait.iter().sum::<u64>() as f64 / wait.len() as f64);
}

pub fn tokenize_file<'a, S: Sampler<'a>>(token_set: &TokenSet, sampler: &'a S, pair_stats: bool) -> TokenStats {
    let nthreads = std::thread::available_parallelism().unwrap().get();
    // dbg!(nthreads);

    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<Sample>(4);
    let jobs_rx_shared = Arc::new(Mutex::new(jobs_rx));
    let (results_tx, results_rx) = mpsc::channel::<TokenStats>();

    let tokenizer = Tokenizer::new(token_set.clone());
    let mut total_stats = TokenStats::new(token_set.tokens.len(), token_set.literal_cost());

    std::thread::scope(|s| {
        let mut join_handles = Vec::new();

        for _ in 0..nthreads {
            let jobs_rx_clone = jobs_rx_shared.clone();
            let results_tx_clone = results_tx.clone();
            join_handles
                .push(s.spawn(|| worker(&tokenizer, jobs_rx_clone, results_tx_clone, pair_stats)));
        }

        let start = Instant::now();
        // let mut jobs_in_flight = 0;

        for sample in sampler.iter() {
            // println!("sending sample of len {}", sample.len());
            jobs_tx.send(sample).unwrap();
            // jobs_in_flight += 1;

            // for result in results_rx.try_iter() {
            //     total_stats.add(&result);
            //     jobs_in_flight -= 1;
            //     let elapsed = std::time::Instant::now() - start;
            //     if sampler.total_size() > 1 << 32 {
            //         print!(
            //             "\rAvg pace: {:.1} MB / s",
            //             total_stats.scanned_bytes as f64 / 1000000.0 / elapsed.as_secs_f64()
            //         );
            //     }
            // }
        }

        std::mem::drop(jobs_tx);

        for _ in 0..nthreads {
            let result = results_rx.recv().unwrap();
            total_stats.add(&result);
        }
        // while jobs_in_flight > 0 {
        //     dbg!(jobs_in_flight);
        //     let result = results_rx.recv().unwrap();
        //     dbg!(result.scanned_bytes);
        //     total_stats.add(&result);
        //     jobs_in_flight -= 1;
        // }
        // dbg!(total_stats.scanned_bytes);

        if total_stats.scanned_bytes > 1 << 34 {
            let elapsed = std::time::Instant::now() - start;
            println!(
                "\rAvg pace: {:.1} MB / s",
                total_stats.scanned_bytes as f64 / 1000000.0 / elapsed.as_secs_f64()
            );
            // eprint!("\r                                          \r");
        }

        while !join_handles.is_empty() {
            join_handles.pop().unwrap().join().unwrap();
        }
    });

    total_stats
}
