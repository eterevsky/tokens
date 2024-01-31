use std::collections::HashMap;
use std::io::Write;

use crate::batch_tokenize::TokenizerCache;
use crate::input::sample::Sampler;
use crate::optimize_bytes::{
    BytesOptimizer, HuffOptimizer, NoopBytesOptimizer, SimpleBytesOptimizer,
};
use crate::processing::Processing;
use crate::stats2::TokenStats;
use crate::tokenset::{show_bytes, Token, TokenSet, TokenType};

fn is_valid_token(s: &[u8]) -> bool {
    let n = "\n".as_bytes()[0];
    for i in 0..(s.len() - 2) {
        if s[i] == n && s[i + 1] == n && s[i + 2] != n {
            return false;
        }
    }

    true
}

fn add_token_bpe<'a, S: Sampler<'a>>(
    token_set: &TokenSet,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
) -> Option<(TokenSet, Vec<u8>)> {
    let stats = tokenizer_cache.get_stats_with_pairs(token_set);
    let mut pairs = (0..stats.pair_counts.len())
        .filter(|&i| stats.pair_counts[i] > 0)
        .collect::<Vec<usize>>();
    pairs.sort_by_key(|&i| -(stats.pair_counts[i] as i64));

    let mut new_token = Vec::new();

    for pair_idx in pairs.iter() {
        let itoken1 = pair_idx / stats.ntokens();
        let itoken2 = pair_idx % stats.ntokens();

        new_token = match (
            &stats.token_set.tokens[itoken1],
            &stats.token_set.tokens[itoken2],
        ) {
            (Token::Str(s1), Token::Str(s2)) => {
                s1.iter().chain(s2.iter()).cloned().collect::<Vec<u8>>()
            }
            _ => unreachable!(),
        };

        if is_valid_token(&new_token) {
            break;
        }
    }

    if new_token.is_empty() {
        dbg!(&token_set.tokens);
        dbg!(stats.token_counts);
        dbg!(stats.pair_counts);

        return None
    }

    let mut new_token_set = token_set.clone();
    new_token_set.add_token(&new_token);

    Some((new_token_set, new_token))
}

fn add_remove_token<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    token_set: &TokenSet,
    ntokens: usize,
    _bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
) -> Option<TokenStats> {
    let add_bpe_res = add_token_bpe(token_set, tokenizer_cache);
    if add_bpe_res.is_none() { return None; }
    let (new_token_set, new_token) = add_bpe_res.unwrap();
    println!("Trying to add token {}", show_bytes(&new_token));
    let stats = tokenizer_cache.get_stats(token_set);

    if new_token_set.ntokens() <= ntokens {
        let new_stats = tokenizer_cache.get_stats(&new_token_set);
        if new_stats.total_tokens < stats.total_tokens {
            println!("Added token {}", show_bytes(&new_token));
            return Some(new_stats.clone());
        }
    } else {
        assert_eq!(new_token_set.ntokens(), ntokens + 1);

        if ntokens - new_token_set.n_long_tokens() >= new_token_set.min_bytes_ext_tokens() {
            let new_stats = tokenizer_cache.get_stats(&new_token_set);
            let token_set_new_bytes =
                BO::optimize_bytes(&new_stats, ntokens - new_token_set.n_long_tokens());
            let new_stats = tokenizer_cache.get_stats(&token_set_new_bytes);
            if new_stats.total_tokens < stats.total_tokens {
                println!(
                    "Added token {} and updated 1-byte tokens",
                    show_bytes(&new_token)
                );
                return Some(new_stats.clone());
            }
        }

        print!("Trying to remove tokens:");
        for (token_idx, token) in new_token_set.tokens.iter().enumerate() {
            if let Token::Str(s) = token {
                if s.len() > 1 && s != &new_token {
                    print!(" {}", show_bytes(&s));
                    std::io::stdout().flush().unwrap();
                    let mut newer_token_set = new_token_set.clone();
                    newer_token_set.remove_token(token_idx);

                    let newer_stats = tokenizer_cache.get_stats(&newer_token_set);
                    let newer_token_set =
                        BO::optimize_bytes(&newer_stats, ntokens - newer_token_set.n_long_tokens());
                    let newer_stats = tokenizer_cache.get_stats(&newer_token_set);

                    if newer_stats.total_tokens < stats.total_tokens {
                        println!();
                        println!("Removed {}", show_bytes(s));
                        return Some(newer_stats.clone());
                    }
                }
            }
        }
        println!()
    }

    None
}

