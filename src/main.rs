use clap::{Parser, ValueEnum};
use object::*;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::PathBuf;

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

fn register_file(
    scripts: &mut Vec<u8>,
    starts_and_ends: &mut Vec<u64>,
    paths: &mut Vec<PathBuf>,
    path: &PathBuf,
) {
    let mut open = File::open(&path).expect(&format!("Not found file"));
    let mut buf = vec![];
    open.read_to_end(&mut buf).expect("Not found file");

    paths.push(path.clone());
    register_bytes(scripts, starts_and_ends, &mut buf);
}

fn register_bytes(scripts: &mut Vec<u8>, starts_and_ends: &mut Vec<u64>, bytes: &mut Vec<u8>) {
    let last = starts_and_ends.last().unwrap();

    starts_and_ends.push(last + (bytes.len() + 1) as u64);
    scripts.append(bytes);
    scripts.push(b'\0');
}

fn main() {
    let args = Args::parse();
    let context = args.context.clone();
    let dir_or_file_or_gems = args.dir_or_file_or_gems.clone();
    let absolute_start_file_path = context.join(args.start);

    let mut starts_and_ends = vec![0u64];
    let mut scripts: Vec<u8> = vec![];
    let mut paths = vec![];

    register_file(
        &mut scripts,
        &mut starts_and_ends,
        &mut paths,
        &absolute_start_file_path,
    );

    for dir_or_file_or_gem in dir_or_file_or_gems.iter() {
        let mut path = PathBuf::from(dir_or_file_or_gem);

        if path.is_file() {
            let mut c = context.clone();
            c.push(path);
            register_file(&mut scripts, &mut starts_and_ends, &mut paths, &c);
            // paths.push(c);
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
                        register_file(&mut scripts, &mut starts_and_ends, &mut paths, &c);
                        // paths.push(c);
                    }
                    Err(e) => println!("{:?}", e),
                }
            }
        } else {
            unreachable!("not found dir: {}", path.display());
        }
    }

    // for path in paths.iter() {
    //     let mut open = File::open(&path).expect(&format!("Not found file"));
    //     let mut buf: Vec<u8> = vec![];
    //     open.read_to_end(&mut buf).expect("Not read file");
    //     buf.push(b'\0');

    //     let start = *starts_and_ends.last().unwrap() as u64;
    //     let end = start + buf.len() as u64;
    //     starts_and_ends.push(end);

    //     scripts.append(&mut buf);
    // }

    let mut patch = RUBY_REQUIRE_PATCH_SRC.to_vec();
    paths.push(PathBuf::from("/root/patch_require.rb"));
    register_bytes(&mut scripts, &mut starts_and_ends, &mut patch);

    // Register the path only when .so file
    for dir_or_file_or_gem in dir_or_file_or_gems.iter() {
        let mut path = PathBuf::from(dir_or_file_or_gem);
        path.push("**/*.so");

        for entry in glob::glob(path.to_str().expect("error pathbuf to str")).expect("not found") {
            match entry {
                Ok(path) => {
                    let mut c = context.clone();
                    c.push(path);
                    paths.push(c);
                }
                Err(e) => println!("{:?}", e),
            }
        }
    }

    let paths = paths
        .iter()
        .map(|path| {
            let mut path_string = String::from(path.to_string_lossy());
            path_string.push(b'\0' as char);
            path_string
        })
        .collect::<Vec<String>>()
        .join(",");

    let starts_and_ends = starts_and_ends
        .iter()
        .flat_map(|len| len.to_le_bytes())
        .collect::<Vec<u8>>();

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

    let format = match args.target {
        Target::Unix => BinaryFormat::Elf,
        Target::MachO => BinaryFormat::MachO,
        Target::Windows => BinaryFormat::Coff,
    };

    let mut object = write::Object::new(format, arch, Endianness::Little);
    object.mangling = write::Mangling::None;
    object.flags = FileFlags::None;

    let section_id = object.section_id(write::StandardSection::ReadOnlyData);

    let path_array_symbol_id = object.add_symbol(object::write::Symbol {
        name: "PATH_ARRAY".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: paths.len() as u64,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let path_array_size_symbol_id = object.add_symbol(object::write::Symbol {
        name: "PATH_ARRAY_SIZE".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: 8,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let start_and_end_symbol_id = object.add_symbol(object::write::Symbol {
        name: "START_AND_END".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: starts_and_ends.len() as u64,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let start_and_end_size_symbol_id = object.add_symbol(object::write::Symbol {
        name: "START_AND_END_SIZE".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: 8,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let files_symbol_id = object.add_symbol(object::write::Symbol {
        name: "FILES".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: scripts.len() as u64,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let files_size_symbol_id = object.add_symbol(object::write::Symbol {
        name: "FILES_SIZE".bytes().collect::<Vec<u8>>(),
        value: 0,
        size: 8,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::None,
        flags: SymbolFlags::None,
    });

    let fs_load_paths = dir_or_file_or_gems.join(",");
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

    let paths_size = paths.len().to_le_bytes();
    let starts_and_ends_size = (starts_and_ends.len() / 8).to_le_bytes();
    let script_size = scripts.len().to_le_bytes();
    let fs_load_paths_size = fs_load_paths.len().to_le_bytes();

    object.add_symbol_data(path_array_symbol_id, section_id, &paths.as_bytes(), 2);
    object.add_symbol_data(path_array_size_symbol_id, section_id, &paths_size, 2);
    object.add_symbol_data(start_and_end_symbol_id, section_id, &starts_and_ends, 2);
    object.add_symbol_data(
        start_and_end_size_symbol_id,
        section_id,
        &starts_and_ends_size,
        2,
    );
    object.add_symbol_data(files_symbol_id, section_id, &scripts, 2);
    object.add_symbol_data(files_size_symbol_id, section_id, &script_size, 2);
    object.add_symbol_data(context_id, section_id, fs_load_paths.as_bytes(), 2);
    object.add_symbol_data(context_size_symbol_id, section_id, &fs_load_paths_size, 2);

    let result = object.write().unwrap();
    let mut file = File::create("fs.o").unwrap();
    file.write_all(&result).unwrap();

    let file_path = std::env::var("LIB_RUBY").unwrap_or(String::from(
        "/workspaces/ruby_packager/dest_dir/lib/libruby-static.a",
    ));
    let file = match fs::File::open(&file_path) {
        Ok(file) => file,
        Err(err) => {
            println!("Failed to open file '{}': {}", file_path, err,);
            return;
        }
    };
    let file = match unsafe { memmap2::Mmap::map(&file) } {
        Ok(mmap) => mmap,
        Err(err) => {
            println!("Failed to map file '{}': {}", file_path, err,);
            return;
        }
    };
    let archive = match object::read::archive::ArchiveFile::parse(&*file) {
        Ok(file) => file,
        Err(err) => {
            println!("Failed to parse file '{}': {}", file_path, err);
            return;
        }
    };

    for (i, member) in archive.members().enumerate() {
        let member = member.unwrap();
        let name = String::from_utf8_lossy(member.name());

        if !name.starts_with("dmy") {
            let data = member.data(&file[..]).unwrap();
            let mut file = File::create(format!("{i:0>3}.o")).unwrap();
            file.write_all(&data).unwrap();
        }
    }
}

// fn main() {
//     let args = Args::parse();
//     let load_path = Command::new("ruby")
//         .arg("-e")
//         .arg("$stdout.sync = true;puts $:;$stdout.flush")
//         .output()
//         .expect("not installed `ruby'");
//     let load_path = unsafe { String::from_utf8_unchecked(load_path.stdout) };
//     let mut load_path = load_path
//         .split("\n")
//         .filter_map(|path| {
//             if path != "" {
//                 Some(String::from(path))
//             } else {
//                 None
//             }
//         })
//         .collect::<Vec<String>>();
//     println!("{:?}", load_path);

//     let mut dir_or_file_or_gems = args.dir_or_file_or_gems.clone();
//     let mut context = args.context.clone();
//     let start = args.start.clone();
//     let start_path = context.clone();

//     let mut files: Vec<PathBuf> = vec![];
//     let mut fs = Fs::new();
//     let mut include_paths = vec![];
//     let mut absolute_start_file_path = start_path.join(start);

//     include_paths.append(&mut dir_or_file_or_gems);
//     include_paths.append(&mut load_path.clone());

//     let mut fs_load_paths = vec![context.clone().to_string_lossy().to_string()];
//     fs_load_paths.append(&mut load_path.clone());

//     for dir_or_file_or_gem in include_paths.iter() {
//         let mut path = PathBuf::from(dir_or_file_or_gem);

//         if path.is_file() {
//             let mut c = context.clone();
//             c.push(path);
//             files.push(c);
//         } else if path.is_dir() {
//             // support only ruby files
//             path.push("**/*.rb");
//             for entry in
//                 glob::glob(path.to_str().expect("error pathbuf to str")).expect("not found")
//             {
//                 match entry {
//                     Ok(path) => {
//                         let mut c = context.clone();
//                         c.push(path);
//                         files.push(c);
//                     }
//                     Err(e) => println!("{:?}", e),
//                 }
//             }
//         } else {
//             unreachable!("not found dir: {}", path.display());
//         }
//     }

//     for file in files.iter() {
//         let mut open = File::open(&file).expect(&format!("Not found start file"));
//         let mut buf: Vec<u8> = vec![];
//         open.read_to_end(&mut buf).expect("Not read file");
//         buf.push(b'\0');
//         fs.insert(&file, &mut buf);
//     }

//     for dir_or_file_or_gem in include_paths.iter() {
//         let mut path = PathBuf::from(dir_or_file_or_gem);
//         path.push("**/*.so");

//         for entry in glob::glob(path.to_str().expect("error pathbuf to str")).expect("not found") {
//             match entry {
//                 Ok(path) => {
//                     let mut c = context.clone();
//                     c.push(path);
//                     fs.insert(c, &mut vec![b'\0']);
//                 }
//                 Err(e) => println!("{:?}", e),
//             }
//         }
//     }

//     let mut buf = RUBY_REQUIRE_PATCH_SRC.to_vec();
//     buf.push(b'\0');
//     fs.insert("/root/patch_require.rb", &mut buf);

//     let format = match args.target {
//         Target::Unix => BinaryFormat::Elf,
//         Target::MachO => BinaryFormat::MachO,
//         Target::Windows => BinaryFormat::Coff,
//     };

//     let fs_as_bytes = to_allocvec(&fs).unwrap();

//     // TODO
//     let arch = if cfg!(target_arch = "x86_64") {
//         Architecture::X86_64
//     } else if cfg!(target_arch = "x86") {
//         Architecture::X86_64_X32
//     } else if cfg!(target_arch = "aarch64") {
//         Architecture::Aarch64
//     } else {
//         panic!("Not supported architecture")
//     };

//     let mut object = write::Object::new(format, arch, Endianness::Little);
//     object.mangling = write::Mangling::None;
//     object.flags = FileFlags::None;

//     // let section_id = object.section_id(write::StandardSection::ReadOnlyData);
//     // {
//     //     let fs_symbol_id = object.add_symbol(object::write::Symbol {
//     //         name: "FS".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: fs_as_bytes.len() as u64,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });

//     //     let size_symbol_id = object.add_symbol(object::write::Symbol {
//     //         name: "FS_SIZE".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: 8,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });

//     //     let size = fs_as_bytes.len().to_le_bytes();

//     //     object.add_symbol_data(fs_symbol_id, section_id, &fs_as_bytes, 2);
//     //     object.add_symbol_data(size_symbol_id, section_id, &size, 2);
//     // }

//     // {
//     //     let fs_load_paths = fs_load_paths.join(",");
//     //     let context_id = object.add_symbol(object::write::Symbol {
//     //         name: "LOAD_PATHS".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: fs_load_paths.len() as u64,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });
//     //     let context_size_symbol_id = object.add_symbol(object::write::Symbol {
//     //         name: "LOAD_PATHS_SIZE".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: 8,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });

//     //     let size = fs_load_paths.len().to_le_bytes();

//     //     object.add_symbol_data(context_id, section_id, fs_load_paths.as_bytes(), 2);
//     //     object.add_symbol_data(context_size_symbol_id, section_id, &size, 2);
//     // }

//     // {
//     //     let mut start_path = absolute_start_file_path.to_str().unwrap().to_string();
//     //     start_path.push('\0');
//     //     let start_path_id = object.add_symbol(object::write::Symbol {
//     //         name: "START_PATH".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: start_path.len() as u64,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });
//     //     let start_path_size_symbol_id = object.add_symbol(object::write::Symbol {
//     //         name: "START_PATH_SIZE".bytes().collect::<Vec<u8>>(),
//     //         value: 0,
//     //         size: 8,
//     //         kind: SymbolKind::Data,
//     //         scope: SymbolScope::Linkage,
//     //         weak: false,
//     //         section: write::SymbolSection::None,
//     //         flags: SymbolFlags::None,
//     //     });

//     //     let size = start_path.len().to_le_bytes();

//     //     object.add_symbol_data(start_path_id, section_id, start_path.as_bytes(), 2);
//     //     object.add_symbol_data(start_path_size_symbol_id, section_id, &size, 2);
//     // }

//     {
//         let result = object.write().unwrap();

//         writer::write::<elf::FileHeader64<Endianness>>();

//         let mut file = File::create("fs.o").unwrap();
//         file.write_all(&result).unwrap();
//     }
// }
