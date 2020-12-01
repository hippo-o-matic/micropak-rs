use std::env;
use std::mem::size_of; // For shortening size_of::<>() functions
use std::fs::File; // For files
use std::io::SeekFrom;
use std::io::prelude::*; // For writing into vecs
use std::path::Path; // For navigating filesystem
use std::path::PathBuf;
use std::collections::HashMap; // For archive tags

use std::convert::TryInto; // For fitting known size slices into arrays


const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const ARCHIVE_VERSION: u8 = 1; // Note: 0 is reserved for generic unsupported, in case versions go over 255 (they won't)
const SUPPORTED_ARCHIVE_VERSIONS: [u8; 1] = [1];
// const COMPRESSION_VERSION: u8 = 1;
// const SUPPORTED_COMPRESSION_VERSIONS: [u8; 1] = [1];

const MAX_BUFFER_SIZE: usize = 2_000_000_000; // Max buffer size is 2 GB


pub struct Archive {
	pub file: File,
	pub header: Header,
}

pub struct FileEntry {
	pub path: PathBuf,
	pub size: u64
}

pub struct Header {
	version: u8, // Version of the archive
	pub entries: Vec<FileEntry>, // Paths for 
	pub tags: HashMap<String, String>, // Additional data tags
	size: u64 // The size of the header in bytes
}

/// A function type for a function that takes a buffer of data, performs a reversible modification to it, and returns the resulting data.
type ByteOp = fn(Vec<u8>) -> Vec<u8>;

/// A [ByteOp] that does nothing, used as a placeholder for functions that require a ByteOp to be passed
pub fn nothing(data: Vec<u8>) -> Vec<u8> { data }

/// Takes a `&`[`Path`] to the top level of a path tree, and returns [`Vec`]<[`PathBuf`]> to each file in that path tree
fn expand_path(path: &Path) -> std::io::Result<Vec<PathBuf>> {
	let mut output_paths = Vec::new();
	if path.is_dir() {
		// For each item in the directory, walk its path tree and add the result to our own
		for entry in std::fs::read_dir(path)? {
			let entry = entry?;
			output_paths.extend(expand_path(&entry.path())?);
		}
	} else if path.is_file() {
		output_paths.push(path.to_path_buf());
	}

	Ok(output_paths)
}

// Takes a header structure and returns the bytes that should be written
// at the front of the archive.
// Additionally, returns a vec of paths that failed
// to be processed, these files should not be added to the archive
fn gen_header(header: &Header, root_paths: &Vec<PathBuf>) -> (Vec<u8>, Vec<PathBuf>) {
	let mut failed: Vec<PathBuf> = Vec::new();
	let mut data: Vec<u8> = Vec::new();

	data.write(&[header.version]).expect(&format!("Failed to do a write operation (Line {}", line!())); // Put the archive version at the front
	// Note: Vec::write apparently can't return an Err(), it just has to say it does because of the rtrait
	// Because of this, we don't really need to check for Err() and can just expect
	
	data.write(&0u64.to_le_bytes()).expect(&format!("Failed to do a write operation (Line {}", line!())); // Reserve a spot for the archive size, which we'll write after
	
	// Write all the tags
	// If a tag causes an error, panic and stop. We do this because the tags might hold
	// information neccesary for taking apart the archive, like compression type.
	// Also, an invalid tag is more likely the fault of the archiving software than a
	// user's input file
	data.write(&(header.tags.len() as u64).to_le_bytes()).expect(&format!("Failed to do a write operation (Line {}", line!())); // Write the number of tags
	for tag in &header.tags {
		data.write(&sized_bit_string(tag.0)).expect(&format!("Failed to write tag \"{}\", invalid name", &tag.0));
		data.write(&sized_bit_string(tag.1)).expect(&format!("Failed to write tag \"{}\", contents invalid", &tag.0));
	}

	// Write the amount of file entries, as u64
	data.write(&(header.entries.len() as u64).to_le_bytes()).expect(&format!("Failed to do a write operation (Line {}", line!()));
	for entry in &header.entries {
		// Write the file's size
		data.write(&entry.size.to_le_bytes()).expect(&format!("Failed to do a write operation (Line {}", line!()));
		
		// Now that we have the size, we can make the path relative for the archive
		let mut relative_path = entry.path.to_path_buf();
		for root in root_paths {
			if entry.path.starts_with(root) && entry.path != *root {
				relative_path = entry.path.strip_prefix(root).expect("Unable to make path relative").to_path_buf();
			}
		}
		
		// Write the relative path to the file
		match relative_path.to_str() {
			None => {
				println!("Couldn't convert path \"{}\" to a string, maybe it isn't UTF-8? Skipping file", entry.path.display());
				failed.push(entry.path.clone());
				continue;
			}
			Some(s) => {
				data.write(&sized_bit_string(&String::from(s))).expect(&format!("Failed to do a write operation (Line {}", line!()));
			}
		}
	}

	// Splice in the size of the archive, after the version
	data.splice(
		size_of::<u8>()..size_of::<u8>() + size_of::<u64>(), // From size_of(u8) to size_of(u8) + size_of(u64)
		(data.len() as u64).to_le_bytes().iter().cloned()
	);
	return (data, failed);
}

