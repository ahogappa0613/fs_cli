// use std::{
//     collections::HashMap,
//     path::{Path, PathBuf},
// };

// use serde::{Deserialize, Serialize};

// #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
// pub struct Fs {
//     /// /path/to/dir/file_a.rb -> 0
//     /// /path/to/dir/file_b.rb -> 1
//     entries: HashMap<PathBuf, usize>,
//     /// [0, 30,     64,        128, ...]
//     /// |....|.......|..........|......|
//     /// | 30 |   34  |    64    |      |
//     indexes: Vec<usize>,
//     data: Vec<u8>,
// }

// #[derive(Debug, PartialEq)]
// pub struct FsIterator<'a> {
//     index: usize,
//     fs: &'a Fs,
// }

// impl Fs {
//     pub fn new() -> Self {
//         Self {
//             indexes: vec![0],
//             ..Default::default()
//         }
//     }

//     pub fn len(&self) -> usize {
//         self.entries.len()
//     }

//     pub fn insert<P: AsRef<Path>>(&mut self, path: P, data: &mut Vec<u8>) {
//         let index = self.indexes.len() - 1;
//         let last_data = self.indexes.last().unwrap(); // SAFETY: indexes initialized by [0]

//         self.entries.insert(path.as_ref().to_path_buf(), index);
//         self.indexes.push(last_data + data.len());
//         self.data.append(data);
//     }

//     pub fn get<P: AsRef<Path>>(&self, path: P) -> Option<&[u8]> {
//         if let Some(index) = self.entries.get(&path.as_ref().to_path_buf()) {
//             Some(self[*index].as_ref())
//         } else {
//             None
//         }
//     }

//     pub fn get_file_size<P: AsRef<Path>>(&self, path: P) -> Option<usize> {
//         if let Some(index) = self.entries.get(&path.as_ref().to_path_buf()) {
//             let index = *index;
//             let start = self.indexes[index];
//             let len = self.indexes[index + 1];
//             Some(len - start)
//         } else {
//             None
//         }
//     }

//     pub fn iter(&self) -> FsIterator {
//         FsIterator { index: 0, fs: self }
//     }
// }

// impl std::ops::Index<usize> for Fs {
//     type Output = [u8];

//     fn index(&self, index: usize) -> &Self::Output {
//         let start = self.indexes[index];
//         let len = self.indexes[index + 1];

//         &self.data[start..len]
//     }
// }

// impl<'a> Iterator for FsIterator<'a> {
//     type Item = &'a [u8];

//     fn next(&mut self) -> Option<Self::Item> {
//         if self.index >= self.fs.indexes.len() - 1 {
//             None
//         } else {
//             let data = self.fs[self.index].as_ref();
//             self.index += 1;

//             Some(data)
//         }
//     }
// }

// #[cfg(test)]
// mod tests {
//     #[test]
//     fn insert_test() {
//         use super::Fs;

//         let mut fs = Fs::new();
//         let mut data = vec![b'h', b'e', b'l', b'l', b'o', b'\0'];
//         fs.insert("hello", &mut data);
//         let mut data = vec![b'w', b'o', b'r', b'l', b'd', b'\0'];
//         fs.insert("world", &mut data);
//         let mut data = vec![];
//         fs.insert("!", &mut data);
//         let mut data = vec![];
//         fs.insert("?", &mut data);
//         let mut data = vec![b'f', b's', b'\0'];
//         fs.insert("fs", &mut data);

//         let expected_data = [b'h', b'e', b'l', b'l', b'o', b'\0'];
//         assert_eq!(&fs[0], &expected_data[..]);

//         let expected_data = [b'w', b'o', b'r', b'l', b'd', b'\0'];
//         assert_eq!(&fs[1], &expected_data[..]);

//         let expected_data = [];
//         assert_eq!(&fs[2], &expected_data[..]);

//         let expected_data = [];
//         assert_eq!(&fs[3], &expected_data[..]);

//         let expected_data = [b'f', b's', b'\0'];
//         assert_eq!(&fs[4], &expected_data[..]);
//     }

//     #[test]
//     fn get_test() {
//         use super::Fs;

//         let mut fs = Fs::new();
//         let mut data = vec![b'h', b'e', b'l', b'l', b'o', b'\0'];
//         fs.insert("hello", &mut data);
//         let mut data = vec![b'w', b'o', b'r', b'l', b'd', b'\0'];
//         fs.insert("world", &mut data);
//         let mut data = vec![];
//         fs.insert("!", &mut data);
//         let mut data = vec![];
//         fs.insert("?", &mut data);
//         let mut data = vec![b'f', b's', b'\0'];
//         fs.insert("fs", &mut data);

//         let expected_data = [b'h', b'e', b'l', b'l', b'o', b'\0'];
//         assert_eq!(fs.get("hello"), Some(&expected_data[..]));

//         let expected_data = [b'w', b'o', b'r', b'l', b'd', b'\0'];
//         assert_eq!(fs.get("world"), Some(&expected_data[..]));

//         let expected_data = [];
//         assert_eq!(fs.get("!"), Some(&expected_data[..]));

//         let expected_data = [];
//         assert_eq!(fs.get("?"), Some(&expected_data[..]));

//         let expected_data = [b'f', b's', b'\0'];
//         assert_eq!(fs.get("fs"), Some(&expected_data[..]));
//     }

//     #[test]
//     fn get_file_size() {
//         use super::Fs;

//         let mut fs = Fs::new();
//         let mut data = vec![b'h', b'e', b'l', b'l', b'o', b'\0'];
//         fs.insert("hello", &mut data);
//         let mut data = vec![b'w', b'o', b'r', b'l', b'd', b'\0'];
//         fs.insert("world", &mut data);
//         let mut data = vec![];
//         fs.insert("!", &mut data);
//         let mut data = vec![];
//         fs.insert("?", &mut data);
//         let mut data = vec![b'f', b's', b'\0'];
//         fs.insert("fs", &mut data);

//         assert_eq!(fs.get_file_size("hello"), Some(6));
//         assert_eq!(fs.get_file_size("world"), Some(6));
//         assert_eq!(fs.get_file_size("!"), Some(0));
//         assert_eq!(fs.get_file_size("?"), Some(0));
//         assert_eq!(fs.get_file_size("fs"), Some(3));
//     }

//     #[test]
//     fn iter_test() {
//         use super::Fs;

//         let mut fs = Fs::new();
//         let mut data = vec![b'h', b'e', b'l', b'l', b'o', b'\0'];
//         fs.insert("hello", &mut data);
//         let mut data = vec![b'w', b'o', b'r', b'l', b'd', b'\0'];
//         fs.insert("world", &mut data);
//         let mut data = vec![];
//         fs.insert("!", &mut data);
//         let mut data = vec![];
//         fs.insert("?", &mut data);
//         let mut data = vec![b'f', b's', b'\0'];
//         fs.insert("fs", &mut data);

//         let mut iter = fs.iter();

//         let expected_data = [b'h', b'e', b'l', b'l', b'o', b'\0'];
//         assert_eq!(iter.next(), Some(&expected_data[..]));

//         let expected_data = [b'w', b'o', b'r', b'l', b'd', b'\0'];
//         assert_eq!(iter.next(), Some(&expected_data[..]));

//         let expected_data = [];
//         assert_eq!(iter.next(), Some(&expected_data[..]));

//         let expected_data = [];
//         assert_eq!(iter.next(), Some(&expected_data[..]));

//         let expected_data = [b'f', b's', b'\0'];
//         assert_eq!(iter.next(), Some(&expected_data[..]));


//         assert_eq!(iter.next(), None);
//     }
// }
