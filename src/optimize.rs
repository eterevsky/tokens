use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::batch_tokenize::{TokenizerCache, tokenize_file};
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

fn show_tokenset_diff(before: &TokenSet, after: &TokenSet) -> String {
    let mut before_set = HashSet::new();
    let mut after_set = HashSet::new();

    for token in before.tokens.iter() {
        before_set.insert(token.clone());
    }
    for token in after.tokens.iter() {
        after_set.insert(token.clone());
    }

    let mut removed = before_set
        .difference(&after_set)
        .map(|t| t.to_string())
        .collect::<Vec<_>>();
    removed.sort();
    let mut added = after_set
        .difference(&before_set)
        .map(|t| t.to_string())
        .collect::<Vec<_>>();
    added.sort();

    format!("{} -> {}", removed.join(" "), added.join(" "))
}

fn add_token_bpe(stats: &TokenStats) -> Option<(TokenSet, i64)> {
    let mut pairs = (0..stats.pair_counts.len())
        .filter(|&i| stats.pair_counts[i] > 0)
        .collect::<Vec<usize>>();
    pairs.sort_by_key(|&i| -(stats.pair_counts[i] as i64));

    let mut new_token = Vec::new();
    let mut token_count = 0;

    for &pair_idx in pairs.iter() {
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
            token_count = stats.pair_counts[pair_idx];
            break;
        }
    }

    if new_token.is_empty() {
        dbg!(&stats.token_set.tokens);
        dbg!(&stats.token_counts);
        dbg!(&stats.pair_counts);

        return None;
    }

    let mut new_tokenset = stats.token_set.clone();
    new_tokenset.add_token(&new_token);

    Some((new_tokenset, token_count as i64))
}

fn count_tokens_in_bytes(tokenset: &TokenSet, stats: &TokenStats) -> u64 {
    let mut byte_cost: [Option<u64>; 256] = [None; 256];

    for token in tokenset.tokens.iter() {
        if let Token::Str(s) = token {
            if s.len() == 1 {
                let byte = s[0];
                assert!(byte_cost[byte as usize].is_none());
                byte_cost[byte as usize] = Some(1);
            }
        }
    }

    for seq in tokenset.sequences.iter() {
        if seq.string.len() == 1 {
            let byte = seq.string[0];
            assert!(byte_cost[byte as usize].is_none());
            byte_cost[byte as usize] = Some(seq.tokens.len() as u64);
        }
    }

    let mut total = 0;

    for (i, token) in stats.token_set.tokens.iter().enumerate() {
        if let Token::Str(s) = token {
            if s.len() == 1 {
                let byte = s[0];
                total += byte_cost[byte as usize].unwrap() * stats.token_counts[i];
            }
        }
    }

    for (i, seq) in stats.token_set.sequences.iter().enumerate() {
        if seq.string.len() == 1 {
            let byte = seq.string[0];
            total += byte_cost[byte as usize].unwrap() * stats.seq_counts[i];
        }
    }

    total
}

fn add_byte<BO: BytesOptimizer>(
    stats: &TokenStats,
    _bytes_optimizer: &BO,
) -> Option<(TokenSet, i64)> {
    let old_count = count_tokens_in_bytes(&stats.token_set, stats);
    let new_tokenset = BO::optimize_bytes(
        &stats,
        stats.token_set.ntokens() - stats.token_set.n_long_tokens() + 1,
    );

    if new_tokenset.ntokens() == stats.token_set.ntokens() {
        return None;
    }

    let new_count = count_tokens_in_bytes(&new_tokenset, stats);
    Some((new_tokenset, old_count as i64 - new_count as i64))
}

fn add_token<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    tokenset: &TokenSet,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
) -> Option<TokenSet> {
    let stats = tokenizer_cache.get_stats_with_pairs(tokenset);

    let maybe_tokenset_byte = add_byte(&stats, bytes_optimizer);
    let maybe_tokenset_token = add_token_bpe(&stats);

    match (maybe_tokenset_byte, maybe_tokenset_token) {
        (None, None) => None,
        (Some((new_token_set, _)), None) => Some(new_token_set),
        (None, Some((new_token_set, _))) => Some(new_token_set),
        (Some((token_set_byte, byte_count)), Some((token_set_token, token_count))) => {
            if byte_count > token_count {
                Some(token_set_byte)
            } else {
                Some(token_set_token)
            }
        }
    }
}

