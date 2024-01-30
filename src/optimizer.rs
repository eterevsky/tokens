use std::cmp::{min, Reverse};
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::input::sample::Sampler;
use crate::stats::TokenStats;
use crate::tokenizer::tokenize_file;
use crate::tokens::{LiteralEncoding, TokenSet};

fn format_token(s: &[u8]) -> String {
    match String::from_utf8(s.to_vec()) {
        Ok(string) => format!("{:?}", string),
        Err(_) => format!("{:?}", s),
    }
}

struct TokenizerCache<'a, S: Sampler<'a>> {
    sampler: &'a S,
    cache: HashMap<Vec<u8>, TokenStats>,
}

impl<'a, S: Sampler<'a>> TokenizerCache<'a, S> {
    fn new(sampler: &'a S) -> Self {
        TokenizerCache {
            sampler,
            cache: HashMap::new(),
        }
    }

    fn get_stats_with_pairs(&mut self, token_set: &TokenSet) -> TokenStats {
        let mut tokens = token_set
            .tokens
            .iter()
            .map(|t| t.string.clone())
            .collect::<Vec<_>>();
        tokens.sort_unstable();

        let mut key = Vec::new();
        for token in tokens {
            key.extend(token);
            key.push(0);
        }

        let stats = tokenize_file(token_set, self.sampler, true);
        let mut stats_clone = stats.clone();
        stats_clone.pair_count.clear();
        stats_clone.pair_count.shrink_to_fit();
        self.cache.insert(key, stats_clone);

        stats
    }

    fn get_stats(&mut self, token_set: &TokenSet) -> TokenStats {
        let mut tokens = token_set
            .tokens
            .iter()
            .map(|t| t.string.clone())
            .collect::<Vec<_>>();
        tokens.sort_unstable();

        let mut key = Vec::new();
        for token in tokens {
            key.extend(token);
            key.push(0);
        }

        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }

        let stats = tokenize_file(token_set, self.sampler, false);
        let mut stats_clone = stats.clone();
        stats_clone.pair_count.clear();
        stats_clone.pair_count.shrink_to_fit();
        self.cache.insert(key, stats_clone);

        stats
    }

    fn total(&self) -> usize {
        self.cache.len()
    }
}

fn add_tokens<'a, S: Sampler<'a>>(
    tokenizer: &mut TokenizerCache<'a, S>,
    token_set: &mut TokenSet,
    tokens_to_add: usize,
) -> Vec<Vec<u8>> {
    let stats = tokenizer.get_stats_with_pairs(token_set);

    let mut token_values = Vec::new();

    for i in 0..256 {
        if stats.literal_count[i] > 0 {
            token_values.push((
                vec![i as u8],
                stats.literal_count[i] * (token_set.literal_cost() - 1),
            ))
        }
    }

    for (&key, &count) in stats.pair_count.iter() {
        let ifirst = key >> 16;
        let isecond = key & 0xFFFF;
        // for (&(ifirst, isecond), &count) in stats.pair_count.iter() {
        if count > 0 {
            let mut token_str = token_set.tokens[ifirst as usize].string.clone();
            token_str.extend(token_set.tokens[isecond as usize].string.clone());
            token_values.push((token_str, count))
        }
    }

    token_values.sort_unstable_by_key(|&(_, value)| -(value as i64));

    let mut added = Vec::new();

    for (token_str, _) in token_values.iter().take(tokens_to_add) {
        token_set.add_token(token_str.as_slice());
        added.push(token_str.clone());
    }

    added
}

fn add_token<'a, S: Sampler<'a>>(
    tokenizer: &mut TokenizerCache<'a, S>,
    token_set: &mut TokenSet,
) -> Vec<u8> {
    // let start = Instant::now();
    let stats = tokenizer.get_stats_with_pairs(token_set);
    // print!(" get_stats_with_pairs: {} ", start.elapsed().as_millis());

    let mut added = None;
    let mut best_cost = 0;

    for i in 0..256 {
        if stats.literal_count[i] > best_cost {
            added = Some(vec![i as u8]);
            best_cost = stats.literal_count[i];
        }
    }

    // let set_size = token_set.tokens.len();

    let mut pair_values: HashMap<Vec<u8>, u64> = HashMap::new();

    // for (&(ifirst, isecond), &count) in stats.pair_count.iter() {
    for (&key, &count) in stats.pair_count.iter() {
        let ifirst = key >> 16;
        let isecond = key & 0xFFFF;
        if count > 0 {
            let mut token_str = token_set.tokens[ifirst as usize].string.clone();
            token_str.extend(token_set.tokens[isecond as usize].string.clone());

            let total_count = pair_values.entry(token_str.clone()).or_insert(0);
            *total_count += count;
            if *total_count > best_cost {
                added = Some(token_str);
                best_cost = *total_count;
            }
        }
    }

    added.unwrap()
}

