use std::fs::File;

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
}

#[derive(Default)]
pub struct ImportCounts {
    pub functions: u32,
    pub tables: u32,
    pub memories: u32,
    pub globals: u32,
    pub tags: u32,
}
