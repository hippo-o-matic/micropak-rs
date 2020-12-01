extern crate getopts; // Command line arguments
use getopts::Options;

use std::fs::File; // For files
use std::path::PathBuf;
use std::collections::HashMap; // For archive tags

pub mod archiver;

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let matches = match do_args(&args) {
		Err(_) => return,
		Ok(m) => m
	};

	let command = &matches.free[0];
	let absolute_paths = archiver::strings_to_paths(matches.free.clone()[1..].to_vec());

	let _compress = matches.opt_present("c");

	if command == "pack" || command == "p" { // Expand and pack absolute_paths
		let mut out_path = match matches.opt_str("o") {
			None => match std::env::current_dir() {
				Err(_) => PathBuf::from("Archive"),
				Ok(dir) => match dir.file_name() {
					None => dir.join(PathBuf::from("Archive")),
					Some(name) => dir.join(name)
				}
			},
			Some(out) => PathBuf::from(&out)
		};

		out_path = out_path.with_extension("mpk");

		let mut file = match File::create(&out_path) {
			Err(why) => panic!("Unable to create {}: {}", out_path.display(), why),
			Ok(file) => file,
		};

		let tags = HashMap::new();

		archiver::pack_archive(&mut file, &absolute_paths, tags);

	} else if command == "unpack" || command == "u" { // Unpack every archive in absolute_paths
		for archive_path in absolute_paths {
			// This little mess determines the output path of the archive
			// If it isn't specified, defaults to the name of the archive file
			// If we can't get the archive name, just calls the folder "Archive"
			let out_path = match matches.opt_str("o") {
				None => match archive_path.parent() {
					None => PathBuf::from("Archive"),
					Some(dir) => dir.join(match archive_path.file_stem() {
						None => PathBuf::from("Archive"),
						Some(stem) => PathBuf::from(stem) 
					})
				},
				Some(out) => PathBuf::from(&out)
			};

			// Try to open the archive file given to us
			let archive_file = match File::open(&archive_path) {
				Err(why) => { 
					println!("Failed to open archive \"{}\", skipping. {}", archive_path.display(), why);
					continue;
				},
				Ok(f) => f
			};

			archiver::unpack_archive(archive_file, &out_path).expect("Unable to unpack archive");
		}

	} else if command == "get" || command == "g" {
		let archive_path = &absolute_paths[0];
		let mut archive_file = match File::open(&archive_path) {
			Err(why) => panic!("Failed to open archive \"{}\", skipping. {}", archive_path.display(), why),
			Ok(f) => f
		};
		let header = archiver::read_header(&mut archive_file);

		let mut archive = archiver::Archive {
			file: archive_file,
			header: header
		};

		let out_path = match matches.opt_str("o") {
			None => match archive_path.parent() {
				None => PathBuf::from("Archive"),
				Some(dir) => dir.join(match archive_path.file_stem() {
					None => PathBuf::from("Archive"),
					Some(stem) => PathBuf::from(stem) 
				})
			},
			Some(out) => PathBuf::from(&out)
		};

		std::fs::create_dir_all(&out_path).expect("Unable to create output directory");

		for path in &absolute_paths[1..] {		
			let mut extracted_file = match File::create(out_path.join(&path)) {
				Err(why) => panic!("Failed to create file \"{}\", skipping. {}", archive_path.display(), why),
				Ok(f) => f
			};

			match archiver::extract_from_archive(&mut path.clone(), &mut archive, &mut extracted_file, archiver::nothing) {
				Err(why) => println!("Failed to extract target file \"{}\": {}", path.display(), why),
				Ok(_) => ()
			};
			continue;
		}
		
	} else if command == "scan" || command == "s" {
		// Prints the paths of every path in each archive given
		for archive_path in &absolute_paths {
			// Try to open the archive file given to us
			let mut archive_file = match File::open(archive_path) {
				Err(why) => { 
					println!("Failed to open archive \"{}\", skipping. {}", archive_path.display(), why);
					continue;
				},
				Ok(f) => f
			};

			let header = archiver::read_header(&mut archive_file);

			for entry in &header.entries {
				println!("{}", entry.path.display());
			}
		}

	} else { // No pack or unpack flag given, print usage
		print!("No commands given, use {} -h to see usage", args[0]);
		return;
	}
}


fn do_args(args: &Vec<String>) -> Result<getopts::Matches, &str> {
	let mut opts = Options::new();
	opts.optopt("o", "output", "Path to place the output", "PATH");
	// opts.optopt("g", "get_from", "Unpack specific files from the archive specified after this flag", "ARCHIVE_PATH");
	// opts.optflag("p", "pack", "Create an archive from the paths provided");
	// opts.optflag("u", "unpack", "Unpack archives from the paths provided");
	// opts.optflag("s", "scan", "Prints the paths of each item in the archive");
	opts.optflag("c", "compress", "Enable experimental compression");
	opts.optflag("h", "help", "Print this message");
	opts.optflag("v", "version", "Print the version of this archiver. If a file is specified, print the version it was packed with");
	let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(f) => { panic!(f.to_string()) }
	};

	let help_msg = format!(
"Usage: {} COMMAND PATH1 PATH2 ... [options]
		
Commands:
pack | p: Create an archive from the paths provided
unpack | u: Unpack archives from the paths provided
get | g: Unpack specific files from the archive specified by the first path given
scan | s: Prints the paths of each item in the archive\n"
	, args[0]);
	
	if matches.opt_present("h") {
		print!("{}", opts.usage(&help_msg));
		return Err("Help message");
	}

	if matches.free.is_empty() {
		print!("{}", opts.usage(&help_msg));
		return Err("No command");
	}

	Ok(matches)
}