fn _add_and_remove_token<'a, S1: Sampler<'a>, S2: Sampler<'a>>(
    tokenizer: &mut TokenizerCache<'a, S1>,
    fast_tokenizer: &mut TokenizerCache<'a, S2>,
    token_set: &TokenSet,
) -> Option<TokenSet> {
    let mut token_set = token_set.clone();
    let initial_stats = tokenizer.get_stats(&token_set);

    add_tokens(tokenizer, &mut token_set, 1);

    let stats_before = tokenizer.get_stats(&token_set);
    let fast_stats_before = fast_tokenizer.get_stats(&token_set);

    let mut token_ids: Vec<usize> = (0..token_set.tokens.len()).collect();
    token_ids.sort_unstable_by_key(|&i| stats_before.token_count[i]);

    // We construct a list of tokens to remove because the ids will change when
    // we are removing and adding tokens.
    let mut token_strs: Vec<Vec<u8>> = Vec::new();

    for &token_id_to_remove in token_ids.iter() {
        let token_to_remove = &token_set.tokens[token_id_to_remove];
        if token_to_remove.is_mandatory {
            continue;
        }
        token_strs.push(token_to_remove.string.clone());
    }

    let add_token_limit = initial_stats.cost() - stats_before.cost();
    // Check if removing a token adds > 3/2 the cost that we can afford on a sparser sampler.
    let fast_add_token_limit =
        add_token_limit * fast_stats_before.scanned_bytes * 3 / (stats_before.scanned_bytes * 2);
    let fast_cost_before = fast_stats_before.cost();

    println!(
        "Can add up to {:.4}% cost by removing a token.",
        100.0 * add_token_limit as f64 / stats_before.cost() as f64
    );

    let mut tries = 0;

    for token_str in token_strs.iter() {
        let token_str = token_str.as_slice();
        tries += 1;

        token_set.remove_token(token_str);
        let fast_stats = fast_tokenizer.get_stats(&token_set);

        if fast_stats.cost() - fast_cost_before > fast_add_token_limit {
            println!("Not checking whether we can remove {} since removing it adds {:.4}% tokens on the smaller sample.",
                     format_token(token_str), 100.0 * (fast_stats.cost() - fast_cost_before) as f64 / fast_cost_before as f64);
            continue;
        }

        let stats = tokenizer.get_stats(&token_set);
        // token_set.update_stats(&stats);

        // if stats.cost < initial_stats.cost
        //     && stats.token_to_add(&new_token_set) == token_str
        // {
        //     println!("Cost after removing {} would be {}, but it would be added on the next iteration.", format_token(&token_str), stats.cost);
        // }

        if stats.cost() < initial_stats.cost() {
            // Found a token to remove.
            println!(
                "removing {} after {} tries",
                format_token(&token_str),
                tries
            );
            return Some(token_set);
        }

        token_set.add_token(&token_str);
    }

    None
}

