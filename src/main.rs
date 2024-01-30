#![allow(dead_code)]

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use tempfile::NamedTempFile;

mod batch_tokenize;
mod input;
mod optimize;
mod optimizer;
mod processing;
mod stats;
mod stats2;
mod tokenizer;
mod tokenizer2;
mod tokens;
mod tokenset;

use self::input::file_sampler::FileSampler;
use self::input::memory_sampler::MemorySampler;
use self::input::preloaded_sampler::PreloadedSampler;
use self::optimize::optimize_tokenset;
use self::optimizer::optimize_bpe;
use self::processing::{process_file, Processing};
use self::tokenizer::tokenize_file;
use self::tokens::{LiteralEncoding, TokenSet};
use self::tokenset::TokenType;

fn optimize_tokens(
    data_filename: &str,
    input_tokens: &Option<String>,
    output_tokens: &Option<String>,
    ntokens: usize,
    initial_size: Option<u64>,
    processing: &Option<String>,
    in_memory: bool,
    nchunks: Option<usize>,
    chunk_size: usize,
    add_block: usize,
    literal_encoding: LiteralEncoding,
) {
    println!(
        "Using {} threads",
        std::thread::available_parallelism().unwrap().get()
    );

    let token_set = if let Some(tokens_file) = input_tokens {
        TokenSet::from_json(tokens_file.as_str())
    } else {
        TokenSet::new(literal_encoding)
    };

    let file_size = std::fs::metadata(data_filename).unwrap().len();
    let initial_size = initial_size.unwrap_or(file_size);

    let fast_sampler = PreloadedSampler::new(data_filename, 16384, 1024);

    let (token_set, token_stats) = if nchunks.is_some() {
        let nchunks = nchunks.unwrap();
        let sampler = PreloadedSampler::new(data_filename, chunk_size, nchunks);
        let (token_set, _) = optimize_bpe(&token_set, ntokens, &sampler, &fast_sampler, add_block);

        let full_sampler = FileSampler::new(data_filename, chunk_size, None);
        let stats = tokenize_file(&token_set, &full_sampler, false);

        (token_set, stats)
    } else if in_memory {
        let sampler = MemorySampler::from_file(data_filename, chunk_size);
        optimize_bpe(&token_set, ntokens, &sampler, &fast_sampler, add_block)
    } else {
        let sampler = FileSampler::new(data_filename, chunk_size, None);
        optimize_bpe(&token_set, ntokens, &sampler, &fast_sampler, add_block)
    };

    let mut tokens_json = token_set.to_json(&token_stats, initial_size);

    let mut processing_json: Vec<json::JsonValue> = Vec::new();
    if let Some(processing) = processing {
        for stage in processing.split(",") {
            if !stage.is_empty() {
                processing_json.push(stage.into());
            }
        }
    }

    tokens_json["processing"] = processing_json.into();

    let tokens_json_str = json::stringify_pretty(tokens_json, 2);
    println!("{}", &tokens_json_str);

    if let Some(out) = output_tokens {
        std::fs::write(&out, &tokens_json_str).unwrap();
    }
}

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

fn optimize_all_for_proc(
    filename_raw: &str,
    filename_processed: Option<&str>,
    processing: Processing,
    tokens_dir: &str,
    min_tokens: usize,
    max_tokens: usize,
) {
    let initial_size = std::fs::metadata(filename_raw).unwrap().len();
    println!("Optimizing for proc '{}'", processing);

    let (filename, _temp) = maybe_process_file(filename_raw, filename_processed, processing);

    if initial_size < 1 << 24 {
        let fast_sampler = MemorySampler::from_file(&filename, 16384);
        let sampler = MemorySampler::from_file(&filename, 1 << 20);
        let slow_sampler = MemorySampler::from_file(&filename, 1 << 24);

        optimizer::optimize_all(
            initial_size,
            &slow_sampler,
            &sampler,
            &fast_sampler,
            tokens_dir,
            min_tokens,
            max_tokens,
            &processing.to_string(),
        );
    } else if initial_size < 1 << 32 {
        let fast_sampler = PreloadedSampler::new(&filename, 16384, 1024);
        let sampler = MemorySampler::from_file(&filename, 1 << 24);
        let slow_sampler = FileSampler::new(&filename, 1 << 24, None);

        optimizer::optimize_all(
            initial_size,
            &slow_sampler,
            &sampler,
            &fast_sampler,
            tokens_dir,
            min_tokens,
            max_tokens,
            &processing.to_string(),
        );
    } else {
        let fast_sampler = FileSampler::new(&filename, 131072, Some(1024));
        // let sampler = FileSampler::new(&filename, 1 << 20, Some(4096));
        let sampler = PreloadedSampler::new(&filename, 1 << 20, 1 << 14);
        let slow_sampler = FileSampler::new(&filename, 1 << 24, None);

        optimizer::optimize_all(
            initial_size,
            &slow_sampler,
            &sampler,
            &fast_sampler,
            tokens_dir,
            min_tokens,
            max_tokens,
            &processing.to_string(),
        );
    }
}

