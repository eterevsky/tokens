use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::input::sample::{Sample, Sampler};
use super::stats2::TokenStats;
use super::tokenizer2::FragmentTokenizer;
use super::tokenset::TokenSet;

pub fn tokenize_file_sync<'a, S: Sampler<'a>>(
    token_set: &TokenSet,
    sampler: &'a S,
    initial_size: Option<u64>,
) -> TokenStats {
    let tokenizer = FragmentTokenizer::new(token_set.clone());
    let mut stats = TokenStats::new(token_set.clone(), initial_size);

    let mut buffer = Vec::new();

    for sample in sampler.iter() {
        tokenizer.process_slice(sample.as_bytes(), &mut stats, &mut buffer);
    }

    stats
}

fn worker(
    tokenizer: &FragmentTokenizer,
    jobs_rx: Arc<Mutex<Receiver<Sample>>>,
    results_tx: Sender<TokenStats>,
) {
    let mut stats = TokenStats::new(tokenizer.token_set.clone(), None);
    let mut buffer = Vec::new();

    loop {
        let job = jobs_rx.lock().unwrap().recv();
        let data = {
            match job {
                Ok(ref sample) => sample.as_bytes(),
                Err(_) => break,
            }
        };

        assert!(!data.is_empty());
        tokenizer.process_slice(data, &mut stats, &mut buffer);
    }

    results_tx.send(stats).unwrap();
}

pub fn tokenize_file<'a, S: Sampler<'a>>(
    token_set: &TokenSet,
    sampler: &'a S,
    initial_size: Option<u64>,
) -> TokenStats {
    if sampler.total_size() < 1 << 25 {
        return tokenize_file_sync(token_set, sampler, initial_size);
    }

    let tokenizer = FragmentTokenizer::new(token_set.clone());
    let mut stats = TokenStats::new(token_set.clone(), initial_size);
    let nthreads = std::thread::available_parallelism().unwrap().get();

    let (jobs_tx, jobs_rx) = mpsc::sync_channel::<Sample>(4);
    let jobs_rx_shared = Arc::new(Mutex::new(jobs_rx));
    let (results_tx, results_rx) = mpsc::channel::<TokenStats>();

    std::thread::scope(|s| {
        let mut join_handles = Vec::new();

        for _ in 0..nthreads {
            let jobs_rx_clone = jobs_rx_shared.clone();
            let results_tx_clone = results_tx.clone();
            join_handles.push(s.spawn(|| worker(&tokenizer, jobs_rx_clone, results_tx_clone)));
        }

        let start = Instant::now();

        for sample in sampler.iter() {
            jobs_tx.send(sample).unwrap();
        }

        std::mem::drop(jobs_tx);

        for _ in 0..nthreads {
            let result = results_rx.recv().unwrap();
            stats.merge(&result);
        }

        if stats.scanned_bytes > 1 << 34 {
            let elapsed = std::time::Instant::now() - start;
            println!(
                "\rAvg pace: {:.1} MB / s",
                stats.scanned_bytes as f64 / 1000000.0 / elapsed.as_secs_f64()
            );
        }

        while !join_handles.is_empty() {
            join_handles.pop().unwrap().join().unwrap();
        }
    });

    stats
}
