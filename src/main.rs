use std::{
    collections::HashMap,
    fs::{self, File},
};

use anyhow::Result;
use clap::Parser as _;
use wasm_encoder::{
    reencode::{Reencode, RoundtripReencoder},
    CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode, ElementMode, ElementSection,
    ElementSegment, EntityType, ExportSection, Function, FunctionSection, GlobalSection,
    ImportSection, Instruction, Module, RawSection, StartSection, TableSection,
};
use wasmparser::{
    ArrayType, BlockType, Catch, CompositeInnerType, Data, Element, Export, FieldType, FuncType,
    Global, GlobalType, HeapType, Import, MemArg, MemoryType, Operator, Parser, Payload::*,
    RefType, StorageType, StructType, Table, TableType, TagType, ValType,
};

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
            wasmparser::Payload::StartSection { func, range: _ } => {
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
                sections.push(Section::raw(12, &buf[range]));
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

    for type_idx in &all_uses.live_tables {
        let new_idx = get_new_index(&all_uses.live_types, type_idx);
        relocations.insert(Relocation::Type(*type_idx), new_idx);
    }
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
                                    RoundtripReencoder.table_type(ty)?,
                                );
                            }
                            num_imported_tables += 1;
                        }
                        wasmparser::TypeRef::Memory(ty) => {
                            if all_uses.live_memories.contains(&num_imported_memories) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    RoundtripReencoder.memory_type(ty),
                                );
                            }
                            num_imported_memories += 1;
                        }
                        wasmparser::TypeRef::Global(ty) => {
                            if all_uses.live_globals.contains(&num_imported_globals) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    RoundtripReencoder.global_type(ty)?,
                                );
                            }
                            num_imported_globals += 1;
                        }
                        wasmparser::TypeRef::Tag(ty) => {
                            if all_uses.live_tags.contains(&num_imported_tags) {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    RoundtripReencoder.tag_type(ty),
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
                                table_section.table(RoundtripReencoder.table_type(table.ty)?);
                            }
                            wasmparser::TableInit::Expr(init_expr) => {
                                // TODO: Re-encode the init expression with relocations.
                                table_section.table_with_init(
                                    RoundtripReencoder.table_type(table.ty)?,
                                    &RoundtripReencoder.const_expr(init_expr.clone())?,
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
                            RoundtripReencoder.global_type(global.ty)?,
                            &RoundtripReencoder.const_expr(global.init_expr.clone())?,
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
                        out.section(&StartSection {
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
                                    expr = RoundtripReencoder.const_expr(offset_expr.clone())?;
                                    ElementMode::Active {
                                        table: *table_index,
                                        offset: &expr,
                                    }
                                }
                                wasmparser::ElementKind::Declared => todo!(),
                            },
                            elements: RoundtripReencoder.element_items(elem.items.clone())?,
                        });
                    }
                }
                out.section(&element_section);
            }
            Section::Code => {
                let mut code_section = CodeSection::new();
                for (i, _) in defined_funcs.iter().enumerate() {
                    let idx = i as u32 + num_imported_functions;
                    if all_uses.live_funcs.contains(&idx) {
                        // TODO: locals
                        let mut new_func = Function::new(vec![]);
                        new_func.instruction(&Instruction::End);
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
                                    expr = RoundtripReencoder.const_expr(offset_expr.clone())?;
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

#[derive(Default, Debug)]
struct Uses {
    live_types: Vec<u32>,
    live_funcs: Vec<u32>,
    live_tables: Vec<u32>,
    live_globals: Vec<u32>,
    live_memories: Vec<u32>,
    live_datas: Vec<u32>,
    live_elems: Vec<u32>,
    live_tags: Vec<u32>,
}

impl Uses {
    fn single_type(idx: u32) -> Uses {
        return Self {
            live_types: vec![idx],
            ..Default::default()
        };
    }

    fn single_func(idx: u32) -> Uses {
        return Self {
            live_funcs: vec![idx],
            ..Default::default()
        };
    }

    fn single_table(idx: u32) -> Uses {
        return Self {
            live_tables: vec![idx],
            ..Default::default()
        };
    }

    fn single_global(idx: u32) -> Uses {
        return Self {
            live_globals: vec![idx],
            ..Default::default()
        };
    }

    fn single_memory(idx: u32) -> Uses {
        return Self {
            live_memories: vec![idx],
            ..Default::default()
        };
    }

    fn single_data(idx: u32) -> Uses {
        return Self {
            live_datas: vec![idx],
            ..Default::default()
        };
    }

    fn single_elem(idx: u32) -> Uses {
        return Self {
            live_elems: vec![idx],
            ..Default::default()
        };
    }

    fn single_tag(idx: u32) -> Uses {
        return Self {
            live_tags: vec![idx],
            ..Default::default()
        };
    }

    fn all<I>(useses: I) -> Uses
    where
        I: IntoIterator<Item = Uses>,
    {
        let mut res = Uses::default();
        for uses in useses {
            res.merge(uses);
        }
        res
    }

    fn merge(&mut self, mut other: Uses) {
        Self::append_and_dedup(&mut self.live_types, &mut other.live_types);
        Self::append_and_dedup(&mut self.live_funcs, &mut other.live_funcs);
        Self::append_and_dedup(&mut self.live_tables, &mut other.live_tables);
        Self::append_and_dedup(&mut self.live_globals, &mut other.live_globals);
        Self::append_and_dedup(&mut self.live_memories, &mut other.live_memories);
        Self::append_and_dedup(&mut self.live_datas, &mut other.live_datas);
        Self::append_and_dedup(&mut self.live_elems, &mut other.live_elems);
        Self::append_and_dedup(&mut self.live_tags, &mut other.live_tags);
    }

    fn append_and_dedup(vec: &mut Vec<u32>, other: &mut Vec<u32>) {
        vec.append(other);
        vec.sort();
        vec.dedup();
    }
}

fn get_type_uses(ty: &CompositeInnerType) -> Uses {
    match ty {
        CompositeInnerType::Func(func_type) => get_functype_uses(func_type),
        CompositeInnerType::Array(array_type) => get_arraytype_uses(array_type),
        CompositeInnerType::Struct(struct_type) => get_structtype_uses(struct_type),
        CompositeInnerType::Cont(_) => todo!(),
    }
}

fn get_functype_uses(ty: &FuncType) -> Uses {
    let mut res = Uses::default();
    for vt in ty.params() {
        res.merge(get_valtype_uses(vt));
    }
    for vt in ty.results() {
        res.merge(get_valtype_uses(vt));
    }
    res
}

fn get_arraytype_uses(ty: &ArrayType) -> Uses {
    get_fieldtype_uses(&ty.0)
}

fn get_structtype_uses(ty: &StructType) -> Uses {
    let mut res = Uses::default();
    for f in ty.fields.iter() {
        res.merge(get_fieldtype_uses(f));
    }
    res
}

fn get_fieldtype_uses(ty: &FieldType) -> Uses {
    get_storagetype_uses(&ty.element_type)
}

fn get_storagetype_uses(ty: &StorageType) -> Uses {
    match ty {
        StorageType::I8 | StorageType::I16 => Uses::default(),
        StorageType::Val(val_type) => get_valtype_uses(val_type),
    }
}

fn get_tabletype_uses(ty: &TableType) -> Uses {
    get_reftype_uses(&ty.element_type)
}

fn get_globaltype_uses(ty: &GlobalType) -> Uses {
    get_valtype_uses(&ty.content_type)
}

fn get_valtype_uses(ty: &ValType) -> Uses {
    match ty {
        ValType::Ref(ref_type) => get_reftype_uses(ref_type),
        _ => Uses::default(),
    }
}

fn get_reftype_uses(ty: &RefType) -> Uses {
    return get_heaptype_uses(&ty.heap_type());
}

fn get_heaptype_uses(ty: &HeapType) -> Uses {
    match ty {
        wasmparser::HeapType::Abstract { .. } => Uses::default(),
        wasmparser::HeapType::Concrete(idx) => match idx {
            wasmparser::UnpackedIndex::Module(idx) => Uses::single_type(*idx),
            _ => todo!(),
        },
    }
}

fn get_blocktype_uses(blockty: &BlockType) -> Uses {
    match blockty {
        BlockType::Empty => Uses::default(),
        BlockType::Type(val_type) => get_valtype_uses(val_type),
        BlockType::FuncType(ty) => Uses::single_type(*ty),
    }
}

fn get_memarg_uses(memarg: &MemArg) -> Uses {
    return Uses::single_memory(memarg.memory);
}

fn get_catch_uses(catch: &Catch) -> Uses {
    match catch {
        Catch::One { tag, label: _ } => Uses::single_tag(*tag),
        Catch::OneRef { tag, label: _ } => Uses::single_tag(*tag),
        Catch::All { label: _ } => Uses::default(),
        Catch::AllRef { label: _ } => Uses::default(),
    }
}

fn get_instr_uses(instr: &Operator<'_>) -> Uses {
    match instr {
        Operator::Unreachable => Uses::default(),
        Operator::Nop => Uses::default(),
        Operator::Block { blockty } => get_blocktype_uses(blockty),
        Operator::Loop { blockty } => get_blocktype_uses(blockty),
        Operator::If { blockty } => get_blocktype_uses(blockty),
        Operator::Else => Uses::default(),
        Operator::End => Uses::default(),
        Operator::Br { relative_depth: _ } => Uses::default(),
        Operator::BrIf { relative_depth: _ } => Uses::default(),
        Operator::BrTable { targets: _ } => Uses::default(),
        Operator::Return => Uses::default(),
        Operator::Call { function_index } => Uses::single_func(*function_index),
        Operator::CallIndirect {
            type_index,
            table_index,
        } => Uses {
            live_types: vec![*type_index],
            live_tables: vec![*table_index],
            ..Default::default()
        },
        Operator::Drop => Uses::default(),
        Operator::Select => Uses::default(),
        Operator::LocalGet { local_index: _ } => Uses::default(),
        Operator::LocalSet { local_index: _ } => Uses::default(),
        Operator::LocalTee { local_index: _ } => Uses::default(),
        Operator::GlobalGet { global_index } => Uses::single_global(*global_index),
        Operator::GlobalSet { global_index } => Uses::single_global(*global_index),
        Operator::I32Load { memarg } => get_memarg_uses(memarg),
        Operator::I64Load { memarg } => get_memarg_uses(memarg),
        Operator::F32Load { memarg } => get_memarg_uses(memarg),
        Operator::F64Load { memarg } => get_memarg_uses(memarg),
        Operator::I32Load8S { memarg } => get_memarg_uses(memarg),
        Operator::I32Load8U { memarg } => get_memarg_uses(memarg),
        Operator::I32Load16S { memarg } => get_memarg_uses(memarg),
        Operator::I32Load16U { memarg } => get_memarg_uses(memarg),
        Operator::I64Load8S { memarg } => get_memarg_uses(memarg),
        Operator::I64Load8U { memarg } => get_memarg_uses(memarg),
        Operator::I64Load16S { memarg } => get_memarg_uses(memarg),
        Operator::I64Load16U { memarg } => get_memarg_uses(memarg),
        Operator::I64Load32S { memarg } => get_memarg_uses(memarg),
        Operator::I64Load32U { memarg } => get_memarg_uses(memarg),
        Operator::I32Store { memarg } => get_memarg_uses(memarg),
        Operator::I64Store { memarg } => get_memarg_uses(memarg),
        Operator::F32Store { memarg } => get_memarg_uses(memarg),
        Operator::F64Store { memarg } => get_memarg_uses(memarg),
        Operator::I32Store8 { memarg } => get_memarg_uses(memarg),
        Operator::I32Store16 { memarg } => get_memarg_uses(memarg),
        Operator::I64Store8 { memarg } => get_memarg_uses(memarg),
        Operator::I64Store16 { memarg } => get_memarg_uses(memarg),
        Operator::I64Store32 { memarg } => get_memarg_uses(memarg),
        Operator::MemorySize { mem } => Uses::single_memory(*mem),
        Operator::MemoryGrow { mem } => Uses::single_memory(*mem),
        Operator::I32Const { value: _ } => Uses::default(),
        Operator::I64Const { value: _ } => Uses::default(),
        Operator::F32Const { value: _ } => Uses::default(),
        Operator::F64Const { value: _ } => Uses::default(),
        Operator::I32Eqz => Uses::default(),
        Operator::I32Eq => Uses::default(),
        Operator::I32Ne => Uses::default(),
        Operator::I32LtS => Uses::default(),
        Operator::I32LtU => Uses::default(),
        Operator::I32GtS => Uses::default(),
        Operator::I32GtU => Uses::default(),
        Operator::I32LeS => Uses::default(),
        Operator::I32LeU => Uses::default(),
        Operator::I32GeS => Uses::default(),
        Operator::I32GeU => Uses::default(),
        Operator::I64Eqz => Uses::default(),
        Operator::I64Eq => Uses::default(),
        Operator::I64Ne => Uses::default(),
        Operator::I64LtS => Uses::default(),
        Operator::I64LtU => Uses::default(),
        Operator::I64GtS => Uses::default(),
        Operator::I64GtU => Uses::default(),
        Operator::I64LeS => Uses::default(),
        Operator::I64LeU => Uses::default(),
        Operator::I64GeS => Uses::default(),
        Operator::I64GeU => Uses::default(),
        Operator::F32Eq => Uses::default(),
        Operator::F32Ne => Uses::default(),
        Operator::F32Lt => Uses::default(),
        Operator::F32Gt => Uses::default(),
        Operator::F32Le => Uses::default(),
        Operator::F32Ge => Uses::default(),
        Operator::F64Eq => Uses::default(),
        Operator::F64Ne => Uses::default(),
        Operator::F64Lt => Uses::default(),
        Operator::F64Gt => Uses::default(),
        Operator::F64Le => Uses::default(),
        Operator::F64Ge => Uses::default(),
        Operator::I32Clz => Uses::default(),
        Operator::I32Ctz => Uses::default(),
        Operator::I32Popcnt => Uses::default(),
        Operator::I32Add => Uses::default(),
        Operator::I32Sub => Uses::default(),
        Operator::I32Mul => Uses::default(),
        Operator::I32DivS => Uses::default(),
        Operator::I32DivU => Uses::default(),
        Operator::I32RemS => Uses::default(),
        Operator::I32RemU => Uses::default(),
        Operator::I32And => Uses::default(),
        Operator::I32Or => Uses::default(),
        Operator::I32Xor => Uses::default(),
        Operator::I32Shl => Uses::default(),
        Operator::I32ShrS => Uses::default(),
        Operator::I32ShrU => Uses::default(),
        Operator::I32Rotl => Uses::default(),
        Operator::I32Rotr => Uses::default(),
        Operator::I64Clz => Uses::default(),
        Operator::I64Ctz => Uses::default(),
        Operator::I64Popcnt => Uses::default(),
        Operator::I64Add => Uses::default(),
        Operator::I64Sub => Uses::default(),
        Operator::I64Mul => Uses::default(),
        Operator::I64DivS => Uses::default(),
        Operator::I64DivU => Uses::default(),
        Operator::I64RemS => Uses::default(),
        Operator::I64RemU => Uses::default(),
        Operator::I64And => Uses::default(),
        Operator::I64Or => Uses::default(),
        Operator::I64Xor => Uses::default(),
        Operator::I64Shl => Uses::default(),
        Operator::I64ShrS => Uses::default(),
        Operator::I64ShrU => Uses::default(),
        Operator::I64Rotl => Uses::default(),
        Operator::I64Rotr => Uses::default(),
        Operator::F32Abs => Uses::default(),
        Operator::F32Neg => Uses::default(),
        Operator::F32Ceil => Uses::default(),
        Operator::F32Floor => Uses::default(),
        Operator::F32Trunc => Uses::default(),
        Operator::F32Nearest => Uses::default(),
        Operator::F32Sqrt => Uses::default(),
        Operator::F32Add => Uses::default(),
        Operator::F32Sub => Uses::default(),
        Operator::F32Mul => Uses::default(),
        Operator::F32Div => Uses::default(),
        Operator::F32Min => Uses::default(),
        Operator::F32Max => Uses::default(),
        Operator::F32Copysign => Uses::default(),
        Operator::F64Abs => Uses::default(),
        Operator::F64Neg => Uses::default(),
        Operator::F64Ceil => Uses::default(),
        Operator::F64Floor => Uses::default(),
        Operator::F64Trunc => Uses::default(),
        Operator::F64Nearest => Uses::default(),
        Operator::F64Sqrt => Uses::default(),
        Operator::F64Add => Uses::default(),
        Operator::F64Sub => Uses::default(),
        Operator::F64Mul => Uses::default(),
        Operator::F64Div => Uses::default(),
        Operator::F64Min => Uses::default(),
        Operator::F64Max => Uses::default(),
        Operator::F64Copysign => Uses::default(),
        Operator::I32WrapI64 => Uses::default(),
        Operator::I32TruncF32S => Uses::default(),
        Operator::I32TruncF32U => Uses::default(),
        Operator::I32TruncF64S => Uses::default(),
        Operator::I32TruncF64U => Uses::default(),
        Operator::I64ExtendI32S => Uses::default(),
        Operator::I64ExtendI32U => Uses::default(),
        Operator::I64TruncF32S => Uses::default(),
        Operator::I64TruncF32U => Uses::default(),
        Operator::I64TruncF64S => Uses::default(),
        Operator::I64TruncF64U => Uses::default(),
        Operator::F32ConvertI32S => Uses::default(),
        Operator::F32ConvertI32U => Uses::default(),
        Operator::F32ConvertI64S => Uses::default(),
        Operator::F32ConvertI64U => Uses::default(),
        Operator::F32DemoteF64 => Uses::default(),
        Operator::F64ConvertI32S => Uses::default(),
        Operator::F64ConvertI32U => Uses::default(),
        Operator::F64ConvertI64S => Uses::default(),
        Operator::F64ConvertI64U => Uses::default(),
        Operator::F64PromoteF32 => Uses::default(),
        Operator::I32ReinterpretF32 => Uses::default(),
        Operator::I64ReinterpretF64 => Uses::default(),
        Operator::F32ReinterpretI32 => Uses::default(),
        Operator::F64ReinterpretI64 => Uses::default(),
        Operator::I32Extend8S => Uses::default(),
        Operator::I32Extend16S => Uses::default(),
        Operator::I64Extend8S => Uses::default(),
        Operator::I64Extend16S => Uses::default(),
        Operator::I64Extend32S => Uses::default(),
        Operator::RefEq => Uses::default(),
        Operator::StructNew { struct_type_index } => Uses::single_type(*struct_type_index),
        Operator::StructNewDefault { struct_type_index } => Uses::single_type(*struct_type_index),
        Operator::StructGet {
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructGetS {
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructGetU {
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructSet {
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::ArrayNew { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayNewDefault { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayNewFixed {
            array_type_index,
            array_size: _,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayNewData {
            array_type_index,
            array_data_index,
        } => Uses {
            live_types: vec![*array_type_index],
            live_datas: vec![*array_data_index],
            ..Default::default()
        },
        Operator::ArrayNewElem {
            array_type_index,
            array_elem_index,
        } => Uses {
            live_types: vec![*array_type_index],
            live_elems: vec![*array_elem_index],
            ..Default::default()
        },
        Operator::ArrayGet { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayGetS { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayGetU { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArraySet { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayLen => Uses::default(),
        Operator::ArrayFill { array_type_index } => Uses::single_type(*array_type_index),
        Operator::ArrayCopy {
            array_type_index_dst,
            array_type_index_src,
        } => Uses {
            live_types: vec![*array_type_index_dst, *array_type_index_src],
            ..Default::default()
        },
        Operator::ArrayInitData {
            array_type_index,
            array_data_index,
        } => Uses {
            live_types: vec![*array_type_index],
            live_datas: vec![*array_data_index],
            ..Default::default()
        },
        Operator::ArrayInitElem {
            array_type_index,
            array_elem_index,
        } => Uses {
            live_types: vec![*array_type_index],
            live_elems: vec![*array_elem_index],
            ..Default::default()
        },
        Operator::RefTestNonNull { hty } => get_heaptype_uses(hty),
        Operator::RefTestNullable { hty } => get_heaptype_uses(hty),
        Operator::RefCastNonNull { hty } => get_heaptype_uses(hty),
        Operator::RefCastNullable { hty } => get_heaptype_uses(hty),
        Operator::BrOnCast {
            relative_depth: _,
            from_ref_type,
            to_ref_type,
        } => Uses::all(vec![
            get_reftype_uses(from_ref_type),
            get_reftype_uses(to_ref_type),
        ]),
        Operator::BrOnCastFail {
            relative_depth: _,
            from_ref_type,
            to_ref_type,
        } => Uses::all(vec![
            get_reftype_uses(from_ref_type),
            get_reftype_uses(to_ref_type),
        ]),
        Operator::AnyConvertExtern => Uses::default(),
        Operator::ExternConvertAny => Uses::default(),
        Operator::RefI31 => Uses::default(),
        Operator::I31GetS => Uses::default(),
        Operator::I31GetU => Uses::default(),
        Operator::I32TruncSatF32S => Uses::default(),
        Operator::I32TruncSatF32U => Uses::default(),
        Operator::I32TruncSatF64S => Uses::default(),
        Operator::I32TruncSatF64U => Uses::default(),
        Operator::I64TruncSatF32S => Uses::default(),
        Operator::I64TruncSatF32U => Uses::default(),
        Operator::I64TruncSatF64S => Uses::default(),
        Operator::I64TruncSatF64U => Uses::default(),
        Operator::MemoryInit { data_index, mem } => Uses {
            live_datas: vec![*data_index],
            live_memories: vec![*mem],
            ..Default::default()
        },
        Operator::DataDrop { data_index } => Uses::single_data(*data_index),
        Operator::MemoryCopy { dst_mem, src_mem } => Uses {
            live_memories: vec![*dst_mem, *src_mem],
            ..Default::default()
        },
        Operator::MemoryFill { mem } => Uses::single_memory(*mem),
        Operator::TableInit { elem_index, table } => Uses {
            live_elems: vec![*elem_index],
            live_tables: vec![*table],
            ..Default::default()
        },
        Operator::ElemDrop { elem_index } => Uses::single_elem(*elem_index),
        Operator::TableCopy {
            dst_table,
            src_table,
        } => Uses {
            live_tables: vec![*dst_table, *src_table],
            ..Default::default()
        },
        Operator::TypedSelect { ty } => get_valtype_uses(ty),
        Operator::RefNull { hty } => get_heaptype_uses(hty),
        Operator::RefIsNull => Uses::default(),
        Operator::RefFunc { function_index } => Uses::single_func(*function_index),
        Operator::TableFill { table } => Uses::single_table(*table),
        Operator::TableGet { table } => Uses::single_table(*table),
        Operator::TableSet { table } => Uses::single_table(*table),
        Operator::TableGrow { table } => Uses::single_table(*table),
        Operator::TableSize { table } => Uses::single_table(*table),
        Operator::ReturnCall { function_index } => Uses::single_func(*function_index),
        Operator::ReturnCallIndirect {
            type_index,
            table_index,
        } => Uses {
            live_types: vec![*type_index],
            live_tables: vec![*table_index],
            ..Default::default()
        },
        Operator::MemoryDiscard { mem } => Uses::single_memory(*mem),
        Operator::MemoryAtomicNotify { memarg } => get_memarg_uses(memarg),
        Operator::MemoryAtomicWait32 { memarg } => get_memarg_uses(memarg),
        Operator::MemoryAtomicWait64 { memarg } => get_memarg_uses(memarg),
        Operator::AtomicFence => Uses::default(),
        Operator::I32AtomicLoad { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicLoad { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicLoad8U { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicLoad16U { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicLoad8U { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicLoad16U { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicLoad32U { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicStore { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicStore { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicStore8 { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicStore16 { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicStore8 { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicStore16 { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicStore32 { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwAdd { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwAdd { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8AddU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16AddU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8AddU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16AddU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32AddU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwSub { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwSub { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8SubU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16SubU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8SubU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16SubU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32SubU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwAnd { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwAnd { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8AndU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16AndU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8AndU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16AndU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32AndU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwOr { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwOr { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8OrU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16OrU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8OrU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16OrU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32OrU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwXor { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwXor { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8XorU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16XorU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8XorU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16XorU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32XorU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwXchg { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwXchg { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8XchgU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16XchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8XchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16XchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32XchgU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmwCmpxchg { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmwCmpxchg { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw8CmpxchgU { memarg } => get_memarg_uses(memarg),
        Operator::I32AtomicRmw16CmpxchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw8CmpxchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw16CmpxchgU { memarg } => get_memarg_uses(memarg),
        Operator::I64AtomicRmw32CmpxchgU { memarg } => get_memarg_uses(memarg),
        Operator::V128Load { memarg } => get_memarg_uses(memarg),
        Operator::V128Load8x8S { memarg } => get_memarg_uses(memarg),
        Operator::V128Load8x8U { memarg } => get_memarg_uses(memarg),
        Operator::V128Load16x4S { memarg } => get_memarg_uses(memarg),
        Operator::V128Load16x4U { memarg } => get_memarg_uses(memarg),
        Operator::V128Load32x2S { memarg } => get_memarg_uses(memarg),
        Operator::V128Load32x2U { memarg } => get_memarg_uses(memarg),
        Operator::V128Load8Splat { memarg } => get_memarg_uses(memarg),
        Operator::V128Load16Splat { memarg } => get_memarg_uses(memarg),
        Operator::V128Load32Splat { memarg } => get_memarg_uses(memarg),
        Operator::V128Load64Splat { memarg } => get_memarg_uses(memarg),
        Operator::V128Load32Zero { memarg } => get_memarg_uses(memarg),
        Operator::V128Load64Zero { memarg } => get_memarg_uses(memarg),
        Operator::V128Store { memarg } => get_memarg_uses(memarg),
        Operator::V128Load8Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Load16Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Load32Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Load64Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Store8Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Store16Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Store32Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Store64Lane { memarg, lane: _ } => get_memarg_uses(memarg),
        Operator::V128Const { value: _ } => Uses::default(),
        Operator::I8x16Swizzle => Uses::default(),
        Operator::I8x16Splat => Uses::default(),
        Operator::I16x8Splat => Uses::default(),
        Operator::I32x4Splat => Uses::default(),
        Operator::I64x2Splat => Uses::default(),
        Operator::F32x4Splat => Uses::default(),
        Operator::F64x2Splat => Uses::default(),
        Operator::I8x16Eq => Uses::default(),
        Operator::I8x16Ne => Uses::default(),
        Operator::I8x16LtS => Uses::default(),
        Operator::I8x16LtU => Uses::default(),
        Operator::I8x16GtS => Uses::default(),
        Operator::I8x16GtU => Uses::default(),
        Operator::I8x16LeS => Uses::default(),
        Operator::I8x16LeU => Uses::default(),
        Operator::I8x16GeS => Uses::default(),
        Operator::I8x16GeU => Uses::default(),
        Operator::I16x8Eq => Uses::default(),
        Operator::I16x8Ne => Uses::default(),
        Operator::I16x8LtS => Uses::default(),
        Operator::I16x8LtU => Uses::default(),
        Operator::I16x8GtS => Uses::default(),
        Operator::I16x8GtU => Uses::default(),
        Operator::I16x8LeS => Uses::default(),
        Operator::I16x8LeU => Uses::default(),
        Operator::I16x8GeS => Uses::default(),
        Operator::I16x8GeU => Uses::default(),
        Operator::I32x4Eq => Uses::default(),
        Operator::I32x4Ne => Uses::default(),
        Operator::I32x4LtS => Uses::default(),
        Operator::I32x4LtU => Uses::default(),
        Operator::I32x4GtS => Uses::default(),
        Operator::I32x4GtU => Uses::default(),
        Operator::I32x4LeS => Uses::default(),
        Operator::I32x4LeU => Uses::default(),
        Operator::I32x4GeS => Uses::default(),
        Operator::I32x4GeU => Uses::default(),
        Operator::I64x2Eq => Uses::default(),
        Operator::I64x2Ne => Uses::default(),
        Operator::I64x2LtS => Uses::default(),
        Operator::I64x2GtS => Uses::default(),
        Operator::I64x2LeS => Uses::default(),
        Operator::I64x2GeS => Uses::default(),
        Operator::F32x4Eq => Uses::default(),
        Operator::F32x4Ne => Uses::default(),
        Operator::F32x4Lt => Uses::default(),
        Operator::F32x4Gt => Uses::default(),
        Operator::F32x4Le => Uses::default(),
        Operator::F32x4Ge => Uses::default(),
        Operator::F64x2Eq => Uses::default(),
        Operator::F64x2Ne => Uses::default(),
        Operator::F64x2Lt => Uses::default(),
        Operator::F64x2Gt => Uses::default(),
        Operator::F64x2Le => Uses::default(),
        Operator::F64x2Ge => Uses::default(),
        Operator::V128Not => Uses::default(),
        Operator::V128And => Uses::default(),
        Operator::V128AndNot => Uses::default(),
        Operator::V128Or => Uses::default(),
        Operator::V128Xor => Uses::default(),
        Operator::V128Bitselect => Uses::default(),
        Operator::V128AnyTrue => Uses::default(),
        Operator::I8x16Abs => Uses::default(),
        Operator::I8x16Neg => Uses::default(),
        Operator::I8x16Popcnt => Uses::default(),
        Operator::I8x16AllTrue => Uses::default(),
        Operator::I8x16Bitmask => Uses::default(),
        Operator::I8x16NarrowI16x8S => Uses::default(),
        Operator::I8x16NarrowI16x8U => Uses::default(),
        Operator::I8x16Shl => Uses::default(),
        Operator::I8x16ShrS => Uses::default(),
        Operator::I8x16ShrU => Uses::default(),
        Operator::I8x16Add => Uses::default(),
        Operator::I8x16AddSatS => Uses::default(),
        Operator::I8x16AddSatU => Uses::default(),
        Operator::I8x16Sub => Uses::default(),
        Operator::I8x16SubSatS => Uses::default(),
        Operator::I8x16SubSatU => Uses::default(),
        Operator::I8x16MinS => Uses::default(),
        Operator::I8x16MinU => Uses::default(),
        Operator::I8x16MaxS => Uses::default(),
        Operator::I8x16MaxU => Uses::default(),
        Operator::I8x16AvgrU => Uses::default(),
        Operator::I16x8ExtAddPairwiseI8x16S => Uses::default(),
        Operator::I16x8ExtAddPairwiseI8x16U => Uses::default(),
        Operator::I16x8Abs => Uses::default(),
        Operator::I16x8Neg => Uses::default(),
        Operator::I16x8Q15MulrSatS => Uses::default(),
        Operator::I16x8AllTrue => Uses::default(),
        Operator::I16x8Bitmask => Uses::default(),
        Operator::I16x8NarrowI32x4S => Uses::default(),
        Operator::I16x8NarrowI32x4U => Uses::default(),
        Operator::I16x8ExtendLowI8x16S => Uses::default(),
        Operator::I16x8ExtendHighI8x16S => Uses::default(),
        Operator::I16x8ExtendLowI8x16U => Uses::default(),
        Operator::I16x8ExtendHighI8x16U => Uses::default(),
        Operator::I16x8Shl => Uses::default(),
        Operator::I16x8ShrS => Uses::default(),
        Operator::I16x8ShrU => Uses::default(),
        Operator::I16x8Add => Uses::default(),
        Operator::I16x8AddSatS => Uses::default(),
        Operator::I16x8AddSatU => Uses::default(),
        Operator::I16x8Sub => Uses::default(),
        Operator::I16x8SubSatS => Uses::default(),
        Operator::I16x8SubSatU => Uses::default(),
        Operator::I16x8Mul => Uses::default(),
        Operator::I16x8MinS => Uses::default(),
        Operator::I16x8MinU => Uses::default(),
        Operator::I16x8MaxS => Uses::default(),
        Operator::I16x8MaxU => Uses::default(),
        Operator::I16x8AvgrU => Uses::default(),
        Operator::I16x8ExtMulLowI8x16S => Uses::default(),
        Operator::I16x8ExtMulHighI8x16S => Uses::default(),
        Operator::I16x8ExtMulLowI8x16U => Uses::default(),
        Operator::I16x8ExtMulHighI8x16U => Uses::default(),
        Operator::I32x4ExtAddPairwiseI16x8S => Uses::default(),
        Operator::I32x4ExtAddPairwiseI16x8U => Uses::default(),
        Operator::I32x4Abs => Uses::default(),
        Operator::I32x4Neg => Uses::default(),
        Operator::I32x4AllTrue => Uses::default(),
        Operator::I32x4Bitmask => Uses::default(),
        Operator::I32x4ExtendLowI16x8S => Uses::default(),
        Operator::I32x4ExtendHighI16x8S => Uses::default(),
        Operator::I32x4ExtendLowI16x8U => Uses::default(),
        Operator::I32x4ExtendHighI16x8U => Uses::default(),
        Operator::I32x4Shl => Uses::default(),
        Operator::I32x4ShrS => Uses::default(),
        Operator::I32x4ShrU => Uses::default(),
        Operator::I32x4Add => Uses::default(),
        Operator::I32x4Sub => Uses::default(),
        Operator::I32x4Mul => Uses::default(),
        Operator::I32x4MinS => Uses::default(),
        Operator::I32x4MinU => Uses::default(),
        Operator::I32x4MaxS => Uses::default(),
        Operator::I32x4MaxU => Uses::default(),
        Operator::I32x4DotI16x8S => Uses::default(),
        Operator::I32x4ExtMulLowI16x8S => Uses::default(),
        Operator::I32x4ExtMulHighI16x8S => Uses::default(),
        Operator::I32x4ExtMulLowI16x8U => Uses::default(),
        Operator::I32x4ExtMulHighI16x8U => Uses::default(),
        Operator::I64x2Abs => Uses::default(),
        Operator::I64x2Neg => Uses::default(),
        Operator::I64x2AllTrue => Uses::default(),
        Operator::I64x2Bitmask => Uses::default(),
        Operator::I64x2ExtendLowI32x4S => Uses::default(),
        Operator::I64x2ExtendHighI32x4S => Uses::default(),
        Operator::I64x2ExtendLowI32x4U => Uses::default(),
        Operator::I64x2ExtendHighI32x4U => Uses::default(),
        Operator::I64x2Shl => Uses::default(),
        Operator::I64x2ShrS => Uses::default(),
        Operator::I64x2ShrU => Uses::default(),
        Operator::I64x2Add => Uses::default(),
        Operator::I64x2Sub => Uses::default(),
        Operator::I64x2Mul => Uses::default(),
        Operator::I64x2ExtMulLowI32x4S => Uses::default(),
        Operator::I64x2ExtMulHighI32x4S => Uses::default(),
        Operator::I64x2ExtMulLowI32x4U => Uses::default(),
        Operator::I64x2ExtMulHighI32x4U => Uses::default(),
        Operator::F32x4Ceil => Uses::default(),
        Operator::F32x4Floor => Uses::default(),
        Operator::F32x4Trunc => Uses::default(),
        Operator::F32x4Nearest => Uses::default(),
        Operator::F32x4Abs => Uses::default(),
        Operator::F32x4Neg => Uses::default(),
        Operator::F32x4Sqrt => Uses::default(),
        Operator::F32x4Add => Uses::default(),
        Operator::F32x4Sub => Uses::default(),
        Operator::F32x4Mul => Uses::default(),
        Operator::F32x4Div => Uses::default(),
        Operator::F32x4Min => Uses::default(),
        Operator::F32x4Max => Uses::default(),
        Operator::F32x4PMin => Uses::default(),
        Operator::F32x4PMax => Uses::default(),
        Operator::F64x2Ceil => Uses::default(),
        Operator::F64x2Floor => Uses::default(),
        Operator::F64x2Trunc => Uses::default(),
        Operator::F64x2Nearest => Uses::default(),
        Operator::F64x2Abs => Uses::default(),
        Operator::F64x2Neg => Uses::default(),
        Operator::F64x2Sqrt => Uses::default(),
        Operator::F64x2Add => Uses::default(),
        Operator::F64x2Sub => Uses::default(),
        Operator::F64x2Mul => Uses::default(),
        Operator::F64x2Div => Uses::default(),
        Operator::F64x2Min => Uses::default(),
        Operator::F64x2Max => Uses::default(),
        Operator::F64x2PMin => Uses::default(),
        Operator::F64x2PMax => Uses::default(),
        Operator::I32x4TruncSatF32x4S => Uses::default(),
        Operator::I32x4TruncSatF32x4U => Uses::default(),
        Operator::F32x4ConvertI32x4S => Uses::default(),
        Operator::F32x4ConvertI32x4U => Uses::default(),
        Operator::I32x4TruncSatF64x2SZero => Uses::default(),
        Operator::I32x4TruncSatF64x2UZero => Uses::default(),
        Operator::F64x2ConvertLowI32x4S => Uses::default(),
        Operator::F64x2ConvertLowI32x4U => Uses::default(),
        Operator::F32x4DemoteF64x2Zero => Uses::default(),
        Operator::F64x2PromoteLowF32x4 => Uses::default(),
        Operator::I8x16RelaxedSwizzle => Uses::default(),
        Operator::I32x4RelaxedTruncF32x4S => Uses::default(),
        Operator::I32x4RelaxedTruncF32x4U => Uses::default(),
        Operator::I32x4RelaxedTruncF64x2SZero => Uses::default(),
        Operator::I32x4RelaxedTruncF64x2UZero => Uses::default(),
        Operator::F32x4RelaxedMadd => Uses::default(),
        Operator::F32x4RelaxedNmadd => Uses::default(),
        Operator::F64x2RelaxedMadd => Uses::default(),
        Operator::F64x2RelaxedNmadd => Uses::default(),
        Operator::I8x16RelaxedLaneselect => Uses::default(),
        Operator::I16x8RelaxedLaneselect => Uses::default(),
        Operator::I32x4RelaxedLaneselect => Uses::default(),
        Operator::I64x2RelaxedLaneselect => Uses::default(),
        Operator::F32x4RelaxedMin => Uses::default(),
        Operator::F32x4RelaxedMax => Uses::default(),
        Operator::F64x2RelaxedMin => Uses::default(),
        Operator::F64x2RelaxedMax => Uses::default(),
        Operator::I16x8RelaxedQ15mulrS => Uses::default(),
        Operator::I16x8RelaxedDotI8x16I7x16S => Uses::default(),
        Operator::I32x4RelaxedDotI8x16I7x16AddS => Uses::default(),
        Operator::TryTable { try_table } => {
            let mut res = Uses::default();
            res.merge(get_blocktype_uses(&try_table.ty));
            for catch in &try_table.catches {
                res.merge(get_catch_uses(catch));
            }
            res
        }
        Operator::Throw { tag_index } => Uses::single_tag(*tag_index),
        Operator::ThrowRef => Uses::default(),
        Operator::Try { blockty } => get_blocktype_uses(blockty),
        Operator::Catch { tag_index } => Uses::single_tag(*tag_index),
        Operator::Rethrow { relative_depth: _ } => Uses::default(),
        Operator::Delegate { relative_depth: _ } => Uses::default(),
        Operator::CatchAll => Uses::default(),
        Operator::GlobalAtomicGet {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicSet {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwAdd {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwSub {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwAnd {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwOr {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwXor {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwXchg {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::GlobalAtomicRmwCmpxchg {
            ordering: _,
            global_index,
        } => Uses::single_global(*global_index),
        Operator::TableAtomicGet {
            ordering: _,
            table_index,
        } => Uses::single_table(*table_index),
        Operator::TableAtomicSet {
            ordering: _,
            table_index,
        } => Uses::single_table(*table_index),
        Operator::TableAtomicRmwXchg {
            ordering: _,
            table_index,
        } => Uses::single_table(*table_index),
        Operator::TableAtomicRmwCmpxchg {
            ordering: _,
            table_index,
        } => Uses::single_table(*table_index),
        Operator::StructAtomicGet {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicGetS {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicGetU {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicSet {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwAdd {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwSub {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwAnd {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwOr {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwXor {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwXchg {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::StructAtomicRmwCmpxchg {
            ordering: _,
            struct_type_index,
            field_index: _,
        } => Uses::single_type(*struct_type_index),
        Operator::ArrayAtomicGet {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicGetS {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicGetU {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicSet {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwAdd {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwSub {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwAnd {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwOr {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwXor {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwXchg {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::ArrayAtomicRmwCmpxchg {
            ordering: _,
            array_type_index,
        } => Uses::single_type(*array_type_index),
        Operator::RefI31Shared => Uses::default(),
        Operator::CallRef { type_index } => Uses::single_type(*type_index),
        Operator::ReturnCallRef { type_index } => Uses::single_type(*type_index),
        Operator::RefAsNonNull => Uses::default(),
        Operator::BrOnNull { relative_depth: _ } => Uses::default(),
        Operator::BrOnNonNull { relative_depth: _ } => Uses::default(),

        Operator::ContNew { .. } => todo!(),
        Operator::ContBind { .. } => todo!(),
        Operator::Suspend { .. } => todo!(),
        Operator::Resume { .. } => todo!(),
        Operator::ResumeThrow { .. } => todo!(),
        Operator::Switch { .. } => todo!(),

        Operator::I64Add128 => Uses::default(),
        Operator::I64Sub128 => Uses::default(),
        Operator::I64MulWideS => Uses::default(),
        Operator::I64MulWideU => Uses::default(),

        _ => Uses::default(),
    }
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

#[derive(Eq, PartialEq, Hash)]
enum Relocation {
    Type(u32),
    Func(u32),
    Table(u32),
    Global(u32),
    Memory(u32),
    Data(u32),
    Elem(u32),
    Tag(u32),
}
