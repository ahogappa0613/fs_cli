use std::error::Error;
use std::fs::{self, File};
use std::io::Write;

use object::read::elf::{Dyn, FileHeader, Rel, Rela, SectionHeader, Sym};
use object::write::elf::{SectionIndex, SymbolIndex, Writer};
use object::{elf, Endianness};

struct Section {
    name: Option<object::write::StringId>,
    offset: usize,
    group: Option<Vec<SectionIndex>>,
}

struct Dynamic {
    tag: u32,
    // Ignored if `string` is set.
    val: u64,
    string: Option<object::write::StringId>,
}

struct Symbol {
    in_sym: usize,
    name: Option<object::write::StringId>,
    section: Option<object::write::elf::SectionIndex>,
}

struct DynamicSymbol {
    in_sym: usize,
    name: Option<object::write::StringId>,
    section: Option<object::write::elf::SectionIndex>,
    hash: Option<u32>,
    gnu_hash: Option<u32>,
}

const INIT_FUNC_START: &[u8] = "Init_".as_bytes();

pub fn write<Elf: FileHeader<Endian = Endianness>>() {
    let file_path = std::env::var("LIB_RUBY").unwrap_or(String::from(
        "/workspaces/ruby_packager/dest_dir/lib/libruby-static.a",
    ));
    let mut buf = vec![];
    let mut writer = Writer::new(object::Endianness::Little, true, &mut buf);

    // let obj = Object::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);

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
        println!("{}", name);
        println!("{}", i);

        if !name.starts_with("dmy") {
            let data = member.data(&file[..]).unwrap();
            let result = copy_file::<elf::FileHeader64<Endianness>>(data).unwrap();
            let mut file = File::create(format!("{i:0>3}.o")).unwrap();
            file.write_all(&result).unwrap();
        } // if !name.starts_with("dmy")
    }
}

