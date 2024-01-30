use clap::ValueEnum;
use serde::Serialize;
use std::fmt;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Processing {
    Raw,
    CapsWords,
}

impl fmt::Display for Processing {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Processing::Raw => "raw",
                Processing::CapsWords => "capswords",
            }
        )
    }
}

enum CharType {
    Letter,
    NonLetter,
    Space,
}

fn get_char_type(ch: char) -> CharType {
    if ch.is_alphabetic() {
        CharType::Letter
    } else if ch == ' ' {
        CharType::Space
    } else {
        CharType::NonLetter
    }
}

fn add_word(out: &mut String, word: &str) {
    assert!(!word.is_empty());

    let mut chars = word.chars();
    let first = chars.next().unwrap();
    let rest = chars.as_str();

    if first.is_uppercase() {
        if rest.chars().all(|ch| ch.is_lowercase()) {
            out.push('\x14');
            out.push(first.to_lowercase().next().unwrap());
            out.push_str(rest);
        } else if rest.chars().all(|ch| ch.is_uppercase()) {
            out.push('\x15');
            out.push_str(word.to_lowercase().as_str());
        } else {
            out.push_str(word);
        }
    } else {
        out.push_str(word);
    }

    out.push('\x16');
}

enum State {
    Word,
    SpaceAfterWord,
    NonWord,
}

/// Processes the text by making the following changes:
///
/// 1. Adds a character `\x16` at the end of each word (a sequence of letter characters, before either a non-letter character or the end of the text).
/// 2. Removes a single space between words. In the sequence <letter> `\x16` <space> <letter>, the space is removed.
/// 3. A capitalized word (a word starting with a capital letter, with remaining letters lowercase) is replaced by a `\x14` character followed by the lowercase version of the word.
/// 4. An all-uppercase word is replaced by a `\x15` character followed by the lowercase version of the word.
pub fn process(text: &str) -> String {
    let mut out = String::with_capacity(2 * text.len());
    let mut state = State::NonWord;
    let mut word = String::new();

    for ch in text.chars() {
        state = match (state, get_char_type(ch)) {
            (State::NonWord, CharType::Letter) => {
                word.push(ch);
                State::Word
            }
            (State::NonWord, CharType::Space | CharType::NonLetter) => {
                out.push(ch);
                State::NonWord
            }
            (State::Word, CharType::Letter) => {
                word.push(ch);
                State::Word
            }
            (State::Word, CharType::Space) => {
                add_word(&mut out, &word);
                word.clear();
                State::SpaceAfterWord
            }
            (State::Word, CharType::NonLetter) => {
                add_word(&mut out, &word);
                word.clear();
                out.push(ch);
                State::NonWord
            }
            (State::SpaceAfterWord, CharType::Letter) => {
                // Skipping the space character, since there was only one space between the words.
                word.push(ch);
                State::Word
            }
            (State::SpaceAfterWord, CharType::Space | CharType::NonLetter) => {
                out.push(' ');
                out.push(ch);
                State::NonWord
            }
        };
    }

    match state {
        State::Word => add_word(&mut out, &word),
        State::SpaceAfterWord => out.push(' '),
        State::NonWord => {}
    }

    out
}

pub fn process_file<R: Read, W: Write>(input: &mut R, output: &mut W) -> io::Result<()> {
    let reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);

    for line in reader.lines() {
        let line = line?;
        let processed = process(&line);
        writer.write_all(processed.as_bytes())?;
        writer.write(b"\n")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_process() {
        assert_eq!(super::process("Hello, world!"), "\x14hello\x16, world\x16!");
        assert_eq!(super::process("hello, world!"), "hello\x16, world\x16!");
        assert_eq!(super::process("HELLO, world!"), "\x15hello\x16, world\x16!");
        assert_eq!(super::process("HeLLo, world!"), "HeLLo\x16, world\x16!");
        assert_eq!(super::process("Hello world!"), "\x14hello\x16world\x16!");
        assert_eq!(
            super::process("Hello , world!"),
            "\x14hello\x16 , world\x16!"
        );
        assert_eq!(super::process("Hello, world "), "\x14hello\x16, world\x16 ");
        assert_eq!(super::process("Hello, world"), "\x14hello\x16, world\x16");
        assert_eq!(
            super::process("Hello, World"),
            "\x14hello\x16, \x14world\x16"
        );
        assert_eq!(super::process("Hello World"), "\x14hello\x16\x14world\x16");
        assert_eq!(super::process("Hello WORLD"), "\x14hello\x16\x15world\x16");
    }
}
