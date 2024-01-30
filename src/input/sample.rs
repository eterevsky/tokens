pub enum Sample<'a> {
    Data(String),
    Ref(&'a str),
}

impl<'a> Sample<'a> {
    /// Create a new Sample from a vector of bytes.
    /// The bytes should be valid UTF-8.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Sample::Data(String::from_utf8(data).unwrap())
    }   

    pub fn from_bytes(data: &'a [u8]) -> Self {
        match String::from_utf8_lossy(data) {
            std::borrow::Cow::Owned(s) => Sample::Data(s),
            std::borrow::Cow::Borrowed(s) => Sample::Ref(s),
        }
    }

    pub fn as_bytes(&'a self) -> &'a [u8] {
        match self {
            Sample::Data(data) => data.as_bytes(),
            Sample::Ref(data) => data.as_bytes(),
        }
    }

    pub fn as_str(&'a self) -> &'a str {
        match self {
            Sample::Data(data) => data.as_str(),
            Sample::Ref(data) => data,
        }
    }
}

pub trait Sampler<'a> {
    type Iter: Iterator<Item = Sample<'a>>;

    fn iter(&'a self) -> Self::Iter;

    fn total_size(&'a self) -> u64;
}