fn remove_and_add_token<'a, S1: Sampler<'a>, S2: Sampler<'a>>(
    tokenizer: &mut TokenizerCache<'a, S1>,
    fast_tokenizer: &mut TokenizerCache<'a, S2>,
    token_set: &TokenSet,
    token_attempts: &mut HashMap<Vec<u8>, usize>,
) -> Option<TokenSet> {
    let initial_stats = tokenizer.get_stats(token_set);
    // let initial_cost = (initial_stats.cost() as u128 * fast_tokenizer.sampler.total_size() as u128
    //     / tokenizer.sampler.total_size() as u128) as u64;

    let mut token_ids: Vec<usize> = (0..token_set.tokens.len()).collect();
    // let mut rng = thread_rng();
    // let uniform = Uniform::new(0, 65536);

    // let mut order_shuffle = Vec::new();
    // for _ in 0..token_set.tokens.len() {
    //     order_shuffle.push(uniform.sample(&mut rng))
    // }

    token_ids.sort_unstable_by_key(|&i| {
        initial_stats.token_count[i] as u64
            + ((*token_attempts
                .get(&token_set.tokens[i].string)
                .unwrap_or(&0) as u64)
                << 32)
    });

    // We construct a list of tokens to remove because the ids will change when
    // we are removing and adding tokens.
    let mut token_strs: Vec<Vec<u8>> = Vec::new();

    for &token_id_to_remove in token_ids.iter() {
        let token_to_remove = &token_set.tokens[token_id_to_remove];
        if token_to_remove.is_mandatory {
            continue;
        }
        // println!("{} {}", format_token(&token_to_remove.string), initial_stats.token_count[token_id_to_remove]);
        token_strs.push(token_to_remove.string.clone());
    }

    let mut tries = 0;

    // dbg!(initial_stats.cost());

    print!("Trying to remove:");
    io::stdout().flush().unwrap();

    // Try no more than the first 512 tokens.
    for token_str in token_strs.iter() {
        let count = token_attempts.entry(token_str.clone()).or_insert(0);

        if tries >= 512 && *count > 0 {
            println!("\nAttempted to remove each token at least once + tried removing {} tokens without success. Giving up.", tries);
            return None;
        }

        tries += 1;
        *count += 1;

        print!(" {} ", format_token(token_str));
        io::stdout().flush().unwrap();

        let token_str = token_str.as_slice();

        let mut new_token_set = token_set.clone();
        new_token_set.remove_token(token_str);

        // let start = Instant::now();
        let added_str = add_token(fast_tokenizer, &mut new_token_set);
        // dbg!(format_token(added_str.as_slice()));
        // print!(" add_token: {} ", start.elapsed().as_millis());

        if added_str == token_str {
            continue;
        }

        new_token_set.add_token(added_str.as_slice());

        // let new_stats = fast_tokenizer.get_stats(&new_token_set);

        // if new_stats.cost() < initial_cost {
        // let start = Instant::now();
        let new_full_stats = tokenizer.get_stats(&new_token_set);
        // dbg!(new_full_stats.cost());

        // print!(" mid get_stats: {} ", start.elapsed().as_millis());
        if new_full_stats.cost() < initial_stats.cost() {
            println!(
                "\nReplacing {} -> {} after {} tries",
                format_token(&token_str),
                format_token(&added_str),
                tries
            );
            return Some(new_token_set);
        }
        // }
        // break;
    }

    println!("\nNo token to replace after {} tries", tries);

    None
}

fn add_tokens_bpe<'a, S: Sampler<'a>>(
    tokenizer: &mut TokenizerCache<'a, S>,
    token_set: &mut TokenSet,
    ntokens: usize,
    add_block: usize,
) {
    while token_set.ntokens() < ntokens {
        let tokens_to_add = min(add_block, ntokens - token_set.ntokens());
        let added = add_tokens(tokenizer, token_set, tokens_to_add);
        for token_str in added.iter() {
            println!("Added {}", format_token(token_str.as_slice()));
        }
        let stats = tokenizer.get_stats(&token_set);
        println!(
            "{} tokens, bytes/cost = {:.3}  literals/bytes = {:.5}",
            token_set.ntokens(),
            stats.scanned_bytes as f64 / stats.cost() as f64,
            stats.total_literals() as f64 / stats.scanned_bytes as f64,
        );
    }
}

pub fn optimize_bpe<'a, S: Sampler<'a>, FS: Sampler<'a>>(
    token_set: &TokenSet,
    ntokens: usize,
    sampler: &'a S,
    fast_sampler: &'a FS,
    add_block: usize,
) -> (TokenSet, TokenStats) {
    let mut token_set = token_set.clone();
    let mut tokenizer = TokenizerCache::new(sampler);
    let mut fast_tokenizer = TokenizerCache::new(fast_sampler);

    add_tokens_bpe(&mut tokenizer, &mut token_set, ntokens, add_block);

    let mut token_attemps = HashMap::new();

    loop {
        let stats = tokenizer.get_stats(&token_set);
        println!(
            "{} tokens, bytes/cost = {:.3}  literals/bytes = {:.5}",
            token_set.ntokens(),
            stats.scanned_bytes as f64 / stats.cost() as f64,
            stats.total_literals() as f64 / stats.scanned_bytes as f64,
        );
        // if let Some(new_token_set) =
        //     add_and_remove_token(&mut tokenizer, &mut fast_tokenizer, &token_set)
        // {
        //     token_set = new_token_set;
        //     continue;
        // }
        if let Some(new_token_set) = remove_and_add_token(
            &mut tokenizer,
            &mut fast_tokenizer,
            &token_set,
            &mut token_attemps,
        ) {
            token_set = new_token_set;
            continue;
        } else {
            break;
        }
    }

    println!("Number of tokenizations: {}", tokenizer.total());

    let stats = tokenizer.get_stats(&token_set);
    (token_set, stats)
}