fn copy_file<Elf: FileHeader<Endian = Endianness>>(
    in_data: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let in_elf = Elf::parse(in_data)?;
    let endian = in_elf.endian()?;
    let is_mips64el = in_elf.is_mips64el(endian);
    let in_segments = in_elf.program_headers(endian, in_data)?;
    let in_sections = in_elf.sections(endian, in_data)?;
    let in_syms = in_sections.symbols(endian, in_data, elf::SHT_SYMTAB)?;
    let in_dynsyms = in_sections.symbols(endian, in_data, elf::SHT_DYNSYM)?;

    let mut out_data = Vec::new();
    let mut writer = object::write::elf::Writer::new(endian, in_elf.is_class_64(), &mut out_data);

    // Find metadata sections, and assign section indices.
    let mut in_dynamic = None;
    let mut in_hash = None;
    let mut in_gnu_hash = None;
    let mut in_versym = None;
    let mut in_verdef = None;
    let mut in_verneed = None;
    let mut in_attributes = None;
    let mut out_sections = Vec::with_capacity(in_sections.len());
    let mut out_sections_index = Vec::with_capacity(in_sections.len());

    for (i, in_section) in in_sections.iter().enumerate() {
        let mut name = None;
        let index;
        let mut group = None;

        match in_section.sh_type(endian) {
            elf::SHT_NULL => {
                index = writer.reserve_null_section_index();
            }
            elf::SHT_PROGBITS
            | elf::SHT_NOBITS
            | elf::SHT_NOTE
            | elf::SHT_REL
            | elf::SHT_RELA
            | elf::SHT_INIT_ARRAY
            | elf::SHT_FINI_ARRAY => {
                name = Some(writer.add_section_name(in_sections.section_name(endian, in_section)?));
                index = writer.reserve_section_index();
            }
            elf::SHT_STRTAB => {
                if i == in_syms.string_section().0 {
                    index = writer.reserve_strtab_section_index();
                } else if i == in_dynsyms.string_section().0 {
                    index = writer.reserve_dynstr_section_index();
                } else if i == in_elf.shstrndx(endian, in_data)? as usize {
                    index = writer.reserve_shstrtab_section_index();
                } else {
                    panic!("Unsupported string section {i}");
                }
            }
            elf::SHT_SYMTAB => {
                if i == in_syms.section().0 {
                    index = writer.reserve_symtab_section_index();
                } else {
                    panic!("Unsupported symtab section {i}");
                }
            }
            elf::SHT_SYMTAB_SHNDX => {
                if i == in_syms.shndx_section().0 {
                    index = writer.reserve_symtab_shndx_section_index();
                } else {
                    panic!("Unsupported symtab shndx section {i}");
                }
            }
            elf::SHT_DYNSYM => {
                if i == in_dynsyms.section().0 {
                    index = writer.reserve_dynsym_section_index();
                } else {
                    panic!("Unsupported dynsym section {i}");
                }
            }
            elf::SHT_DYNAMIC => {
                assert!(in_dynamic.is_none());
                in_dynamic = in_section.dynamic(endian, in_data)?;
                debug_assert!(in_dynamic.is_some());
                index = writer.reserve_dynamic_section_index();
            }
            elf::SHT_HASH => {
                assert!(in_hash.is_none());
                in_hash = in_section.hash_header(endian, in_data)?;
                debug_assert!(in_hash.is_some());
                index = writer.reserve_hash_section_index();
            }
            elf::SHT_GNU_HASH => {
                assert!(in_gnu_hash.is_none());
                in_gnu_hash = in_section.gnu_hash_header(endian, in_data)?;
                debug_assert!(in_gnu_hash.is_some());
                index = writer.reserve_gnu_hash_section_index();
            }
            elf::SHT_GNU_VERSYM => {
                in_versym = in_section.gnu_versym(endian, in_data)?;
                debug_assert!(in_versym.is_some());
                index = writer.reserve_gnu_versym_section_index();
            }
            elf::SHT_GNU_VERDEF => {
                in_verdef = in_section.gnu_verdef(endian, in_data)?;
                debug_assert!(in_verdef.is_some());
                index = writer.reserve_gnu_verdef_section_index();
            }
            elf::SHT_GNU_VERNEED => {
                in_verneed = in_section.gnu_verneed(endian, in_data)?;
                debug_assert!(in_verneed.is_some());
                index = writer.reserve_gnu_verneed_section_index();
            }
            elf::SHT_GNU_ATTRIBUTES => {
                in_attributes = in_section.gnu_attributes(endian, in_data)?;
                debug_assert!(in_attributes.is_some());
                index = writer.reserve_gnu_attributes_section_index();
            }
            elf::SHT_GROUP => {
                if let Some((flag, indexes)) = in_section.group(endian, in_data)? {
                    match flag {
                        elf::GRP_COMDAT => {
                            index = writer.reserve_section_index();
                            name =
                                Some(writer.add_section_name(
                                    in_sections.section_name(endian, in_section)?,
                                ));

                            let mut group_indexes = Vec::with_capacity(indexes.len());
                            for s_index in indexes {
                                group_indexes.push(SectionIndex(s_index.get(endian)));
                            }
                            group = Some(group_indexes);
                        }
                        other => {
                            panic!("Unsupported COMDAT section type {other}")
                        }
                    }
                } else {
                    unreachable!()
                }
            }
            other => {
                panic!("Unsupported section type {:x}", other);
            }
        }
        out_sections.push(Section {
            name,
            offset: 0,
            group,
        });
        out_sections_index.push(index);
    }

    debug_assert_eq!(in_sections.len(), out_sections.len());

    // Assign dynamic strings.
    let mut out_dynamic = Vec::new();
    if let Some((in_dynamic, link)) = in_dynamic {
        out_dynamic.reserve(in_dynamic.len());
        let in_dynamic_strings = in_sections.strings(endian, in_data, link)?;
        for d in in_dynamic {
            let tag = d.d_tag(endian).into().try_into()?;
            let val = d.d_val(endian).into();
            let string = if d.is_string(endian) {
                let s = in_dynamic_strings
                    .get(val.try_into()?)
                    .map_err(|_| "Invalid dynamic string")?;
                Some(writer.add_dynamic_string(s))
            } else {
                None
            };
            out_dynamic.push(Dynamic { tag, val, string });
            if tag == elf::DT_NULL {
                break;
            }
        }
    }

    // Assign dynamic symbol indices.
    let mut out_dynsyms = Vec::with_capacity(in_dynsyms.len());
    for (i, in_dynsym) in in_dynsyms.iter().enumerate().skip(1) {
        let section = match in_dynsyms.symbol_section(endian, in_dynsym, i)? {
            Some(in_section) => {
                // Skip symbols for sections we aren't copying.
                if out_sections_index[in_section.0].0 == 0 {
                    continue;
                }
                Some(out_sections_index[in_section.0])
            }
            None => None,
        };
        let mut name = None;
        let mut hash = None;
        let mut gnu_hash = None;
        if in_dynsym.st_name(endian) != 0 {
            let in_name = in_dynsyms.symbol_name(endian, in_dynsym)?;
            let redefined_name = in_name;
            name = Some(writer.add_dynamic_string(redefined_name));
            if !redefined_name.is_empty() {
                hash = Some(elf::hash(redefined_name));
                if !in_dynsym.is_undefined(endian) {
                    gnu_hash = Some(elf::gnu_hash(redefined_name));
                }
            }
        };
        out_dynsyms.push(DynamicSymbol {
            in_sym: i,
            name,
            section,
            hash,
            gnu_hash,
        });
    }
    // We must sort for GNU hash before allocating symbol indices.
    if let Some(in_gnu_hash) = in_gnu_hash.as_ref() {
        // TODO: recalculate bucket_count
        out_dynsyms.sort_by_key(|sym| match sym.gnu_hash {
            None => (0, 0),
            Some(hash) => (1, hash % in_gnu_hash.bucket_count.get(endian)),
        });
    }
    let mut out_dynsyms_index = vec![Default::default(); in_dynsyms.len()];
    for out_dynsym in out_dynsyms.iter_mut() {
        out_dynsyms_index[out_dynsym.in_sym] = writer.reserve_dynamic_symbol_index();
    }

    // Assign symbol indices.
    let mut num_local = 0;
    let mut out_syms = Vec::with_capacity(in_syms.len());
    let mut out_syms_index = Vec::with_capacity(in_syms.len());
    out_syms_index.push(Default::default());
    for (i, in_sym) in in_syms.iter().enumerate().skip(1) {
        let section = match in_syms.symbol_section(endian, in_sym, i)? {
            Some(in_section) => {
                // Skip symbols for sections we aren't copying.
                if out_sections_index[in_section.0].0 == 0 {
                    out_syms_index.push(Default::default());
                    continue;
                }
                Some(out_sections_index[in_section.0])
            }
            None => None,
        };
        out_syms_index.push(writer.reserve_symbol_index(section));
        let name = if in_sym.st_name(endian) != 0 {
            Some(writer.add_string(in_syms.symbol_name(endian, in_sym)?))
        } else {
            None
        };
        out_syms.push(Symbol {
            in_sym: i,
            name,
            section,
        });
        if in_sym.st_bind() == elf::STB_LOCAL {
            num_local = writer.symbol_count();
        }
    }

    // Symbol version parameters.
    if let Some((mut verdefs, link)) = in_verdef.clone() {
        let strings = in_sections.strings(endian, in_data, link)?;
        while let Some((verdef, mut verdauxs)) = verdefs.next()? {
            assert!(verdef.vd_cnt.get(endian) > 0);
            while let Some(verdaux) = verdauxs.next()? {
                writer.add_dynamic_string(verdaux.name(endian, strings)?);
            }
        }
    }

    if let Some((mut verneeds, link)) = in_verneed.clone() {
        let strings = in_sections.strings(endian, in_data, link)?;
        while let Some((verneed, mut vernauxs)) = verneeds.next()? {
            writer.add_dynamic_string(verneed.file(endian, strings)?);
            while let Some(vernaux) = vernauxs.next()? {
                writer.add_dynamic_string(vernaux.name(endian, strings)?);
            }
        }
    }

    let mut gnu_attributes = Vec::new();
    if let Some(attributes) = in_attributes {
        let mut writer = writer.attributes_writer();
        let mut subsections = attributes.subsections()?;
        while let Some(subsection) = subsections.next()? {
            writer.start_subsection(subsection.vendor());
            let mut subsubsections = subsection.subsubsections();
            while let Some(subsubsection) = subsubsections.next()? {
                writer.start_subsubsection(subsubsection.tag());
                match subsubsection.tag() {
                    elf::Tag_File => {}
                    elf::Tag_Section => {
                        let mut indices = subsubsection.indices();
                        while let Some(index) = indices.next()? {
                            writer.write_subsubsection_index(out_sections_index[index as usize].0);
                        }
                        writer.write_subsubsection_index(0);
                    }
                    elf::Tag_Symbol => {
                        let mut indices = subsubsection.indices();
                        while let Some(index) = indices.next()? {
                            writer.write_subsubsection_index(out_syms_index[index as usize].0);
                        }
                        writer.write_subsubsection_index(0);
                    }
                    _ => unimplemented!(),
                }
                writer.write_subsubsection_attributes(subsubsection.attributes_data());
                writer.end_subsubsection();
            }
            writer.end_subsection();
        }
        gnu_attributes = writer.data();
        assert_ne!(gnu_attributes.len(), 0);
    }

    // Start reserving file ranges.
    writer.reserve_file_header();

    let hash_addr = 0;
    let gnu_hash_addr = 0;
    let versym_addr = 0;
    let verdef_addr = 0;
    let verneed_addr = 0;
    let dynamic_addr = 0;
    let dynsym_addr = 0;
    let dynstr_addr = 0;

    // Reserve sections at any offset.
    for (i, in_section) in in_sections.iter().enumerate() {
        match in_section.sh_type(endian) {
            elf::SHT_PROGBITS | elf::SHT_NOTE | elf::SHT_INIT_ARRAY | elf::SHT_FINI_ARRAY => {
                out_sections[i].offset = writer.reserve(
                    in_section.sh_size(endian).into() as usize,
                    in_section.sh_addralign(endian).into() as usize,
                );
            }
            elf::SHT_GNU_ATTRIBUTES => {
                writer.reserve_gnu_attributes(gnu_attributes.len());
            }
            elf::SHT_GROUP => {
                debug_assert!(out_sections[i].group.is_some());
                if let Some((_, indexes)) = in_section.group(endian, in_data)? {
                    out_sections[i].offset = writer.reserve_comdat(indexes.len());
                }
            }
            _ => {}
        }
    }

    writer.reserve_symtab();
    writer.reserve_symtab_shndx();
    writer.reserve_strtab();

    for (i, in_section) in in_sections.iter().enumerate() {
        if !in_segments.is_empty()
            && in_section.sh_flags(endian).into() & u64::from(elf::SHF_ALLOC) != 0
        {
            continue;
        }
        match in_section.sh_type(endian) {
            elf::SHT_REL => {
                let (rels, _link) = in_section.rel(endian, in_data)?.unwrap();
                out_sections[i].offset = writer.reserve_relocations(rels.len(), false);
            }
            elf::SHT_RELA => {
                let (rels, _link) = in_section.rela(endian, in_data)?.unwrap();
                out_sections[i].offset = writer.reserve_relocations(rels.len(), true);
            }
            _ => {}
        }
    }

    writer.reserve_shstrtab();
    writer.reserve_section_headers();

    writer.write_file_header(&object::write::elf::FileHeader {
        os_abi: in_elf.e_ident().os_abi,
        abi_version: in_elf.e_ident().abi_version,
        e_type: in_elf.e_type(endian),
        e_machine: in_elf.e_machine(endian),
        e_entry: in_elf.e_entry(endian).into(),
        e_flags: in_elf.e_flags(endian),
    })?;

    for (i, in_section) in in_sections.iter().enumerate() {
        match in_section.sh_type(endian) {
            elf::SHT_PROGBITS | elf::SHT_NOTE | elf::SHT_INIT_ARRAY | elf::SHT_FINI_ARRAY => {
                writer.write_align(in_section.sh_addralign(endian).into() as usize);
                debug_assert_eq!(out_sections[i].offset, writer.len());
                writer.write(in_section.data(endian, in_data)?);
            }
            elf::SHT_GNU_ATTRIBUTES => {
                writer.write_gnu_attributes(&gnu_attributes);
            }
            elf::SHT_GROUP => {
                debug_assert!(out_sections[i].group.is_some());
                writer.write_comdat_header();

                if let Some(section_indexes) = &out_sections[i].group {
                    for section_index in section_indexes.iter() {
                        writer.write_comdat_entry(SectionIndex(section_index.0));
                    }
                }
            }
            _ => {}
        }
    }

    writer.write_null_symbol();
    for sym in &out_syms {
        let in_sym = in_syms.symbol(sym.in_sym)?;

        writer.write_symbol(&object::write::elf::Sym {
            name: sym.name,
            section: sym.section,
            st_info: in_sym.st_info(),
            st_other: in_sym.st_other(),
            st_shndx: in_sym.st_shndx(endian),
            st_value: in_sym.st_value(endian).into(),
            st_size: in_sym.st_size(endian).into(),
        });
    }
    writer.write_symtab_shndx();
    writer.write_strtab();

    for in_section in in_sections.iter() {
        if !in_segments.is_empty()
            && in_section.sh_flags(endian).into() & u64::from(elf::SHF_ALLOC) != 0
        {
            continue;
        }
        let out_syms = if in_section.sh_link(endian) as usize == in_syms.section().0 {
            &out_syms_index
        } else {
            &out_dynsyms_index
        };
        match in_section.sh_type(endian) {
            elf::SHT_REL => {
                let (rels, _link) = in_section.rel(endian, in_data)?.unwrap();
                writer.write_align_relocation();
                for rel in rels {
                    let in_sym = rel.r_sym(endian);
                    let out_sym = if in_sym != 0 {
                        out_syms[in_sym as usize].0
                    } else {
                        0
                    };
                    writer.write_relocation(
                        false,
                        &object::write::elf::Rel {
                            r_offset: rel.r_offset(endian).into(),
                            r_sym: out_sym,
                            r_type: rel.r_type(endian),
                            r_addend: 0,
                        },
                    );
                }
            }
            elf::SHT_RELA => {
                let (rels, _link) = in_section.rela(endian, in_data)?.unwrap();
                writer.write_align_relocation();
                for rel in rels {
                    let in_sym = rel.r_sym(endian, is_mips64el);
                    let out_sym = if in_sym != 0 {
                        out_syms[in_sym as usize].0
                    } else {
                        0
                    };
                    writer.write_relocation(
                        true,
                        &object::write::elf::Rel {
                            r_offset: rel.r_offset(endian).into(),
                            r_sym: out_sym,
                            r_type: rel.r_type(endian, is_mips64el),
                            r_addend: rel.r_addend(endian).into(),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    writer.write_shstrtab();

    writer.write_null_section_header();
    for (i, in_section) in in_sections.iter().enumerate() {
        match in_section.sh_type(endian) {
            elf::SHT_NULL => {}
            elf::SHT_PROGBITS
            | elf::SHT_NOBITS
            | elf::SHT_NOTE
            | elf::SHT_REL
            | elf::SHT_RELA
            | elf::SHT_INIT_ARRAY
            | elf::SHT_FINI_ARRAY => {
                let out_section = &out_sections[i];
                let sh_link = out_sections_index[in_section.sh_link(endian) as usize].0;
                let mut sh_info = in_section.sh_info(endian);
                if in_section.sh_flags(endian).into() as u32 & elf::SHF_INFO_LINK != 0 {
                    sh_info = out_sections_index[sh_info as usize].0;
                }
                writer.write_section_header(&object::write::elf::SectionHeader {
                    name: out_section.name,
                    sh_type: in_section.sh_type(endian),
                    sh_flags: in_section.sh_flags(endian).into(),
                    sh_addr: in_section.sh_addr(endian).into(),
                    sh_offset: out_section.offset as u64,
                    sh_size: in_section.sh_size(endian).into(),
                    sh_link,
                    sh_info,
                    sh_addralign: in_section.sh_addralign(endian).into(),
                    sh_entsize: in_section.sh_entsize(endian).into(),
                });
            }
            elf::SHT_STRTAB => {
                if i == in_syms.string_section().0 {
                    writer.write_strtab_section_header();
                } else if i == in_dynsyms.string_section().0 {
                    writer.write_dynstr_section_header(dynstr_addr);
                } else if i == in_elf.shstrndx(endian, in_data)? as usize {
                    writer.write_shstrtab_section_header();
                } else {
                    panic!("Unsupported string section {}", i);
                }
            }
            elf::SHT_SYMTAB => {
                if i == in_syms.section().0 {
                    writer.write_symtab_section_header(num_local);
                } else {
                    panic!("Unsupported symtab section {}", i);
                }
            }
            elf::SHT_SYMTAB_SHNDX => {
                if i == in_syms.shndx_section().0 {
                    writer.write_symtab_shndx_section_header();
                } else {
                    panic!("Unsupported symtab shndx section {}", i);
                }
            }
            elf::SHT_DYNSYM => {
                if i == in_dynsyms.section().0 {
                    writer.write_dynsym_section_header(dynsym_addr, 1);
                } else {
                    panic!("Unsupported dynsym section {}", i);
                }
            }
            elf::SHT_DYNAMIC => {
                writer.write_dynamic_section_header(dynamic_addr);
            }
            elf::SHT_HASH => {
                writer.write_hash_section_header(hash_addr);
            }
            elf::SHT_GNU_HASH => {
                writer.write_gnu_hash_section_header(gnu_hash_addr);
            }
            elf::SHT_GNU_VERSYM => {
                writer.write_gnu_versym_section_header(versym_addr);
            }
            elf::SHT_GNU_VERDEF => {
                writer.write_gnu_verdef_section_header(verdef_addr);
            }
            elf::SHT_GNU_VERNEED => {
                writer.write_gnu_verneed_section_header(verneed_addr);
            }
            elf::SHT_GNU_ATTRIBUTES => {
                writer.write_gnu_attributes_section_header();
            }
            elf::SHT_GROUP => {
                let out_section = &out_sections[i];
                writer.write_comdat_section_header(
                    out_section.name.unwrap(),
                    SectionIndex(in_section.sh_link(endian)),
                    SymbolIndex(in_section.sh_info(endian)),
                    out_section.offset,
                    out_section.group.as_ref().unwrap().len(),
                );
            }
            other => {
                panic!("Unsupported section type {:x}", other);
            }
        }
    }
    debug_assert_eq!(writer.reserved_len(), writer.len());

    Ok(out_data)
}
