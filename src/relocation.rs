use std::{collections::HashMap, fmt::Display};

// use anyhow::Error;
use wasm_encoder::reencode::{utils, Reencode};

#[derive(Eq, PartialEq, Hash)]
pub enum Relocation {
    Type(u32),
    Func(u32),
    Table(u32),
    Global(u32),
    Memory(u32),
    Data(u32),
    Elem(u32),
    Tag(u32),
}

#[derive(Debug)]
pub struct Error(anyhow::Error);

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<wasm_encoder::reencode::Error<anyhow::Error>> for Error {
    fn from(err: wasm_encoder::reencode::Error<anyhow::Error>) -> Self {
        Self(anyhow::anyhow!(err))
    }
}

pub struct RelocatingReencoder<'a> {
    pub relocations: &'a HashMap<Relocation, u32>,
}

impl<'a> Reencode for RelocatingReencoder<'a> {
    type Error = Error;

    fn data_index(&mut self, data: u32) -> u32 {
        utils::data_index(
            self,
            *self
                .relocations
                .get(&Relocation::Data(data))
                .unwrap_or(&data),
        )
    }

    fn element_index(&mut self, element: u32) -> u32 {
        utils::element_index(
            self,
            *self
                .relocations
                .get(&Relocation::Elem(element))
                .unwrap_or(&element),
        )
    }

    fn function_index(&mut self, func: u32) -> u32 {
        utils::function_index(
            self,
            *self
                .relocations
                .get(&Relocation::Func(func))
                .unwrap_or(&func),
        )
    }

    fn global_index(&mut self, global: u32) -> u32 {
        utils::global_index(
            self,
            *self
                .relocations
                .get(&Relocation::Global(global))
                .unwrap_or(&global),
        )
    }

    fn memory_index(&mut self, memory: u32) -> u32 {
        utils::memory_index(
            self,
            *self
                .relocations
                .get(&Relocation::Memory(memory))
                .unwrap_or(&memory),
        )
    }

    fn table_index(&mut self, table: u32) -> u32 {
        utils::table_index(
            self,
            *self
                .relocations
                .get(&Relocation::Table(table))
                .unwrap_or(&table),
        )
    }

    fn tag_index(&mut self, tag: u32) -> u32 {
        utils::tag_index(
            self,
            *self.relocations.get(&Relocation::Tag(tag)).unwrap_or(&tag),
        )
    }

    fn type_index(&mut self, ty: u32) -> u32 {
        utils::type_index(
            self,
            *self.relocations.get(&Relocation::Type(ty)).unwrap_or(&ty),
        )
    }
}
