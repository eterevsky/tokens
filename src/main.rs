#![allow(dead_code)]

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tempfile::NamedTempFile;

mod batch_tokenize;
mod input;
mod optimize;
mod optimize_bytes;
mod processing;
mod stats2;
mod tokenizer2;
mod tokenset;

use self::input::file_sampler::FileSampler;
use self::input::memory_sampler::MemorySampler;
use self::processing::{process_file, Processing};
use self::stats2::TokenStats;
use self::tokenset::{TokenSet, TokenType};

fn maybe_process_file(
    filename_raw: &str,
    filename_processed: Option<&str>,
    processing: Processing,
) -> (String, Option<NamedTempFile>) {
    match (filename_processed, processing) {
        (_, Processing::Raw) => (filename_raw.to_string(), None),
        (Some(f), Processing::CapsWords) => (f.to_string(), None),
        (None, Processing::CapsWords) => {
            println!("Pre-processing the data file... ");
            let mut temp_processed = NamedTempFile::new().unwrap();
            let mut input = File::open(filename_raw).unwrap();
            process_file(&mut input, &mut temp_processed).unwrap();
            println!("done");
            let filename = temp_processed.path().to_str().unwrap().to_string();
            (filename, Some(temp_processed))
        }
    }
}

fn process(filename: &str, output: &str) {
    let mut input = File::open(filename).unwrap();
    let mut output = File::create(output).unwrap();

    process_file(&mut input, &mut output).unwrap();
}

fn count_chars(filename: &str) {
    let file = File::open(filename).unwrap();
    let mut reader = BufReader::new(file);

    let mut total = 0;
    let mut counts_low: [usize; 256] = [0; 256];
    let mut counts = HashMap::new();

    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => panic!("{}", e),
        }
        let chars_in_line = line.chars().count();
        let next_total = total + chars_in_line;
        if total / 1000000000 != next_total / 1000000000 {
            println!("{}", next_total);
        }
        total = next_total;

        for c in line.chars() {
            let c = c as u32;
            if c < 256 {
                counts_low[c as usize] += 1;
            } else {
                *counts.entry(c).or_insert(0) += 1;
            }
        }
    }
    println!("Total: {}", total);

    for (c, count) in counts_low.iter().enumerate() {
        if *count > 0 {
            println!("{:?} {}", std::char::from_u32(c as u32).unwrap(), count);
        }
    }

    let mut max_c = 0;
    for c in counts.keys() {
        if *c > max_c {
            max_c = *c;
        }
    }

    println!("Max char: {:?}", std::char::from_u32(max_c).unwrap());
}

fn read_token_set(filename: &str) -> TokenSet {
    let path = Path::new(filename);
    let input_tokens_file = File::open(path).expect("Input tokens file not found");
    let reader = BufReader::new(input_tokens_file);

    // Deserialize the JSON data into a serde_json::Value
    let tokenset_json: Value = serde_json::from_reader(reader).unwrap();
    TokenSet::from_json(tokenset_json)
}

fn load_save_tokens(
    filename_raw: &str,
    filename_processed: Option<&str>,
    input_tokens_path: &str,
    tokens_dir: &str,
) {
    let tokens_dir_path = Path::new(tokens_dir);
    let token_set = read_token_set(input_tokens_path);

    let (filename, _temp) =
        maybe_process_file(filename_raw, filename_processed, token_set.processing);
    let initial_size = std::fs::metadata(filename_raw).unwrap().len();

    println!("Opening {}", &filename);
    let sampler = FileSampler::new(&filename, 1 << 24, None);

    println!(
        "Tokenizing {} using token set {}.",
        &filename,
        token_set.name()
    );
    let stats = batch_tokenize::tokenize_file(&token_set, &sampler, Some(initial_size));

    let output_path = tokens_dir_path.join(format!("{}.json", token_set.name()));
    println!("Writing the token set to {}.", output_path.display());
    let serialized = serde_json::to_string(&stats.to_json()).unwrap();
    std::fs::write(&output_path, serialized).unwrap();
}

fn save_tokens(stats: &TokenStats, tokens_dir: &Path) {
    let output_path = tokens_dir.join(format!("{}.json", stats.token_set.name()));
    println!("Writing the token set to {}.", output_path.display());
    let serialized = serde_json::to_string(&stats.to_json()).unwrap();
    std::fs::write(&output_path, serialized).unwrap();
}