fn remove_add_token<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    token_set: &TokenSet,
    ntokens: usize,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
    removal_count: &mut HashMap<Vec<u8>, usize>,
) -> Option<TokenStats> {
    if token_set.ntokens() < ntokens {
        if let Some(new_tokenset) = add_token(token_set, bytes_optimizer, tokenizer_cache) {
            let stats = tokenizer_cache.get_stats(&new_tokenset);
            println!("{}", show_tokenset_diff(token_set, &new_tokenset));
            println!("processed bytes / token: {}", stats.bytes_per_token());
            return Some(stats);
        } else {
            return None;
        }
    }

    assert_eq!(token_set.ntokens(), ntokens);
    let stats = tokenizer_cache.get_stats(token_set);

    if token_set.ntokens() - token_set.n_long_tokens() > token_set.min_bytes_ext_tokens() {
        let new_token_set =
            BO::optimize_bytes(&stats, token_set.ntokens() - token_set.n_long_tokens() - 1);
        assert!(new_token_set.ntokens() == ntokens - 1);
        let new_stats = tokenizer_cache.get_stats_with_pairs(&new_token_set);
        if let Some((new_token_set, _)) = add_token_bpe(&new_stats) {
            let new_stats = tokenizer_cache.get_stats(&new_token_set);
            if new_stats.total_tokens < stats.total_tokens {
                println!("{}", show_tokenset_diff(token_set, &new_token_set));
                println!("processed bytes / token: {}", new_stats.bytes_per_token());
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

        if let Some(newer_tokenset) = add_token(&new_token_set, bytes_optimizer, tokenizer_cache) {
            let newer_stats = tokenizer_cache.get_stats(&newer_tokenset);
            if newer_stats.total_tokens < stats.total_tokens {
                println!();
                println!("{}", show_tokenset_diff(token_set, &newer_tokenset));
                println!("processed bytes / token: {}", newer_stats.bytes_per_token());
                return Some(newer_stats);
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
        println!("{}", show_tokenset_diff(token_set, &new_token_set));
        println!("processed bytes / token: {}", new_stats.bytes_per_token());

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

fn save_tokens(token_set: &TokenSet, tokens_dir: &Path) {
    let output_path = tokens_dir.join(format!("{}.json", token_set.name()));
    println!("Writing the token set to {}.", output_path.display());
    let serialized = serde_json::to_string(&token_set.to_json()).unwrap();
    std::fs::write(&output_path, serialized).unwrap();
}

fn optimize_tokenset_impl<'a, S: Sampler<'a>, BO: BytesOptimizer>(
    mut token_set: TokenSet,
    ntokens: usize,
    bytes_optimizer: &BO,
    tokenizer_cache: &mut TokenizerCache<'a, S>,
    tokens_dir: &Path,
) -> TokenStats {
    let stats = tokenizer_cache.get_stats(&token_set);
    println!(
        "Initial tokens: {}, bytes/token = {}",
        token_set.ntokens(),
        stats.bytes_per_token()
    );

    let mut removal_count = HashMap::new();
    let mut last_save = Instant::now();

    loop {
        if let Some(new_token_set) = optimization_step(
            &token_set,
            ntokens,
            bytes_optimizer,
            tokenizer_cache,
            &mut removal_count,
        ) {
            token_set = new_token_set;
            if Instant::now() - last_save > Duration::from_secs(60) {
                save_tokens(&token_set, tokens_dir);
                last_save = Instant::now();
            }
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
    pretrained_token_set: Option<TokenSet>,
    tokens_dir: &Path,
) -> TokenStats {
    let mut tokenizer_cache = TokenizerCache::new(sampler, initial_size);

    let token_set = match (pretrained_token_set, token_type) {
        (Some(ts), _) => ts,
        (None, TokenType::Bits1) => TokenSet::new_bits1(processing, true),
        (None, TokenType::Bits2) => TokenSet::new_bits2(processing, true),
        (None, TokenType::Bits4) => TokenSet::new_bits4(processing, true),
        (None, TokenType::Bytes) => TokenSet::new_bytes(processing),
        (None, TokenType::BytesHuff) => {
            let token_set = TokenSet::new_bytes(processing);
            let stats = tokenizer_cache.get_stats(&token_set);
            HuffOptimizer::optimize_bytes(&stats, ntokens)
        }
    };

    match token_type {
        TokenType::Bits1 | TokenType::Bits2 | TokenType::Bits4 => {
            let bytes_optimizer = SimpleBytesOptimizer {};
            optimize_tokenset_impl(
                token_set,
                ntokens,
                &bytes_optimizer,
                &mut tokenizer_cache,
                tokens_dir,
            )
        }
        TokenType::Bytes => {
            let noop_bytes_optimizer = NoopBytesOptimizer {};
            optimize_tokenset_impl(
                token_set,
                ntokens,
                &noop_bytes_optimizer,
                &mut tokenizer_cache,
                tokens_dir,
            )
        }
        TokenType::BytesHuff => {
            let bytes_optimizer = HuffOptimizer {};
            optimize_tokenset_impl(
                token_set,
                ntokens,
                &bytes_optimizer,
                &mut tokenizer_cache,
                tokens_dir,
            )
        }
    }
}

pub struct Optimizer {
    ntokens: usize,
    processing: Processing,
    token_type: TokenType,
    unprocessed_data_size: Option<u64>,
    tokens_dir: Box<Path>,
}

impl Optimizer {
    pub fn new(
        ntokens: usize,
        processing: Processing,
        token_type: TokenType,
        unprocessed_data_size: Option<u64>,
        tokens_dir: &Path,
     ) -> Self {
        Self {
            ntokens,
            processing,
            token_type,
            unprocessed_data_size,
            tokens_dir: tokens_dir.into(),
        }
    }

    pub fn optimize<'a>(&self, sampler: &'a impl Sampler<'a>, pretrained_token_set: Option<TokenSet>) -> TokenStats {
        optimize_tokenset(
            self.ntokens,
            sampler,
            self.processing,
            self.token_type,
            self.unprocessed_data_size,
            pretrained_token_set,
            &self.tokens_dir,
        )
    }

    pub fn get_stats<'a>(&self, sampler:  &'a impl Sampler<'a>, tokenset: &TokenSet) -> TokenStats {
        tokenize_file(tokenset, sampler, self.unprocessed_data_size)
    }
}
