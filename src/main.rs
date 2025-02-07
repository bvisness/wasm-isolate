mod relocation;
mod uses;

use std::{
    collections::HashMap,
    fs::{self, File},
};

use anyhow::Result;
use clap::Parser as _;
use wasm_encoder::{
    reencode::Reencode, CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode,
    ElementMode, ElementSection, ElementSegment, EntityType, ExportSection, Function,
    FunctionSection, GlobalSection, ImportSection, Instruction, Module, RawSection, TableSection,
};
use wasmparser::{
    CompositeInnerType, Data, Element, Export, Global, GlobalType, Import, MemoryType, Operator,
    Parser, Payload::*, Table, TableType, TagType, ValType,
};

use relocation::*;
use uses::*;

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    filename: String,

    #[arg(short, long, required = true)]
    funcs: Vec<u32>,

    #[arg(short, long, required = true)]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let filename = args.filename;
    let mut reader = get_reader(filename);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let parser = Parser::new(0);

    let mut types: Vec<CompositeInnerType> = vec![];
    let mut num_imported_functions: u32 = 0;
    let mut num_imported_tables: u32 = 0;
    let mut num_imported_memories: u32 = 0;
    let mut num_imported_globals: u32 = 0;
    let mut num_imported_tags: u32 = 0;
    let mut func_types: Vec<u32> = vec![];
    let mut table_types: Vec<TableType> = vec![];
    let mut memory_types: Vec<MemoryType> = vec![];
    let mut global_types: Vec<GlobalType> = vec![];
    let mut tag_types: Vec<TagType> = vec![];

    let mut current_func = 0;
    let mut first_func: bool = true;

    let mut imports: Vec<Import> = vec![];
    let mut defined_tables: Vec<Table> = vec![];
    let mut defined_globals: Vec<Global> = vec![];
    let mut exports: Vec<Export> = vec![];
    let mut start_idx: Option<u32> = None;
    let mut elems: Vec<Element> = vec![];
    let mut defined_funcs: Vec<Func> = vec![];
    let mut datas: Vec<Data> = vec![];

    let mut sections: Vec<Section> = vec![];

    for payload in parser.parse_all(&buf) {
        match payload? {
            // Sections for WebAssembly modules
            TypeSection(r) => {
                sections.push(Section::raw(1, &buf[r.range()]));

                for rg in r {
                    for t in rg?.into_types() {
                        types.push(t.composite_type.inner);
                    }
                }
            }
            ImportSection(r) => {
                sections.push(Section::Import);

                for import in r {
                    let import = import?;
                    match import.ty {
                        // TODO: Save these types for walking later.
                        wasmparser::TypeRef::Func(type_idx) => {
                            num_imported_functions += 1;
                            func_types.push(type_idx);
                        }
                        wasmparser::TypeRef::Table(ty) => {
                            num_imported_tables += 1;
                            table_types.push(ty);
                        }
                        wasmparser::TypeRef::Memory(ty) => {
                            num_imported_memories += 1;
                            memory_types.push(ty);
                        }
                        wasmparser::TypeRef::Global(ty) => {
                            num_imported_globals += 1;
                            global_types.push(ty);
                        }
                        wasmparser::TypeRef::Tag(ty) => {
                            num_imported_tags += 1;
                            tag_types.push(ty);
                        }
                    }
                    imports.push(import);
                }
            }
            FunctionSection(r) => {
                sections.push(Section::Function);
                for f in r {
                    func_types.push(f?);
                }
            }
            TableSection(r) => {
                sections.push(Section::Table);
                for table in r {
                    let table = table?;
                    table_types.push(table.ty);
                    defined_tables.push(table);
                }
            }
            MemorySection(r) => {
                sections.push(Section::Memory);
                for mem_type in r {
                    memory_types.push(mem_type?);
                }
            }
            TagSection(r) => {
                sections.push(Section::raw(13, &buf[r.range()]));
            }
            GlobalSection(r) => {
                sections.push(Section::Global);
                for global in r {
                    let global = global?;
                    global_types.push(global.ty);
                    defined_globals.push(global);
                }
            }
            ExportSection(r) => {
                sections.push(Section::Export);
                for export in r {
                    exports.push(export?);
                }
            }
            StartSection { func, range: _ } => {
                // IDEA: Just because we presere the start function doesn't
                // necessarily mean we want to preserve the start section.
                // Should we have a flag for this?
                sections.push(Section::Start);
                start_idx = Some(func);
            }
            ElementSection(r) => {
                sections.push(Section::Element);
                for elem in r {
                    elems.push(elem?);
                }
            }
            DataCountSection { count: _, range } => {
                sections.push(Section::DataCount);
            }
            DataSection(r) => {
                sections.push(Section::Data);
                for data in r {
                    datas.push(data?);
                }
            }

            // Here we know how many functions we'll be receiving as
            // `CodeSectionEntry`, so we can prepare for that, and
            // afterwards we can parse and handle each function
            // individually.
            CodeSectionStart { .. } => {
                sections.push(Section::Code);
                current_func = num_imported_functions;
            }
            CodeSectionEntry(body) => {
                if first_func {
                    first_func = false
                } else {
                    current_func += 1;
                }

                let mut func = Func {
                    type_idx: func_types[current_func as usize],
                    locals: vec![],
                    instructions: vec![],
                };

                for local in body.get_locals_reader()? {
                    func.locals.push(local?);
                }
                for instr in body.get_operators_reader()? {
                    func.instructions.push(instr?);
                }

                defined_funcs.push(func)
            }

            CustomSection(r) => {
                if r.name() == "name" {
                    continue;
                }
                sections.push(Section::raw(0, &buf[r.range()]));
            }

            _ => {}
        }
    }

    //
    // Iterate over all live objects until we have gathered all the references.
    //

    let mut work_queue: Vec<WorkItem> = vec![];
    for func in args.funcs {
        work_queue.push(WorkItem::Func(func));
    }

    let mut all_uses = Uses::default();

    while !work_queue.is_empty() {
        let work = work_queue.first().expect("non-empty queue");

        let new_uses = match work {
            WorkItem::Type(idx) => {
                let mut res = Uses::single_type(*idx);
                res.merge(get_type_uses(&types[*idx as usize]));
                res
            }
            WorkItem::Func(idx) => {
                let mut res = Uses::single_func(*idx);
                if *idx < num_imported_functions {
                    // TODO: Track type of imported function
                    // Suggestion: Make an array of all func types, both imports and defined.
                    // This can be separate from the array of defined functions.
                } else {
                    let func = &defined_funcs[(idx - num_imported_functions) as usize];
                    res.merge(Uses::single_type(func.type_idx));
                    for (_, ty) in &func.locals {
                        res.merge(get_valtype_uses(ty));
                    }
                    for instr in &func.instructions {
                        res.merge(get_instr_uses(instr));
                    }
                }
                res
            }
            WorkItem::Table(idx) => {
                let mut res = Uses::single_table(*idx);
                res.merge(get_tabletype_uses(&table_types[*idx as usize]));
                res
            }
            WorkItem::Global(idx) => {
                let mut res = Uses::single_global(*idx);
                res.merge(get_globaltype_uses(&global_types[*idx as usize]));
                res
            }
            WorkItem::Memory(idx) => Uses::single_memory(*idx),
            WorkItem::Data(_) => todo!(),
            WorkItem::Elem(_) => todo!(),
            WorkItem::Tag(_) => todo!(),
        };
        work_queue.remove(0);

        // Push all unused things to the queue
        for idx in &new_uses.live_types {
            if !all_uses.live_types.contains(idx) {
                work_queue.push(WorkItem::Type(*idx));
            }
        }
        for idx in &new_uses.live_funcs {
            if !all_uses.live_funcs.contains(idx) {
                work_queue.push(WorkItem::Func(*idx));
            }
        }
        for idx in &new_uses.live_tables {
            if !all_uses.live_tables.contains(idx) {
                work_queue.push(WorkItem::Table(*idx));
            }
        }
        for idx in &new_uses.live_globals {
            if !all_uses.live_globals.contains(idx) {
                work_queue.push(WorkItem::Global(*idx));
            }
        }
        for idx in &new_uses.live_memories {
            if !all_uses.live_memories.contains(idx) {
                work_queue.push(WorkItem::Memory(*idx));
            }
        }
        for idx in &new_uses.live_datas {
            if !all_uses.live_datas.contains(idx) {
                work_queue.push(WorkItem::Data(*idx));
            }
        }
        for idx in &new_uses.live_elems {
            if !all_uses.live_elems.contains(idx) {
                work_queue.push(WorkItem::Elem(*idx));
            }
        }
        for idx in &new_uses.live_tags {
            if !all_uses.live_tags.contains(idx) {
                work_queue.push(WorkItem::Tag(*idx));
            }
        }

        all_uses.merge(new_uses);
    }
    println!("{:#?}", all_uses);

    //
    // Track relocations
    //

    let mut relocations = HashMap::<Relocation, u32>::new();

    // TODO: We do not relocate types for now.
    // for type_idx in &all_uses.live_tables {
    //     let new_idx = get_new_index(&all_uses.live_types, type_idx);
    //     relocations.insert(Relocation::Type(*type_idx), new_idx);
    // }
    for func_idx in &all_uses.live_funcs {
        let new_idx = get_new_index(&all_uses.live_funcs, func_idx);
        relocations.insert(Relocation::Func(*func_idx), new_idx);
    }
    for table_idx in &all_uses.live_tables {
        let new_idx = get_new_index(&all_uses.live_tables, table_idx);
        relocations.insert(Relocation::Table(*table_idx), new_idx);
    }
    for global_idx in &all_uses.live_globals {
        let new_idx = get_new_index(&all_uses.live_globals, global_idx);
        relocations.insert(Relocation::Global(*global_idx), new_idx);
    }
    for mem_idx in &all_uses.live_memories {
        let new_idx = get_new_index(&all_uses.live_memories, mem_idx);
        relocations.insert(Relocation::Memory(*mem_idx), new_idx);
    }
    for data_idx in &all_uses.live_datas {
        let new_idx = get_new_index(&all_uses.live_datas, data_idx);
        relocations.insert(Relocation::Data(*data_idx), new_idx);
    }
    for elem_idx in &all_uses.live_elems {
        let new_idx = get_new_index(&all_uses.live_elems, elem_idx);
        relocations.insert(Relocation::Elem(*elem_idx), new_idx);
    }
    for tag_idx in &all_uses.live_tags {
        let new_idx = get_new_index(&all_uses.live_tags, tag_idx);
        relocations.insert(Relocation::Tag(*tag_idx), new_idx);
    }

    //
    // Output the new wasm module.
    //

    let mut out = Module::new();
    let mut reencoder = RelocatingReencoder {
        relocations: &relocations,
    };
    for section in sections {
        match section {
            Section::Passthrough(sec) => {
                out.section(&sec);
            }
            Section::Import => {
                let mut import_section = ImportSection::new();

                let mut num_imported_funcs = 0;
                let mut num_imported_tables = 0;
                let mut num_imported_memories = 0;
                let mut num_imported_globals = 0;
                let mut num_imported_tags = 0;
                for import in &imports {
                    match import.ty {
                        wasmparser::TypeRef::Func(type_idx) => {
                            if all_uses.live_funcs.contains(&num_imported_funcs) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    EntityType::Function(type_idx),
                                );
                            }
                            num_imported_funcs += 1;
                        }
                        wasmparser::TypeRef::Table(ty) => {
                            if all_uses.live_tables.contains(&num_imported_tables) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.table_type(ty)?,
                                );
                            }
                            num_imported_tables += 1;
                        }
                        wasmparser::TypeRef::Memory(ty) => {
                            if all_uses.live_memories.contains(&num_imported_memories) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.memory_type(ty),
                                );
                            }
                            num_imported_memories += 1;
                        }
                        wasmparser::TypeRef::Global(ty) => {
                            if all_uses.live_globals.contains(&num_imported_globals) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.global_type(ty)?,
                                );
                            }
                            num_imported_globals += 1;
                        }
                        wasmparser::TypeRef::Tag(ty) => {
                            if all_uses.live_tags.contains(&num_imported_tags) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.tag_type(ty),
                                );
                            }
                            num_imported_tags += 1;
                        }
                    }
                }

                out.section(&import_section);
            }
            Section::Function => {
                let mut function_section = FunctionSection::new();
                for (i, _) in defined_funcs.iter().enumerate() {
                    let idx = num_imported_functions + i as u32;
                    if relocations.get(&Relocation::Func(idx)).is_some() {
                        // TODO: Relocate...or don't? Depends what we do with types.
                        function_section.function(func_types[idx as usize]);
                    }
                }
                out.section(&function_section);
            }
            Section::Table => {
                let mut table_section = TableSection::new();
                for (i, table) in defined_tables.iter().enumerate() {
                    let idx = num_imported_tables + i as u32;
                    if relocations.get(&Relocation::Table(idx)).is_some() {
                        match &table.init {
                            wasmparser::TableInit::RefNull => {
                                table_section.table(reencoder.table_type(table.ty)?);
                            }
                            wasmparser::TableInit::Expr(init_expr) => {
                                // TODO: Re-encode the init expression with relocations.
                                table_section.table_with_init(
                                    reencoder.table_type(table.ty)?,
                                    &reencoder.const_expr(init_expr.clone())?,
                                );
                            }
                        }
                    }
                }
                out.section(&table_section);
            }
            Section::Memory => todo!(),
            Section::Global => {
                let mut global_section = GlobalSection::new();
                for (i, global) in defined_globals.iter().enumerate() {
                    let idx = num_imported_globals + i as u32;
                    if relocations.get(&Relocation::Global(idx)).is_some() {
                        // TODO: Re-encode the init expression with relocations.
                        global_section.global(
                            reencoder.global_type(global.ty)?,
                            &reencoder.const_expr(global.init_expr.clone())?,
                        );
                    }
                }
                out.section(&global_section);
            }
            Section::Export => {
                let mut export_section = ExportSection::new();
                for export in &exports {
                    let reloc = match export.kind {
                        wasmparser::ExternalKind::Func => Relocation::Func(export.index),
                        wasmparser::ExternalKind::Table => Relocation::Table(export.index),
                        wasmparser::ExternalKind::Memory => Relocation::Memory(export.index),
                        wasmparser::ExternalKind::Global => Relocation::Global(export.index),
                        wasmparser::ExternalKind::Tag => Relocation::Tag(export.index),
                    };
                    if let Some(new_idx) = relocations.get(&reloc) {
                        export_section.export(export.name, export.kind.into(), *new_idx);
                    }
                }
                out.section(&export_section);
            }
            Section::Start => {
                if let Some(idx) = start_idx {
                    if relocations.get(&Relocation::Func(idx)).is_some() {
                        out.section(&wasm_encoder::StartSection {
                            function_index: idx,
                        });
                    }
                }
            }
            Section::Element => {
                let mut element_section = ElementSection::new();
                for (i, elem) in elems.iter().enumerate() {
                    let idx = i as u32;
                    if relocations.get(&Relocation::Elem(idx)).is_some() {
                        let expr: ConstExpr;
                        element_section.segment(ElementSegment {
                            mode: match &elem.kind {
                                wasmparser::ElementKind::Passive => ElementMode::Passive,
                                wasmparser::ElementKind::Active {
                                    table_index,
                                    offset_expr,
                                } => {
                                    expr = reencoder.const_expr(offset_expr.clone())?;
                                    ElementMode::Active {
                                        table: *table_index,
                                        offset: &expr,
                                    }
                                }
                                wasmparser::ElementKind::Declared => todo!(),
                            },
                            elements: reencoder.element_items(elem.items.clone())?,
                        });
                    }
                }
                out.section(&element_section);
            }
            Section::Code => {
                let mut code_section = CodeSection::new();
                for (i, func) in defined_funcs.iter().enumerate() {
                    let idx = i as u32 + num_imported_functions;
                    if all_uses.live_funcs.contains(&idx) {
                        let mut new_locals: Vec<(u32, wasm_encoder::ValType)> = vec![];
                        for (n, ty) in &func.locals {
                            new_locals.push((*n, reencoder.val_type(*ty)?));
                        }
                        let mut new_func = Function::new(new_locals);
                        for instr in &func.instructions {
                            new_func.instruction(&reencoder.instruction(instr.clone())?);
                        }
                        code_section.function(&new_func);
                    }
                }
                out.section(&code_section);
            }
            Section::Data => {
                let mut data_section = DataSection::new();
                for (i, data) in datas.iter().enumerate() {
                    let idx = i as u32;
                    if relocations.get(&Relocation::Data(idx)).is_some() {
                        let expr: ConstExpr;
                        data_section.segment(DataSegment {
                            mode: match &data.kind {
                                wasmparser::DataKind::Passive => DataSegmentMode::Passive,
                                wasmparser::DataKind::Active {
                                    memory_index,
                                    offset_expr,
                                } => {
                                    expr = reencoder.const_expr(offset_expr.clone())?;
                                    DataSegmentMode::Active {
                                        memory_index: *memory_index,
                                        offset: &expr,
                                    }
                                }
                            },
                            data: vec![],
                        });
                    }
                }
                out.section(&data_section);
            }
            Section::DataCount => todo!(),
            Section::Tag => todo!(),
        }
    }
    let out_bytes = out.finish();

    fs::write(args.out, out_bytes).expect("unable to write file");

    Ok(())
}

fn get_new_index(live_things: &Vec<u32>, idx: &u32) -> u32 {
    live_things
        .iter()
        .position(|&v| v == *idx)
        .expect("original index should have been in vec") as u32
}

fn get_reader(filename: String) -> Box<dyn std::io::Read> {
    if filename == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(File::open(filename).expect("Failed to open file"))
    }
}

struct Func<'a> {
    type_idx: u32,
    locals: Vec<(u32, ValType)>,
    instructions: Vec<Operator<'a>>,
}

enum WorkItem {
    Type(u32),
    Func(u32),
    Table(u32),
    Global(u32),
    Memory(u32),
    Data(u32),
    Elem(u32),
    Tag(u32),
}

enum Section<'a> {
    Passthrough(RawSection<'a>),
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
    fn raw(id: u8, bytes: &'a [u8]) -> Section<'a> {
        let foo = RawSection {
            id: id,
            data: bytes,
        };
        Self::Passthrough(foo)
    }
}
