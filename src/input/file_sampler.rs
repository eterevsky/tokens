use rand::Rng;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::iter::Iterator;

use crate::input::sample::{Sample, Sampler};

use super::util::find_paragraph_end;

pub struct FileSampler {
    filename: String,
    sample_size: usize,
    max_samples: Option<usize>,
    file_size: u64,
}

impl FileSampler {
    pub fn new(filename: &str, sample_size: usize, max_samples: Option<usize>) -> Self {
        FileSampler {
            filename: filename.to_string(),
            sample_size,
            max_samples,
            file_size: std::fs::metadata(filename).unwrap().len(),
        }
    }
}

impl<'a> Sampler<'a> for FileSampler {
    type Iter = FileIterator<'a>;

    fn iter(&'a self) -> Self::Iter {
        let file = File::open(self.filename.as_str()).unwrap();

        if let Some(chunks_selection) = self.max_samples {
            FileIterator {
                _sampler: self,
                file,
                sample_size: self.sample_size,
                file_size: self.file_size,
                samples_left: Some(chunks_selection),
            }
        } else {
            FileIterator {
                _sampler: self,
                file,
                sample_size: self.sample_size,
                file_size: self.file_size,
                samples_left: None,
            }
        }
    }

    fn total_size(&self) -> u64 {
        if let Some(cs) = self.max_samples {
            (self.sample_size * cs) as u64
        } else {
            self.file_size
        }
    }
}

pub struct FileIterator<'a> {
    _sampler: &'a FileSampler,
    file: File,
    file_size: u64,
    sample_size: usize,
    samples_left: Option<usize>,
}

impl<'a> Iterator for FileIterator<'a> {
    type Item = Sample<'a>;

    fn next(&mut self) -> Option<Sample<'a>> {
        let mut buffer = Vec::new();
        buffer.resize(self.sample_size, 0);

        if let Some(samples_left) = self.samples_left {
            if samples_left == 0 {
                None
            } else {
                self.samples_left = Some(samples_left - 1);

                let mut rng = rand::thread_rng();
                let max_seek = self.file_size - self.sample_size as u64;
                let start = rng.gen_range(0..max_seek);

                self.file.seek(SeekFrom::Start(start)).unwrap();
                let read_bytes = self.file.read(&mut buffer).unwrap();

                buffer.truncate(read_bytes);
                let paragraph_end = find_paragraph_end(&buffer, buffer.len());
                buffer.truncate(paragraph_end);
                Some(Sample::from_vec(buffer))
            }
        } else {
            let read_bytes = self.file.read(&mut buffer).unwrap();

            if read_bytes == 0 {
                None
            } else if read_bytes < self.sample_size {
                buffer.truncate(read_bytes);
                Some(Sample::from_vec(buffer))
            } else {
                let end = find_paragraph_end(&buffer, read_bytes);
                if end < read_bytes {
                    buffer.truncate(end);
                    self.file
                        .seek(SeekFrom::Current(end as i64 - read_bytes as i64))
                        .unwrap();
                }
                Some(Sample::from_vec(buffer))
            }
        }
    }
}