/// Optimize the set of tokens consisting of one byte. Collect all single-byte
/// tokens and literals make sure that the number of usages of tokens is
/// strictly higher than that of literals. If this is not the case, turn
/// the most common literals into a tokens and vice versa.
///
/// Returns true if the token set was modified.
fn optimize_byte_tokens(token_set: &mut TokenSet, stats: &TokenStats) -> bool {
    let mut byte_count = [0u64; 256];
    let mut one_byte_token_count = 0;
    let mut is_token = [false; 256];
    for (i, token) in token_set.tokens.iter().enumerate() {
        if token.string.len() == 1 {
            byte_count[token.string[0] as usize] += stats.token_count[i];
            one_byte_token_count += 1;
            is_token[token.string[0] as usize] = true;
        }
    }

    for i in 0..256 {
        if stats.literal_count[i] > 0 {
            assert!(byte_count[i] == 0);
            byte_count[i] = stats.literal_count[i];
        }
    }

    let mut ids = (0..256).collect::<Vec<_>>();
    ids.sort_unstable_by_key(|&i| Reverse(byte_count[i]));

    let mut to_add = Vec::new();
    let mut to_remove = Vec::new();
    for i in 0..256 {
        if i < one_byte_token_count {
            if !is_token[ids[i]] {
                to_add.push(ids[i]);
            }
        } else {
            if is_token[ids[i]] {
                to_remove.push(ids[i]);
            }
        }
    }

    if to_add.is_empty() {
        assert!(to_remove.is_empty());
        return false;
    }

    assert!(!to_remove.is_empty());

    print!("Removing tokens:");
    for &i in &to_remove {
        print!(" {}", format_token(&[i as u8]));
        token_set.remove_token(&[i as u8]);
    }

    print!("\nAdding tokens:");
    for &i in &to_add {
        print!(" {}", format_token(&[i as u8]));
        token_set.add_token(&[i as u8]);
    }
    println!();

    true
}

fn save_token_set(
    token_set: &TokenSet,
    stats: &TokenStats,
    output_path: &Path,
    processing: &str,
    initial_size: u64,
) {
    let mut tokens_json = token_set.to_json(stats, initial_size);
    tokens_json["processing"] = processing.into();

    println!("Writing to {}", output_path.display());

    std::fs::write(&output_path, json::stringify_pretty(tokens_json, 2)).unwrap();
}

fn optimize_token_set<'a, S1: Sampler<'a>, S2: Sampler<'a>, S3: Sampler<'a>>(
    initial_size: u64,
    mut token_set: TokenSet,
    slow_sampler: &'a S1,
    sampler: &'a S2,
    fast_sampler: &'a S3,
    ntokens: usize,
    processing: &str,
    block: usize,
    output_path: &Path,
) {
    let mut tokenizer = TokenizerCache::new(sampler);
    let mut fast_tokenizer = TokenizerCache::new(fast_sampler);

    // dbg!(token_set.ntokens());

    if token_set.ntokens() < ntokens {
        add_tokens_bpe(&mut tokenizer, &mut token_set, ntokens, block);
    }

    let mut stats = tokenize_file(&token_set, slow_sampler, false);
    // for i in 0..token_set.tokens.len() {
    //     println!(
    //         "{}: {}",
    //         format_token(&token_set.tokens[i].string),
    //         stats.token_count[i]
    //     );
    // }

    let cost = stats.cost();
    if optimize_byte_tokens(&mut token_set, &stats) {
        stats = tokenize_file(&token_set, slow_sampler, false);
        assert!(stats.cost() <= cost);
    }
    let mut best_cost = stats.cost();

    save_token_set(&token_set, &stats, output_path, processing, initial_size);

    println!(
        "Initial stats: bytes/cost = {:.4}  literals/bytes = {:.5}",
        initial_size as f64 / stats.cost() as f64,
        stats.total_literals() as f64 / initial_size as f64,
    );

    let mut last_update_time = Instant::now();
    // Number of times each token was attempted to remove
    let mut token_attempts = HashMap::new();

    while let Some(new_token_set) = remove_and_add_token(
        &mut tokenizer,
        &mut fast_tokenizer,
        &token_set,
        &mut token_attempts,
    ) {
        token_set = new_token_set;

        if Instant::now() - last_update_time > Duration::from_secs(600) {
            let mut stats = tokenize_file(&token_set, slow_sampler, false);
            if optimize_byte_tokens(&mut token_set, &stats) {
                stats = tokenize_file(&token_set, slow_sampler, false);
            }
            let cost = stats.cost();
            if cost < best_cost {
                println!(
                    "Stats: bytes/cost = {:.4}  literals/bytes = {:.5}",
                    initial_size as f64 / stats.cost() as f64,
                    stats.total_literals() as f64 / initial_size as f64,
                );
                save_token_set(&token_set, &stats, output_path, processing, initial_size);
                best_cost = cost;
            } else {
                println!("Cost increased, not saving");
            }
            last_update_time = Instant::now();
        }
    }

    let mut stats = tokenize_file(&token_set, slow_sampler, false);
    let cost = stats.cost();
    if optimize_byte_tokens(&mut token_set, &stats) {
        stats = tokenize_file(&token_set, slow_sampler, false);
        assert!(stats.cost() < cost);
    }
    let cost = stats.cost();
    if cost <= best_cost {
        println!(
            "Stats: bytes/cost = {:.3}  literals/bytes = {:.5}",
            stats.scanned_bytes as f64 / stats.cost() as f64,
            stats.total_literals() as f64 / stats.scanned_bytes as f64,
        );
        save_token_set(&token_set, &stats, output_path, processing, initial_size);
    } else {
        println!("Cost increased, not saving");
    }
}

