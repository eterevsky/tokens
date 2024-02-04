use std::io::{BufReader, BufRead};
use std::fs::File;

use crate::input::sample::{Sample, Sampler};

use super::util::find_paragraph_end;

pub struct MemorySampler {
    data: Vec<u8>,
    chunk_size: usize,
}

impl MemorySampler {
    pub fn from_file(filename: &str, chunk_size: usize) -> Self {
        let data = std::fs::read(filename).unwrap();
        MemorySampler { data, chunk_size }
    }

    /// Create a sampler by concatenating random full paragraphs from the file
    /// to reach approximately `size` bytes. The fragments read from the file will
    /// be distributed uniformly across the file.
    pub fn sample_from_file(filename: &str, size: usize, chunk_size: usize) -> Self {
        let file_size = std::fs::metadata(filename).unwrap().len() as usize;
        let target_share = size as f64 / file_size as f64;
        let mut data = Vec::new();
        let file = File::open(filename).unwrap();
        let mut reader = BufReader::new(file);
        
        let mut paragraph = Vec::new();
        let mut buffer = Vec::new();
        let mut read_bytes = 0;

        while reader.read_until(10, &mut buffer).unwrap() > 0 {
            if buffer[0] != 10 && paragraph.ends_with(&[10, 10]) {
                // We have a full paragraph

                if data.len() as f64 / (read_bytes as f64) < target_share {
                    data.extend_from_slice(&paragraph);
                }

                read_bytes += paragraph.len();
                paragraph.clear();
            }

            paragraph.extend_from_slice(&buffer);
            buffer.clear();
        }

        if data.len() as f64 / (read_bytes as f64) < target_share {
            data.extend_from_slice(&paragraph);
        }

        MemorySampler { data, chunk_size }
    }

    pub fn from_str(data: &str, chunk_size: usize) -> Self {
        MemorySampler {
            data: data.as_bytes().to_vec(),
            chunk_size: chunk_size,
        }
    }
}

impl<'a> Sampler<'a> for MemorySampler {
    type Iter = MemoryIterator<'a>;

    fn iter(&'a self) -> Self::Iter {
        MemoryIterator {
            sampler: self,
            position: 0,
        }
    }

    fn total_size(&'a self) -> u64 {
        self.data.len() as u64
    }
}

pub struct MemoryIterator<'a> {
    sampler: &'a MemorySampler,
    position: usize,
}

impl<'a> Iterator for MemoryIterator<'a> {
    type Item = Sample<'a>;

    fn next(&mut self) -> Option<Sample<'a>> {
        if self.position < self.sampler.data.len() {
            let start = self.position;
            self.position =
                std::cmp::min(start + self.sampler.chunk_size, self.sampler.data.len());
            let paragraph_end = find_paragraph_end(&self.sampler.data, self.position);
            if self.position < self.sampler.data.len() && paragraph_end > start {
                self.position = paragraph_end;
            }
            Some(Sample::from_bytes(&self.sampler.data[start..self.position]))
        } else {
            None
        }
    }
}
