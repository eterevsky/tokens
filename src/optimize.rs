use crate::batch_tokenize::tokenize_file;
use crate::input::sample::Sampler;
use crate::processing::Processing;
use crate::stats2::TokenStats;
use crate::tokenset::{show_bytes, Token, TokenSet, TokenType};

trait BytesOptimizer {
    fn optimize_bytes(token_stats: &TokenStats, n_byte_tokens: usize) -> TokenSet;
}

struct SimpleBytesOptimizer {}

impl BytesOptimizer for SimpleBytesOptimizer {
    fn optimize_bytes(stats: &TokenStats, n_byte_tokens: usize) -> TokenSet {
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

struct NoopBytesOptimizer {}

impl BytesOptimizer for NoopBytesOptimizer {
    fn optimize_bytes(token_stats: &TokenStats, _n_byte_tokens: usize) -> TokenSet {
        token_stats.token_set.clone()
    }
}

fn add_remove_token<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    stats: &TokenStats,
    ntokens: usize,
    sampler: &'a S,
    _bytes_optimizer: &BO,
) -> Option<TokenStats> {
    let pair_idx = (0..stats.pair_counts.len())
        .max_by_key(|&i| stats.pair_counts[i])
        .unwrap();
    let itoken1 = pair_idx / stats.ntokens();
    let itoken2 = pair_idx % stats.ntokens();

    let new_token = match (
        &stats.token_set.tokens[itoken1],
        &stats.token_set.tokens[itoken2],
    ) {
        (Token::Str(s1), Token::Str(s2)) => {
            s1.iter().chain(s2.iter()).cloned().collect::<Vec<u8>>()
        }
        _ => unreachable!(),
    };

    let mut new_token_set = stats.token_set.clone();
    new_token_set.add_token(&new_token);

    if new_token_set.ntokens() <= ntokens {
        let new_stats = tokenize_file(&new_token_set, sampler, None);
        if new_stats.total_tokens < stats.total_tokens {
            println!("Added token {}", show_bytes(&new_token));
            return Some(new_stats);
        }
    } else {
        assert_eq!(new_token_set.ntokens(), ntokens + 1);

        if new_token_set.n_ext_tokens + new_token_set.n_long_tokens() < ntokens {
            let new_stats = tokenize_file(&new_token_set, sampler, None);
            let token_set_new_bytes = BO::optimize_bytes(
                &new_stats,
                ntokens - new_token_set.n_ext_tokens - new_token_set.n_long_tokens(),
            );
            let new_stats = tokenize_file(&token_set_new_bytes, sampler, None);
            if new_stats.total_tokens < stats.total_tokens {
                println!(
                    "Added token {} and updated 1-byte tokens",
                    show_bytes(&new_token)
                );
                return Some(new_stats);
            }
        }

        println!("Trying to add token {}", show_bytes(&new_token));
        print!("Trying to remove tokens:");
        for (token_idx, token) in new_token_set.tokens.iter().enumerate() {
            if let Token::Str(s) = token {
                if s.len() > 1 && s != &new_token {
                    print!(" {}", show_bytes(&s));
                    let mut newer_token_set = new_token_set.clone();
                    newer_token_set.remove_token(token_idx);

                    let newer_stats = tokenize_file(&newer_token_set, sampler, None);
                    let newer_token_set = BO::optimize_bytes(
                        &newer_stats,
                        ntokens - newer_token_set.n_ext_tokens - newer_token_set.n_long_tokens(),
                    );
                    let newer_stats = tokenize_file(&newer_token_set, sampler, None);

                    if newer_stats.total_tokens < stats.total_tokens {
                        println!();
                        println!("Removed {}", show_bytes(s));
                        return Some(newer_stats);
                    }
                }
            }
        }
        println!()
    }

    None
}

fn optimization_step<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    token_set: &TokenSet,
    ntokens: usize,
    sampler: &'a S,
    bytes_optimizer: &BO,
) -> Option<TokenSet> {
    let stats = tokenize_file(&token_set, sampler, None);

    let n_ext_tokens = token_set.n_ext_tokens;
    let n_long_tokens = token_set.n_long_tokens();

    let new_token_set = BO::optimize_bytes(&stats, ntokens - n_ext_tokens - n_long_tokens);
    let new_stats = tokenize_file(&new_token_set, sampler, None);
    if new_stats.total_tokens < stats.total_tokens {
        println!(
            "Updated encoding of single bytes. New bytes/token: {}",
            stats.bytes_per_token()
        );
        return Some(new_stats.token_set);
    }

    if let Some(new_stats) = add_remove_token(&stats, ntokens, sampler, bytes_optimizer) {
        return Some(new_stats.token_set);
    }

    None
}

fn optimize_tokenset_impl<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    mut token_set: TokenSet,
    ntokens: usize,
    sampler: &'a S,
    bytes_optimizer: &BO,
    initial_size: Option<u64>,
) -> TokenStats {
    let stats = tokenize_file(&token_set, sampler, initial_size);
    println!(
        "Initial tokens: {}, bytes/token = {}",
        token_set.ntokens(),
        stats.bytes_per_token()
    );

    loop {
        if let Some(new_token_set) =
            optimization_step(&token_set, ntokens, sampler, bytes_optimizer)
        {
            token_set = new_token_set;
        } else {
            break;
        }
    }

    token_set.sort();
    tokenize_file(&token_set, sampler, initial_size)
}

pub fn optimize_tokenset<'a, S: Sampler<'a>>(
    ntokens: usize,
    sampler: &'a S,
    processing: Processing,
    token_type: TokenType,
    initial_size: Option<u64>,
) -> TokenStats {
    let bytes_optimizer = SimpleBytesOptimizer {};
    match token_type {
        TokenType::Bits1 => {
            let token_set = TokenSet::new_bits1(processing, true);
            optimize_tokenset_impl(token_set, ntokens, sampler, &bytes_optimizer, initial_size)
        }
        TokenType::Bits2 => {
            let token_set = TokenSet::new_bits2(processing, true);
            optimize_tokenset_impl(token_set, ntokens, sampler, &bytes_optimizer, initial_size)
        }
        TokenType::Bits4 => {
            let token_set = TokenSet::new_bits4(processing, true);
            optimize_tokenset_impl(token_set, ntokens, sampler, &bytes_optimizer, initial_size)
        }
        TokenType::Bytes => {
            let token_set = TokenSet::new_bits4(processing, true);
            let noop_bytes_optimizer = NoopBytesOptimizer {};
            optimize_tokenset_impl(
                token_set,
                ntokens,
                sampler,
                &noop_bytes_optimizer,
                initial_size,
            )
        }
        TokenType::BytesHuff => unimplemented!(),
        TokenType::CharsHuff => unimplemented!(),
    }
}