fn load_prev_token_set(
    tokens_dir: &str,
    ntokens: usize,
    processing: &str,
    literal_encoding: LiteralEncoding,
) -> Option<TokenSet> {
    let tokens_filename = format!(
        "{}/tokens{}_{}_{}.json",
        tokens_dir, ntokens, processing, literal_encoding
    );
    if Path::new(&tokens_filename).exists() {
        println!("Loading pre-trained token set from {}", tokens_filename);
        Some(TokenSet::from_json(&tokens_filename))
    } else {
        if ntokens > 2 {
            load_prev_token_set(tokens_dir, ntokens / 2, processing, literal_encoding)
        } else {
            None
        }
    }
}

pub fn optimize_all<'a, S1: Sampler<'a>, S2: Sampler<'a>, S3: Sampler<'a>>(
    initial_size: u64,
    slow_sampler: &'a S1,
    sampler: &'a S2,
    fast_sampler: &'a S3,
    tokens_dir: &str,
    min_tokens: usize,
    max_tokens: usize,
    processing: &str,
) {
    let tokens_dir_path = std::path::Path::new(tokens_dir);

    let mut ntokens = min_tokens;
    while ntokens <= max_tokens {
        let mut block = ntokens / 2;
        while block * block >= ntokens {
            block /= 2;
        }

        for &literal_encoding in &[
            // LiteralEncoding::Bits1,
            // LiteralEncoding::Bits2,
            // LiteralEncoding::Bits4,
            // LiteralEncoding::All,
            // LiteralEncoding::Dist4,
            LiteralEncoding::Dist8,
        ] {
            if literal_encoding.reserved_tokens() > ntokens
                || (literal_encoding == LiteralEncoding::Bits1 && ntokens >= 128)
                || (literal_encoding == LiteralEncoding::Bits2 && ntokens >= 512)
                || (literal_encoding == LiteralEncoding::All && ntokens < 256)
                || (processing == "raw" && ntokens >= 1024)
            {
                continue;
            }

            println!(
                "Optimizing tokens for processing '{}', literals {}, ntokens {}, block {}",
                processing, literal_encoding, ntokens, block
            );

            let token_set = if let Some(prev_token_set) =
                load_prev_token_set(tokens_dir, ntokens, processing, literal_encoding)
            {
                prev_token_set
            } else {
                TokenSet::new(literal_encoding)
            };

            let output_filename =
                format!("tokens{}_{}_{}.json", ntokens, processing, literal_encoding);
            let output_path = tokens_dir_path.join(output_filename);

            optimize_token_set(
                initial_size,
                token_set,
                slow_sampler,
                sampler,
                fast_sampler,
                ntokens,
                processing,
                block,
                &output_path,
            );
        }

        ntokens *= 2;
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::input::memory_sampler::MemorySampler;

//     use super::*;

//     #[test]
//     fn test_remove_and_add() {
//         let mut sampler = MemorySampler::new_from_str("abc, abc", 4);
//         let mut tokenizer = TokenizerCache::new(&sampler);
//         let mut fast_tokenizer = TokenizerCache::new(&sampler);
//         let mut token_attempts = HashMap::new();
//         let mut token_set = TokenSet::new(LiteralEncoding::Dist8);

//         remove_and_add_token(
//             &mut tokenizer,
//             &mut fast_tokenizer,
//             &token_set,
//             &mut token_attempts,
//         );
//     }
// }
