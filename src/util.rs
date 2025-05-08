use std::{cmp::Ordering, fs::File};

use wasm_encoder::RawSection;

pub fn get_reader(filename: String) -> Box<dyn std::io::Read> {
    if filename == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(File::open(filename).expect("Failed to open file"))
    }
}

#[derive(Debug)]
pub enum Section<'a> {
    Passthrough(RawSection<'a>),
    Type,
    Import,
    Function,
    Table,
    Memory,
    Global,
    Export,
    Start,
    Element,
    Code,
    Data,
    DataCount,
    Tag,
}

impl<'a> Section<'a> {
    pub fn raw(id: u8, bytes: &'a [u8]) -> Section<'a> {
        let foo = RawSection {
            id: id,
            data: bytes,
        };
        Self::Passthrough(foo)
    }

    fn binary_order(&self) -> u8 {
        match self {
            Section::Type => 0,
            Section::Import => 1,
            Section::Function => 2,
            Section::Table => 3,
            Section::Memory => 4,
            Section::Tag => 5,
            Section::Global => 6,
            Section::Export => 7,
            Section::Start => 8,
            Section::Element => 9,
            Section::DataCount => 10,
            Section::Code => 11,
            Section::Data => 12,
            Section::Passthrough(_) => 255, // Custom sections are not reordered
        }
    }
}

pub fn ensure_section<'a>(sections: &mut Vec<Section<'a>>, new_section: Section<'a>) {
    // Do not insert if a section of the same kind already exists
    if sections
        .iter()
        .any(|s| std::mem::discriminant(s) == std::mem::discriminant(&new_section))
    {
        return;
    }

    let new_order = match &new_section {
        Section::Passthrough(_) => return, // Do not insert custom sections here
        _ => new_section.binary_order(),
    };

    // Find correct insertion index
    let mut insert_index = sections.len();
    for (i, section) in sections.iter().enumerate() {
        match section {
            // Skip over custom/unknown sections
            Section::Passthrough(_) => continue,

            // If we encounter a section with a binary order greater than ours,
            // insert the new section there (immediately before it).
            _ if section.binary_order() > new_order => {
                insert_index = i;
                break;
            }

            // Rust!
            _ => {}
        }
    }

    sections.insert(insert_index, new_section);
}

#[derive(Default)]
pub struct ImportCounts {
    pub functions: u32,
    pub tables: u32,
    pub memories: u32,
    pub globals: u32,
    pub tags: u32,
}