/// Reads a `&mut`[`File`] and returns the archive header if one is found.
pub fn read_header(file: &mut File) -> Header {
	let mut index: usize = 0;
	let mut header = Header {version: 0, entries: Vec::new(), tags: HashMap::new(), size: 0};

	// Read in the file signiture, archive version and the header size
	let mut info_buf: [u8; size_of::<u8>() + size_of::<u64>()] = Default::default();
	file.read_exact(&mut info_buf).expect("Unable to read archive info");

	header.version = match info_buf.get(0) {
		None => panic!("Unable to read archive info"),
		Some(v) => *v
	};

	let mut arr: [u8; size_of::<u64>()] = Default::default();
	arr.copy_from_slice(&info_buf[1..1 + size_of::<u64>()]);
	header.size = u64::from_le_bytes(arr);

	// TODO: Make this part not awful (the unwrap)
	let mut data = vec![0u8; header.size.try_into().unwrap()];
	file.read(&mut data).expect("Unable to read archive header");

	if !SUPPORTED_ARCHIVE_VERSIONS.contains(&header.version) {
		panic!("This version of the archiver ({}) does not support this archive's version ({}).\nTry updating to the latest version, your current version is {}", ARCHIVE_VERSION, header.version, VERSION);
	};

// Header version 1
if header.version == 1 {

	// Tags ******
	let mut arr: [u8; 8] = Default::default();
	arr.copy_from_slice(&data[index..index + size_of::<u64>()]);
	let tag_num = u64::from_le_bytes(arr);
	index += size_of::<u64>();

	for _ in 0..tag_num {
		header.tags.insert(read_sized_bit_string(&data, &mut index), read_sized_bit_string(&data, &mut index));
	};

	// Files ******
	let mut arr: [u8; 8] = Default::default();
	arr.copy_from_slice(&data[index..index + size_of::<u64>()]);
	let file_num = u64::from_le_bytes(arr);
	index += size_of::<u64>();

	for _ in 0..file_num {
		let mut arr: [u8; 8] = Default::default();
		arr.copy_from_slice(&data[index..index + size_of::<u64>()]);
		let file_size = u64::from_le_bytes(arr);
		index += size_of::<u64>();

		header.entries.push(FileEntry { path: PathBuf::from(read_sized_bit_string(&data, &mut index)), size: file_size });
	};

}

	header
}

// Pack functions ********************************************************