fn remove_add_token<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    token_set: &TokenSet,
    ntokens: usize,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
    removal_count: &mut HashMap<Vec<u8>, usize>,
) -> Option<TokenStats> {
    if token_set.ntokens() < ntokens {
        return add_remove_token(token_set, ntokens, bytes_optimizer, tokenizer_cache);
    }

    assert_eq!(token_set.ntokens(), ntokens);

    let stats = tokenizer_cache.get_stats(token_set);

    if token_set.ntokens() - token_set.n_long_tokens() > token_set.min_bytes_ext_tokens() {
        let new_token_set =
            BO::optimize_bytes(&stats, token_set.ntokens() - token_set.n_long_tokens() - 1);
        assert!(new_token_set.ntokens() == ntokens - 1);
        if let Some((new_token_set, new_token)) = add_token_bpe(&new_token_set, tokenizer_cache) {
            let new_stats = tokenizer_cache.get_stats(&new_token_set);
            if new_stats.total_tokens < stats.total_tokens {
                println!("Added {}", show_bytes(&new_token));
                return Some(new_stats);
            }
        }
    }

    let mut to_remove = vec![];
    for token in token_set.tokens.iter() {
        if let Token::Str(s) = token {
            if s.len() > 1 {
                to_remove.push(s.clone())
            }
        }
    }
    to_remove.sort_unstable_by_key(|s| removal_count.get(s).unwrap_or(&0));

    print!("Removing:");
    for s in to_remove {
        *removal_count.entry(s.clone()).or_insert(0) += 1;
        let mut new_token_set = token_set.clone();
        print!(" {}", show_bytes(s.as_slice()));
        std::io::stdout().flush().unwrap();
        let token_idx = new_token_set.find_token(&s).unwrap();
        new_token_set.remove_token(token_idx);
        assert!(new_token_set.ntokens() == ntokens - 1);

        if let Some((new_token_set, new_token)) = add_token_bpe(&new_token_set, tokenizer_cache) {
            assert!(new_token_set.ntokens() == ntokens);
            let new_stats = tokenizer_cache.get_stats(&new_token_set);
    
            if new_stats.total_tokens < stats.total_tokens {
                println!();
                println!("{} -> {}", show_bytes(s.as_slice()), show_bytes(&new_token));
                return Some(new_stats);
            }    
        }
    }
    println!();

    None
}

fn optimization_step<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    token_set: &TokenSet,
    ntokens: usize,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
    removal_count: &mut HashMap<Vec<u8>, usize>,
) -> Option<TokenSet> {
    let stats = tokenizer_cache.get_stats(token_set);

    let new_token_set = BO::optimize_bytes(&stats, ntokens - token_set.n_long_tokens());
    let new_stats = tokenizer_cache.get_stats(&new_token_set);
    if new_stats.total_tokens < stats.total_tokens {
        println!(
            "Updated encoding of single bytes. New bytes/token: {}",
            stats.bytes_per_token()
        );
        return Some(new_stats.token_set);
    }

    if let Some(new_stats) = remove_add_token(
        token_set,
        ntokens,
        bytes_optimizer,
        tokenizer_cache,
        removal_count,
    ) {
        assert!(new_stats.token_set.ntokens() <= ntokens);
        return Some(new_stats.token_set);
    }

    // if let Some(new_stats) = add_remove_token(token_set, ntokens, bytes_optimizer, tokenizer_cache)
    // {
    //     assert!(new_stats.token_set.ntokens() <= ntokens);
    //     return Some(new_stats.token_set);
    // }

    None
}

fn optimize_tokenset_impl<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    mut token_set: TokenSet,
    ntokens: usize,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
) -> TokenStats {
    let stats = tokenizer_cache.get_stats(&token_set);
    println!(
        "Initial tokens: {}, bytes/token = {}",
        token_set.ntokens(),
        stats.bytes_per_token()
    );

    let mut removal_count = HashMap::new();

    loop {
        if let Some(new_token_set) = optimization_step(
            &token_set,
            ntokens,
            bytes_optimizer,
            tokenizer_cache,
            &mut removal_count,
        ) {
            token_set = new_token_set;
        } else {
            break;
        }
    }

    token_set.sort();
    tokenizer_cache.get_stats(&token_set).clone()
}

pub fn optimize_tokenset<'a, S: Sampler<'a>>(
    ntokens: usize,
    sampler: &'a S,
    processing: Processing,
    token_type: TokenType,
    initial_size: Option<u64>,
) -> TokenStats {
    let bytes_optimizer = SimpleBytesOptimizer {};
    let mut tokenizer_cache = TokenizerCache::new(sampler, initial_size);

    match token_type {
        TokenType::Bits1 => {
            let token_set = TokenSet::new_bits1(processing, true);
            optimize_tokenset_impl(token_set, ntokens, &bytes_optimizer, &mut tokenizer_cache)
        }
        TokenType::Bits2 => {
            let token_set = TokenSet::new_bits2(processing, true);
            optimize_tokenset_impl(token_set, ntokens, &bytes_optimizer, &mut tokenizer_cache)
        }
        TokenType::Bits4 => {
            let token_set = TokenSet::new_bits4(processing, true);
            optimize_tokenset_impl(token_set, ntokens, &bytes_optimizer, &mut tokenizer_cache)
        }
        TokenType::Bytes => {
            let token_set = TokenSet::new_bits4(processing, true);
            let noop_bytes_optimizer = NoopBytesOptimizer {};
            optimize_tokenset_impl(
                token_set,
                ntokens,
                &noop_bytes_optimizer,
                &mut tokenizer_cache,
            )
        }
        TokenType::BytesHuff => {
            let token_set = TokenSet::new_bytes(processing);
            let stats = tokenizer_cache.get_stats(&token_set);
            let token_set = HuffOptimizer::optimize_bytes(&stats, ntokens);
            // token_set.sort();
            // tokenizer_cache.get_stats(&token_set)
            let bytes_optimizer = HuffOptimizer {};
            optimize_tokenset_impl(token_set, ntokens, &bytes_optimizer, &mut tokenizer_cache)
        }
    }
}
