use crate::input::sample::{Sample, Sampler};

use super::util::find_paragraph_end;

pub struct MemorySampler {
    data: Vec<u8>,
    sample_size: usize,
}

impl MemorySampler {
    pub fn from_file(filename: &str, sample_size: usize) -> Self {
        let data = std::fs::read(filename).unwrap();
        MemorySampler { data, sample_size }
    }

    pub fn from_str(data: &str, chunk_size: usize) -> Self {
        MemorySampler {
            data: data.as_bytes().to_vec(),
            sample_size: chunk_size,
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
                std::cmp::min(start + self.sampler.sample_size, self.sampler.data.len());
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