/// Creates an archive on `archive_file`, containing all paths contained by `root_paths`,
/// paths specified in `root_paths` will be located at the root of the archive, while folders
/// will recursively include paths they contain.
/// Tags can be added with `tags`, which can be used for arbitrary metadata
pub fn pack_archive(archive_file: &mut File, root_paths: &Vec<PathBuf>, tags: HashMap<String, String>) {
	let mut header = Header {
		version: ARCHIVE_VERSION,
		tags: tags,
		size: 0,
		entries: get_file_sizes(expand_paths(&root_paths))
	};

	let h_data = gen_header(&header, &root_paths);
	let failed_paths = h_data.1;
	
	// Remove failed paths
	for p in &failed_paths {
		match header.entries.iter().position(|r| r.path == *p) {
			Some(i) => {
				header.entries.remove(i);
			},
			None => ()
		};
	}

	// Write the header data from gen_header()
	archive_file.write(&h_data.0).expect("Failed to write to archive");

	// Append the files to the archive_file file
	for entry in &mut header.entries.iter() {
		let mut file = match File::open(&entry.path) {
			Err(why) => { 
				panic!("Failed to open file \"{}\", stopping. {}", entry.path.display(), why);
			},
			Ok(f) => f
		};

		match append_to_archive(&mut file, archive_file, nothing) {
			Err(why) => panic!("Failed to append file data from \"{}\" to archive_file: {}", entry.path.display(), why),
			Ok(_) => ()
		}
	}
}

/// Appends a File `file` to the end of `archive_file`.
/// A [`ByteOp`] can be passed to change the file data as it is copied
fn append_to_archive(file: &mut File, archive_file: &mut File, compression: ByteOp) -> std::io::Result<()> {
	let size = file.metadata()?.len();
	let max_size = MAX_BUFFER_SIZE.try_into().expect(
		&format!("Woah, you're running this on a >64 bit platform? Cool! It's broken. Try lowering your buffer size to something below {} bytes", u64::MAX));
	let mut remaining_size = size;

	while remaining_size > max_size {
		// Seek to the position of the next chunk. We do size - remaining because doing a 
		// simple SeekFrom::End(size) doesn't work, as it wants an i64 rather than a u64
		file.seek(SeekFrom::Start(size - remaining_size))?;
		// Create a buffer and read into it
		let mut buffer = vec![0u8; MAX_BUFFER_SIZE];
		file.read_exact(&mut buffer)?;

		// Run the given compression function on the data pulled. If there is no compression the data doesn't change
		buffer = compression(buffer);

		// Seek to the end of the archive file and write the compressed data
		archive_file.seek(SeekFrom::End(0))?;
		archive_file.write(&buffer)?;

		remaining_size -= max_size; // Decrease the size of the file remaining
	}

	// Do the same operations one more time for the either the last bytes, or for files already below
	// the maximum buffer size
	file.seek(SeekFrom::Start(size - remaining_size))?;
	let mut buffer = vec![0u8; remaining_size.try_into().unwrap()]; // remaining_size should be less than MAX_BUFFER_SIZE (a usize), so it's guaranteed to fit into usize
	file.read(&mut buffer)?;
	buffer = compression(buffer);
	archive_file.seek(SeekFrom::End(0))?;
	archive_file.write(&buffer)?;

	Ok(())
}

// Unpack functions ********************************************************

pub fn unpack_archive(mut file: File, out_path: &Path) -> std::io::Result<()> {
	// Try to create the directory to extract to
	match std::fs::create_dir_all(&out_path) {
		Err(why) => {
			println!("Failed to make directory \"{}\", skipping {}. {}", out_path.display(), out_path.display(), why); 
		},
		Ok(f) => f
	};

	let mut archive = Archive {
		header: read_header(&mut file),
		file: file
	};
	
	extract_all_archive(&mut archive, &out_path, nothing)?;
	archive.file.sync_all()?;
	
	Ok(())
}


// Finds a file (path_in_archive) in an archive and copies it to (out_path)
pub fn extract_from_archive(path_in_archive: &Path, archive: &mut Archive, mut out_file: &mut File, decompression: ByteOp) -> std::io::Result<()> {
	let mut index = archive.header.size; // Start at the end of the header

	for entry in &mut archive.header.entries {
		if entry.path == *path_in_archive { // Once we find the entry,
			buffered_copy(&mut archive.file, &mut out_file, &mut index, entry.size, decompression)?;
		}
	}

	Ok(())
}

pub fn extract_all_archive(archive: &mut Archive, out_path: &Path, decompression: ByteOp) -> std::io::Result<()> {
	let mut index = archive.header.size; // Start at the end of the header
		
	std::fs::create_dir_all(out_path)?;

	for entry in &mut archive.header.entries {
		let e_path = out_path.join(&entry.path);

		// Create directories for file
		match e_path.parent() {
			None => (),
			Some(parent) => std::fs::create_dir_all(parent)?
		}

		// Try to create the file
		let mut out_file = match File::create(&e_path) {
			Err(why) => {
				println!("Unable to create file \"{}\", {}", e_path.display(), why);
				continue;
			},
			Ok(file) => file 
		};

		buffered_copy(&mut archive.file, &mut out_file, &mut index, entry.size, decompression)?;
	};

	Ok(())
}

