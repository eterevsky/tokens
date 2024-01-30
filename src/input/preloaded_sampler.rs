use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::iter::Iterator;

use crate::input::sample::{Sample, Sampler};
use crate::input::util::{extract_valid_utf8_slice, find_paragraph_end};

pub struct PreloadedSampler {
    chunks: Vec<Vec<u8>>,
    _total_size: u64,
}

impl PreloadedSampler {
    pub fn new(filename: &str, sample_size: usize, max_samples: usize) -> Self {
        // Get the metadata of the file
        let data_len = std::fs::metadata(filename).unwrap().len() as usize;

        let (sample_size, nsamples) = if data_len <= sample_size {
            (data_len, 1)
        } else if data_len <= sample_size * max_samples {
            (sample_size, data_len / sample_size)
        } else {
            (sample_size, max_samples)
        };

        println!(
            "Preparing SelectionSampler with {} chunks x {} bytes",
            nsamples, sample_size
        );

        let step = data_len / nsamples;

        let mut file = File::open(filename).unwrap();

        let mut chunks = Vec::new();

        for i in 0..nsamples as u64 {
            file.seek(SeekFrom::Start(i * step as u64)).unwrap();
            let mut chunk = Vec::new();
            chunk.resize(sample_size, 0);
            let read_bytes = file.read(&mut chunk).unwrap();
            chunk.truncate(read_bytes);

            let paragraph_end = find_paragraph_end(&chunk, chunk.len());
            let chunk = &chunk[..paragraph_end];
            let valid_chunk = extract_valid_utf8_slice(chunk);

            if valid_chunk.len() == chunk.len() {
                chunks.push(chunk.to_vec());
            } else {
                chunks.push(valid_chunk.to_vec());
            };
        }            

        let _total_size = chunks.iter().map(|c| c.len() as u64).sum();
        PreloadedSampler {
            chunks,
            _total_size,
        }
    }
}

impl<'a> Sampler<'a> for PreloadedSampler {
    type Iter = SelectionIterator<'a>;

    fn iter(&'a self) -> Self::Iter {
        SelectionIterator {
            sampler: self,
            position: 0,
        }
    }

    fn total_size(&'a self) -> u64 {
        self._total_size
    }
}

pub struct SelectionIterator<'a> {
    sampler: &'a PreloadedSampler,
    position: usize,
}

impl<'a> Iterator for SelectionIterator<'a> {
    type Item = Sample<'a>;

    fn next(&mut self) -> Option<Sample<'a>> {
        if self.position < self.sampler.chunks.len() {
            let chunk = &self.sampler.chunks[self.position];
            self.position += 1;
            Some(Sample::from_bytes(chunk))
        } else {
            None
        }
    }
}