fn optimize_with_increasing_data(
    optimizer: &optimize::Optimizer,
    filename: &str,
    min_data_size: usize,
    input_token_set: Option<TokenSet>,
) -> TokenStats {
    let full_sampler = FileSampler::new(filename, 1 << 24, None);
    let full_size = std::fs::metadata(filename).unwrap().len() as usize;
    let mut tokenset = input_token_set;
    let mut size = min_data_size;
    let mut full_stats = None;

    while size < full_size {
        let sampler = MemorySampler::sample_from_file(filename, size, 1 << 20);
        println!("Optimizing with {} bytes of data.", sampler.total_size());

        if let Some(tokenset) = tokenset.as_ref() {
            let stats = optimizer.get_stats(&sampler, tokenset);
            println!("bytes / token (bigger data): {}", stats.bytes_per_token());
        }

        let stats = optimizer.optimize(&sampler, tokenset);
        println!("bytes / token (optimized): {}", stats.bytes_per_token());

        full_stats = optimizer.get_stats(&full_sampler, &stats.token_set);
        println!(
            "bytes / token (full data): {}",
            full_stats.bytes_per_token()
        );

        if size > 1 << 24
            && (stats.bytes_per_token() - full_stats.bytes_per_token()).abs()
                / full_stats.bytes_per_token()
                < 0.005
        {
            println!("The difference is less than 0.5%. Stopping.");
            tokenset = Some(stats.token_set);
            break;
        }

        tokenset = Some(stats.token_set);

        size *= 2;
    }

    full_stats
}

fn optimize(
    ntokens: usize,
    filename_raw: &str,
    filename_processed: Option<&str>,
    tokens_dir: &str,
    processing: Processing,
    token_type: TokenType,
    input_tokens: Option<&str>,
    min_data_size: Option<usize>,
) {
    let tokens_dir_path = Path::new(tokens_dir);

    let (filename, _temp) = maybe_process_file(filename_raw, filename_processed, processing);
    let initial_size = std::fs::metadata(filename_raw).unwrap().len();

    let input_token_set = if let Some(filename) = input_tokens {
        println!("Reading the input token set from {}.", filename);
        Some(read_token_set(filename))
    } else {
        None
    };

    println!(
        "Optimizing a token set with {} tokens from data in {}",
        ntokens, &filename
    );

    let optimizer = optimize::Optimizer::new(
        ntokens,
        processing,
        token_type,
        Some(initial_size),
        &tokens_dir_path,
    );

    let stats = if let Some(min_data_size) = min_data_size {
        optimize_with_increasing_data(&optimizer, &filename, min_data_size, input_token_set)
    } else if initial_size < 1 << 34 {
        optimizer.optimize(
            &MemorySampler::from_file(&filename, 1 << 20),
            input_token_set,
        )
    } else {
        optimizer.optimize(&FileSampler::new(&filename, 1 << 24, None), input_token_set)
    };

    save_tokens(&stats, tokens_dir_path);
}

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Process {
        #[arg(short, long)]
        data: String,

        #[arg(short, long)]
        output: String,
    },

    CountChars {
        #[arg(short, long)]
        data: String,
    },

    ConvertTokens {
        #[arg(short, long)]
        data: String,

        #[arg(long)]
        processed_data: Option<String>,

        #[arg(short, long)]
        input_tokens: String,

        #[arg(short, long)]
        tokens_dir: String,
    },

    Optimize {
        #[arg(short, long)]
        data: String,

        #[arg(long)]
        processed_data: Option<String>,

        #[arg(short, long)]
        tokens_dir: String,

        #[arg(short, long)]
        processing: Processing,

        #[arg(id = "type", long)]
        token_type: TokenType,

        #[arg(short, long)]
        ntokens: usize,

        #[arg(short, long)]
        input_tokens: Option<String>,

        /// The tokenset will initially be optimized using a smaller dataet
        /// extracted from the full data. This option specifies the initial
        /// size of the sample that will be extracted from the full data.
        #[arg(long)]
        min_data_size: Option<usize>,
    },
}

fn main() {
    let args = Args::parse();

    match &args.command {
        Command::ConvertTokens {
            data,
            processed_data,
            input_tokens,
            tokens_dir,
        } => load_save_tokens(data, processed_data.as_deref(), input_tokens, tokens_dir),

        Command::Optimize {
            data,
            processed_data,
            tokens_dir,
            processing,
            token_type,
            ntokens,
            input_tokens,
            min_data_size,
        } => optimize(
            *ntokens,
            data,
            processed_data.as_deref(),
            tokens_dir,
            *processing,
            *token_type,
            input_tokens.as_deref(),
            *min_data_size,
        ),

        Command::Process { data, output } => process(data.as_str(), output.as_str()),

        Command::CountChars { data } => count_chars(data.as_str()),
    }
}