fn buffered_copy(file: &mut File, output: &mut dyn Write, index: &mut u64, size: u64, modify: ByteOp) -> std::io::Result<()> {
	let max_size = MAX_BUFFER_SIZE.try_into().expect(
		&format!("Woah, you're running this on a >64 bit platform? Cool! It's broken. Try lowering your buffer size to something below {} bytes", u64::MAX));

	let mut remaining_size = size;
	// If the file size is bigger than our buffer, split it up
	while remaining_size > max_size {
		// Seek to the position of the next chunk. We do size - remaining because doing a 
		// simple SeekFrom::End(size) doesn't work, as it wants an i64 rather than a u64
		file.seek(SeekFrom::Start(*index + (size - remaining_size) ))?;
		let mut buffer = vec![0u8; MAX_BUFFER_SIZE];
		file.read_exact(&mut buffer)?;

		buffer = modify(buffer);
		output.write(&buffer)?;

		remaining_size -= max_size;
	}

	// Do the same operations one more time for the either the last bytes, or for files already below
	// the maximum buffer size
	file.seek(SeekFrom::Start(*index + (size - remaining_size) ))?;
	let mut buffer = vec![0u8; remaining_size.try_into().unwrap()]; // remaining_size should be less than MAX_BUFFER_SIZE (a usize), so it's guaranteed to fit into usize
	file.read_exact(&mut buffer)?;

	buffer = modify(buffer);
	output.write(&buffer)?;

	*index += size; // Add each entry's size to the *index, which will give us the *index of the file data when we find it
	
	Ok(())
}

// Expands multiple root paths while checking for duplicates
fn expand_paths(input_paths: &Vec<PathBuf>) -> Vec<PathBuf> {
	let mut output_paths: Vec<PathBuf> = Vec::new(); // Make a vec of pathbufs to return

	for in_path in input_paths { // For each input path given to us,
		match expand_path(&in_path) { // walk its path tree and check for errors
			Err(why) => println!("Unable to follow path tree with root \"{}\": {}", in_path.display(), why),
			Ok(tree_paths) => {
				for p in tree_paths { 
					if !output_paths.contains(&p) { // Check for duplicates
						output_paths.push(p);
					}
				} 
			}, 
		};
	}

	output_paths
}

// Takes a vec of strings and returns a vec of PathBufs
pub fn strings_to_paths(strings: Vec<String>) -> Vec<PathBuf> {
	let mut paths = Vec::new();
	for string in strings {
		let path = PathBuf::from(string);
		paths.push(path);
	}
	paths
}

// Returns a vec of tuples, (path, file_size). The files left in <paths> are the paths that failed the metadata check and are not in the output
fn get_file_sizes(paths: Vec<PathBuf>) -> Vec<FileEntry> {
	let mut out = Vec::new();
	for path in paths {
		let size = match path.metadata() { // Try to get the metadata
			Err(why) => {
				println!("Failed to get metadata from \"{}\" because: {}, skipping file.", path.display(), why);
				continue;
			},
			// Sucessfully got metadata
			Ok(metadata) => metadata.len()
		};

		out.push(FileEntry { path: path, size: size });
	}

	out
}

/// Creates a Vec<u8> consisting of the size of (string) as a u64(little endian), and the string as bytes
/// 
/// # Examples
/// 
/// ```
/// let b_string = sized_bit_string("Hello");
/// assert_eq!(b_string, vec![5,0,0,0,0,0,0,0,72,101,108,108,111]);
/// //					    ^----size-----^  ^----"Hello"----^
/// ```
fn sized_bit_string(string: &String) -> Vec<u8> {
	let mut buffer: Vec<u8> = Vec::new();
	buffer.write(&(string.len() as u64).to_le_bytes()).expect("couldn't write string length to buffer");
	buffer.write(string.as_bytes()).expect("couldn't write string to buffer");
	return buffer;
}

