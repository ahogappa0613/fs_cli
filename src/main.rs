use clap::{Parser, ValueEnum};
use object::*;
use std::fs::File;
use std::io::prelude::*;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use fs_cli::fs::Fs;
use fs_cli::writer;
use postcard::to_allocvec;

const RUBY_REQUIRE_PATCH_SRC: &[u8] = include_bytes!("./patch_require.rb");

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Ruby execute context.
    /// Specify absolute path.
    context: PathBuf,

    /// Packing dirs, files or gems.(e.g. dir/, file.rb, gem_name)
    /// These paths are use with `context' path, and if reading from FS fails, the path is used as the relative path from the executable's location instead
    /// so not support absolute path like start with `/' (e.g. /no_support_path)
    dir_or_file_or_gems: Vec<String>,

    /// Start up file.
    #[arg(short, long, default_value = "main.rb")]
    start: PathBuf,

    /// TODO
    /// start args
    #[arg(long)]
    args: Option<String>,

    /// TODO
    /// cross compilation target
    #[arg(long, value_enum, default_value_t = get_target())]
    target: Target,

    /// TODO
    /// compress ruby files with gzip
    #[arg(long)]
    compression: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum Target {
    Unix,
    MachO,
    Windows,
}

fn get_target() -> Target {
    if cfg!(unix) {
        Target::Unix
    } else if cfg!(macos) {
        Target::MachO
    } else if cfg!(windows) {
        Target::Windows
    } else {
        panic!("Not supported target")
    }
}

fn main() {
    let args = Args::parse();
    let load_path = Command::new("ruby")
        .arg("-e")
        .arg("$stdout.sync = true;puts $:;$stdout.flush")
        .output()
        .expect("not installed `ruby'");
    let load_path = unsafe { String::from_utf8_unchecked(load_path.stdout) };
    let mut load_path = load_path
        .split("\n")
        .filter_map(|path| {
            if path != "" {
                Some(String::from(path))
            } else {
                None
            }
        })
        .collect::<Vec<String>>();
    println!("{:?}", load_path);

    let mut dir_or_file_or_gems = args.dir_or_file_or_gems.clone();
    let mut context = args.context.clone();
    let start = args.start.clone();
    let start_path = context.clone();

    let mut files: Vec<PathBuf> = vec![];
    let mut fs = Fs::new();
    let mut include_paths = vec![];
    let mut absolute_start_file_path = start_path.join(start);

    include_paths.append(&mut dir_or_file_or_gems);
    include_paths.append(&mut load_path.clone());

    let mut fs_load_paths = vec![context.clone().to_string_lossy().to_string()];
    fs_load_paths.append(&mut load_path.clone());

    for dir_or_file_or_gem in include_paths.iter() {
        let mut path = PathBuf::from(dir_or_file_or_gem);

        if path.is_file() {
            let mut c = context.clone();
            c.push(path);
            files.push(c);
        } else if path.is_dir() {
            // support only ruby files
            path.push("**/*.rb");
            for entry in
                glob::glob(path.to_str().expect("error pathbuf to str")).expect("not found")
            {
                match entry {
                    Ok(path) => {
                        let mut c = context.clone();
                        c.push(path);
                        files.push(c);
                    }
                    Err(e) => println!("{:?}", e),
                }
            }
        } else {
            unreachable!("not found dir: {}", path.display());
        }
    }

    for file in files.iter() {
        let mut open = File::open(&file).expect(&format!("Not found start file"));
        let mut buf: Vec<u8> = vec![];
        open.read_to_end(&mut buf).expect("Not read file");
        buf.push(b'\0');
        fs.insert(&file, &mut buf);
    }

    for dir_or_file_or_gem in include_paths.iter() {
        let mut path = PathBuf::from(dir_or_file_or_gem);
        path.push("**/*.so");

        for entry in glob::glob(path.to_str().expect("error pathbuf to str")).expect("not found") {
            match entry {
                Ok(path) => {
                    let mut c = context.clone();
                    c.push(path);
                    fs.insert(c, &mut vec![b'\0']);
                }
                Err(e) => println!("{:?}", e),
            }
        }
    }

    let mut buf = RUBY_REQUIRE_PATCH_SRC.to_vec();
    buf.push(b'\0');
    fs.insert("/root/patch_require.rb", &mut buf);

    let format = match args.target {
        Target::Unix => BinaryFormat::Elf,
        Target::MachO => BinaryFormat::MachO,
        Target::Windows => BinaryFormat::Coff,
    };

    let fs_as_bytes = to_allocvec(&fs).unwrap();

    // TODO
    let arch = if cfg!(target_arch = "x86_64") {
        Architecture::X86_64
    } else if cfg!(target_arch = "x86") {
        Architecture::X86_64_X32
    } else if cfg!(target_arch = "aarch64") {
        Architecture::Aarch64
    } else {
        panic!("Not supported architecture")
    };

    let mut object = write::Object::new(format, arch, Endianness::Little);
    object.mangling = write::Mangling::None;
    object.flags = FileFlags::None;

    let section_id = object.section_id(write::StandardSection::ReadOnlyData);
    {
        let fs_symbol_id = object.add_symbol(object::write::Symbol {
            name: "FS".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: fs_as_bytes.len() as u64,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });

        let size_symbol_id = object.add_symbol(object::write::Symbol {
            name: "FS_SIZE".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: 8,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });

        let size = fs_as_bytes.len().to_le_bytes();

        object.add_symbol_data(fs_symbol_id, section_id, &fs_as_bytes, 2);
        object.add_symbol_data(size_symbol_id, section_id, &size, 2);
    }

    {
        let fs_load_paths = fs_load_paths.join(",");
        let context_id = object.add_symbol(object::write::Symbol {
            name: "LOAD_PATHS".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: fs_load_paths.len() as u64,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });
        let context_size_symbol_id = object.add_symbol(object::write::Symbol {
            name: "LOAD_PATHS_SIZE".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: 8,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });

        let size = fs_load_paths.len().to_le_bytes();

        object.add_symbol_data(context_id, section_id, fs_load_paths.as_bytes(), 2);
        object.add_symbol_data(context_size_symbol_id, section_id, &size, 2);
    }

    {
        let mut start_path = absolute_start_file_path.to_str().unwrap().to_string();
        start_path.push('\0');
        let start_path_id = object.add_symbol(object::write::Symbol {
            name: "START_PATH".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: start_path.len() as u64,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });
        let start_path_size_symbol_id = object.add_symbol(object::write::Symbol {
            name: "START_PATH_SIZE".bytes().collect::<Vec<u8>>(),
            value: 0,
            size: 8,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::None,
            flags: SymbolFlags::None,
        });

        let size = start_path.len().to_le_bytes();

        object.add_symbol_data(start_path_id, section_id, start_path.as_bytes(), 2);
        object.add_symbol_data(start_path_size_symbol_id, section_id, &size, 2);
    }

    {
        let result = object.write().unwrap();

        writer::write::<elf::FileHeader64<Endianness>>();

        let mut file = File::create("fs.o").unwrap();
        file.write_all(&result).unwrap();
    }
}
