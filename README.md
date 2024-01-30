# TexMo tokens

Optimizing set of tokens for language models. This is part of my language model playground TexMo which I haven't published yet.

This project aims to generate token sets from 2 (basically bits) to ~2^16 tokens. Since it is possible to generate token sets with less than 256 tokens, not every byte is necessarily has its own token. I'm using several different strategies to encode rare bytes by token sequences.

## Usage

```
cargo run --release -- optimize \
    -d <training data file> \
    -t <tokens directory> \
    --type=bits4 \
    -p caps-words \
    -n <number of tokens>
```

The program uses a single big text file as training data. The training data is expected to have Unix line ends.

A JSON file with the tokenset will be written to the specified directory.

The type of the token set mostly refers to how it represents bytes that don't have their own tokens. `bits4` encodes them as pairs of tokens which represent hexadecimal digits.

Tokenization could involve an optional reversible processing stage which is aimed to normalize spaces and capital letters. `-p caps-words` enables processing.

Number of tokens could be as low as 2 (single bits) and as high as tens of thousands.

## Processing

Tokenization involves an optional _processing_ stage, which is aimed to normalize spaces and capital letters, so that:

1. A capitalized can be represented by the same token as non-capitalized.
2. A sequence of words separated by spaces can be encoded as a sequence of tokens with one token per word, without extra tokens for spaces.
3. Spaces are not included in the tokens, so the word is still represented by the same token even if it is preceeded or followed by a punctuation sign.

This is achieved by adding `end-of-word`, `capitalized` and `all-caps` markers.

`end-of-word` is added after every word whether it is followed by a space or a punctuation sign. When decoding, if `end-of-word` appears between two letters, it can be replaced by a space. Otherwise it's dropped.

`capitalized` and `all-caps` markers are added _before_ the word. The first capitalizes the first letter, the second capitalizes all letters.

To process a text file, run

```
cargo run --release -- process -d <raw file> -o <output>
```

## Algorithm(s)

The program primarily relies on BPE algorithm, but also tries to remove previously added tokens to further optimize the token set.