/// From a buffer, reads a length (u64 little endian) in and returns a string from the length of bytes behind it,
/// starting from (index), and adds the length read to (index) 
/// Returns an empty string if it cant get the string's contents
/// 
/// # Examples
/// 
/// ```
/// let buffer = vec![0,0,0,5,0,0,0,0,0,0,0,72,101,108,108,111,0,0,0];
/// //					  ^----size-----^  ^----"Hello"----^
/// assert_eq!(read_sized_bit_string(buffer, 3), "Hello");
/// ```
fn read_sized_bit_string(buffer: &Vec<u8>, index: &mut usize) -> String {
	let len: usize = u64::from_le_bytes(
		buffer[*index..*index + size_of::<u64>()] // Take a slice of the buffer, from start_byte to the end of a u64
		.try_into().expect("slice for [string length] was wrong length, should have been 4 bytes") // Try to turn it into a 4 element array, if not, error
	) as usize;
	*index += size_of::<u64>();

	let mut contents = Vec::new();
	contents.extend(&buffer[*index..*index + len]);
	*index += len;

	return match String::from_utf8(contents) {
		Err(why) => {
			eprintln!("{}", why);
			String::new()
		},
		Ok(string) => string
	}
}


#[cfg(test)]
mod tests {
	use super::*;
	use std::fs::File;

	fn create_test_file(path: &str, data: Vec<u8>) -> std::io::Result<File> {
		let p = match PathBuf::from(path).parent() {
			None => PathBuf::from(path),
			Some(s) => s.to_path_buf()
		};

		std::fs::create_dir_all(p)?;
		let mut f = File::create(path)?;
		f.write_all(&data)?;

		f.sync_all()?;
		Ok(f)
	}

	fn compare_files(path1: &str, path2: &str) -> std::io::Result<bool> {
		let mut file1 = match File::open(path1) {
			Err(why) => panic!("Unable to open {}: {}", path1, why),
			Ok(file) => file,
		};

		let mut file2 = match File::open(path2) {
			Err(why) => panic!("Unable to open {}: {}", path2, why),
			Ok(file) => file,
		};

		let mut content1 = String::new();
		let mut content2 = String::new();
		file1.read_to_string(&mut content1)?;
		file2.read_to_string(&mut content2)?;

		if content1 == content2 {
			Ok(true)
		} else {
			Ok(false)
		}
	}

	#[test]
	fn basic_archive_test() -> std::io::Result<()> {
		create_test_file("pack_test/1.txt", b"Some test data".to_vec())?;
		create_test_file("pack_test/folder/2.txt", b"Some more test data".to_vec())?;
		create_test_file("pack_test/other_folder/folder/3.txt", b"Different test data".to_vec())?;

		let out_path = std::path::PathBuf::from("pack_test.mpk");
		let mut file = match File::create(&out_path) {
			Err(why) => panic!("Unable to create {}: {}", out_path.display(), why),
			Ok(file) => file,
		};

		let tags = std::collections::HashMap::new();
		// let mut str_paths: Vec<String> = Vec::new();
		// str_paths.push("pack_test/1.txt".to_string());
		// str_paths.push("pack_test/folder/2.txt".to_string());
		// str_paths.push("pack_test/other_folder/folder/3.txt".to_string());

		// let paths = strings_to_paths(str_paths);

		pack_archive(&mut file, &vec!(PathBuf::from("pack_test")), tags);
		file.flush()?;

		let unpack_file = match File::open(&out_path) {
			Err(why) => panic!("Unable to create {}: {}", out_path.display(), why),
			Ok(file) => file,
		};
		unpack_archive(unpack_file, &PathBuf::from("unpack_test"))?;

		compare_files("unpack_test/1.txt", "pack_test/1.txt")?;
		compare_files("unpack_test/folder/2.txt", "pack_test/folder/2.txt")?;
		compare_files("unpack_test/other_folder/folder/3.txt", "pack_test/other_folder/folder/3.txt")?;

		std::fs::remove_dir_all("pack_test")?;
		std::fs::remove_dir_all("unpack_test")?;

		Ok(())
	}
}