fn optimize_all(
    filename: &str,
    filename_caps_words: Option<&str>,
    tokens_dir: &str,
    min_tokens: usize,
    max_tokens: usize,
) {
    optimize_all_for_proc(
        filename,
        Some(filename),
        Processing::Raw,
        tokens_dir,
        min_tokens,
        std::cmp::min(max_tokens, 256),
    );
    optimize_all_for_proc(
        filename,
        filename_caps_words,
        Processing::CapsWords,
        tokens_dir,
        min_tokens,
        max_tokens,
    );
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

fn load_save_tokens(
    filename_raw: &str,
    filename_processed: Option<&str>,
    input_tokens_path: &str,
    tokens_dir: &str,
) {
    let tokens_dir_path = std::path::Path::new(tokens_dir);
    println!("Reading the token set from {}.", input_tokens_path);
    let input_tokens_file = File::open(input_tokens_path).expect("Input tokens file not found");
    let reader = BufReader::new(input_tokens_file);

    // Deserialize the JSON data into a serde_json::Value
    let tokenset_json: Value = serde_json::from_reader(reader).unwrap();
    let token_set = tokenset::TokenSet::from_json(tokenset_json);

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

fn optimize(
    ntokens: usize,
    filename_raw: &str,
    filename_processed: Option<&str>,
    tokens_dir: &str,
    processing: Processing,
    token_type: TokenType,
) {
    let tokens_dir_path = std::path::Path::new(tokens_dir);

    let (filename, _temp) = maybe_process_file(filename_raw, filename_processed, processing);
    let initial_size = std::fs::metadata(filename_raw).unwrap().len();

    println!("Opening {}", &filename);
    let sampler = FileSampler::new(&filename, 1 << 24, None);

    println!("Optimizing a token set with {} tokens", ntokens);
    let stats = optimize_tokenset(
        ntokens,
        &sampler,
        processing,
        token_type,
        Some(initial_size),
    );

    let output_path = tokens_dir_path.join(format!("{}.json", stats.token_set.name()));
    println!("Writing the token set to {}.", output_path.display());
    let serialized = serde_json::to_string(&stats.to_json()).unwrap();
    std::fs::write(&output_path, serialized).unwrap();
}

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

const DEFAULT_PROCESSING: &str = "raw";

#[derive(Subcommand, Debug)]
enum Command {
    OptimizeTokens {
        #[arg(short, long)]
        data: String,

        #[arg(short, long)]
        input_tokens: Option<String>,

        #[arg(short, long)]
        output_tokens: Option<String>,

        #[arg(short, long)]
        ntokens: usize,

        #[arg(long, default_value_t=LiteralEncoding::Dist8)]
        literals: LiteralEncoding,

        /// If the input is pre-processed, this argument specifies the size
        /// of the input before pre-processing.
        #[arg(long)]
        initial_size: Option<u64>,

        /// A comma-separated list of the processing stages applied to the input
        /// for the purposes of including it into the output.
        #[arg(long)]
        processing: Option<String>,

        #[arg(long, default_value_t=16 * 1024 * 1024)]
        chunk_size: usize,

        /// Sample this number of training data chunks, keep them in memory
        /// and train the token set on them. Final stats will be calculated
        /// the whole dataset.
        #[arg(long)]
        nchunks: Option<usize>,

        /// Use in-memory sampler in case all training data fits in memory.
        /// Ignored when nchunks is specified.
        #[arg(long)]
        in_memory: bool,

        /// How many tokens will be added after each pass.
        #[arg(long, default_value_t = 1)]
        add_block: usize,
    },

    OptimizeAll {
        #[arg(short, long)]
        data: String,

        // #[arg(long)]
        // data_caps: Option<String>,
        #[arg(long)]
        data_caps_words: Option<String>,

        #[arg(short, long)]
        tokens_dir: String,

        #[arg(long, default_value_t = 2)]
        min_tokens: usize,

        #[arg(long, default_value_t = 16384)]
        max_tokens: usize,
    },

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
        } => optimize(
            *ntokens,
            data,
            processed_data.as_deref(),
            tokens_dir,
            *processing,
            *token_type,
        ),

        Command::OptimizeTokens {
            data,
            input_tokens,
            output_tokens,
            ntokens,
            literals,
            initial_size,
            processing,
            in_memory,
            nchunks,
            chunk_size,
            add_block,
        } => optimize_tokens(
            data.as_str(),
            input_tokens,
            output_tokens,
            *ntokens,
            *initial_size,
            processing,
            *in_memory,
            *nchunks,
            *chunk_size,
            *add_block,
            *literals,
        ),

        Command::OptimizeAll {
            data,
            // data_caps,
            data_caps_words,
            tokens_dir,
            min_tokens,
            max_tokens,
        } => optimize_all(
            data.as_str(),
            data_caps_words.as_deref(),
            &tokens_dir.as_str(),
            *min_tokens,
            *max_tokens,
        ),

        Command::Process { data, output } => process(data.as_str(), output.as_str()),

        Command::CountChars { data } => count_chars(data.as_str()),
    }
}
