mod relocation;
mod uses;

use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
};

use anyhow::Result;
use clap::Parser as _;
use wasm_encoder::{
    reencode::Reencode, CodeSection, ConstExpr, DataSection, DataSegment, DataSegmentMode,
    ElementMode, ElementSection, ElementSegment, EntityType, ExportSection, Function,
    FunctionSection, GlobalSection, ImportSection, MemorySection, Module, RawSection, TableSection,
    TagSection, TypeSection,
};
use wasmparser::{
    Data, DataKind, Element, ElementKind, Export, Global, GlobalType, Import, MemoryType, Operator,
    Parser, Payload::*, RecGroup, SubType, Table, TableInit, TableType, TagType, ValType,
};

use relocation::*;
use uses::*;

#[derive(clap::Parser, Debug)]
#[command(
    version,
    about = "wasm-isolate strips a WebAssembly module down to specific features of interest without breaking validation."
)]
struct Args {
    /// The file to read from, or "-" to read from stdin
    filename: String,

    /// Type indices to preserve, separated by commas
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    types: Vec<u32>,

    /// Function indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    funcs: Vec<u32>,

    /// Table indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    tables: Vec<u32>,

    /// Global indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    globals: Vec<u32>,

    /// Memory indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    memories: Vec<u32>,

    /// Data segment indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    datas: Vec<u32>,

    /// Elem segment indices to preserve, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    elems: Vec<u32>,

    /// Tag indices to preserve, separated by commas
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    tags: Vec<u32>,

    #[arg(short, long)]
    out: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let filename = args.filename;
    let mut reader = get_reader(filename);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let parser = Parser::new(0);

    let mut types: Vec<SubType> = vec![];
    let mut rec_groups: Vec<RecGroup> = vec![];
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
                sections.push(Section::Type);

                for rg in r {
                    let rg = rg?;
                    rec_groups.push(rg.clone());
                    for t in rg.into_types() {
                        types.push(t);
                    }
                }
            }
            ImportSection(r) => {
                sections.push(Section::Import);

                for import in r {
                    let import = import?;
                    match import.ty {
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
                sections.push(Section::Tag);
                for tag_type in r {
                    tag_types.push(tag_type?);
                }
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
            DataCountSection { count: _, range: _ } => {
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
    // TODO: Ensure that we have an export section for later.
    //

    //
    // Iterate over all live objects until we have gathered all the references.
    //

    let mut work_queue: Vec<WorkItem> = vec![];
    for idx in &args.types {
        if *idx < types.len() as u32 {
            work_queue.push(WorkItem::Type(*idx));
        }
    }
    for idx in &args.funcs {
        if *idx < func_types.len() as u32 {
            work_queue.push(WorkItem::Func(*idx));
        }
    }
    for idx in &args.tables {
        if *idx < table_types.len() as u32 {
            work_queue.push(WorkItem::Table(*idx));
        }
    }
    for idx in &args.globals {
        if *idx < global_types.len() as u32 {
            work_queue.push(WorkItem::Global(*idx));
        }
    }
    for idx in &args.memories {
        if *idx < memory_types.len() as u32 {
            work_queue.push(WorkItem::Memory(*idx));
        }
    }
    for idx in &args.datas {
        if *idx < datas.len() as u32 {
            work_queue.push(WorkItem::Data(*idx));
        }
    }
    for idx in &args.elems {
        if *idx < elems.len() as u32 {
            work_queue.push(WorkItem::Elem(*idx));
        }
    }
    for idx in &args.tags {
        if *idx < tag_types.len() as u32 {
            work_queue.push(WorkItem::Tag(*idx));
        }
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
                res.merge(Uses::single_type(func_types[*idx as usize]));
                if *idx >= num_imported_functions {
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
                if *idx >= num_imported_tables {
                    let table = &defined_tables[(idx - num_imported_tables) as usize];
                    if let TableInit::Expr(expr) = &table.init {
                        res.merge(get_constexpr_uses(expr)?);
                    }
                }
                res
            }
            WorkItem::Global(idx) => {
                let mut res = Uses::single_global(*idx);
                res.merge(get_globaltype_uses(&global_types[*idx as usize]));
                if *idx >= num_imported_globals {
                    let global = &defined_globals[(idx - num_imported_globals) as usize];
                    res.merge(get_constexpr_uses(&global.init_expr)?)
                }
                res
            }
            WorkItem::Memory(idx) => Uses::single_memory(*idx),
            WorkItem::Data(idx) => {
                let mut res = Uses::single_data(*idx);
                let data = &datas[*idx as usize];
                match &data.kind {
                    DataKind::Passive => (),
                    DataKind::Active {
                        memory_index,
                        offset_expr,
                    } => {
                        res.merge(Uses::single_memory(*memory_index));
                        res.merge(get_constexpr_uses(offset_expr)?);
                    }
                };
                res
            }
            WorkItem::Elem(idx) => {
                let mut res = Uses::single_elem(*idx);
                let elem = &elems[*idx as usize];
                match &elem.kind {
                    ElementKind::Passive | ElementKind::Declared => (),
                    ElementKind::Active {
                        table_index,
                        offset_expr,
                    } => {
                        // It's not clear to me why the table index is optional at this stage, but
                        // other code in wasm-tools defaults to zero if it's missing.
                        res.merge(Uses::single_table(table_index.unwrap_or(0)));
                        res.merge(get_constexpr_uses(offset_expr)?);
                    }
                };
                match &elem.items {
                    wasmparser::ElementItems::Functions(funcs) => {
                        for func_idx in funcs.clone() {
                            res.merge(Uses::single_func(func_idx?));
                        }
                    }
                    wasmparser::ElementItems::Expressions(ref_type, exprs) => {
                        res.merge(get_reftype_uses(ref_type));
                        for expr in exprs.clone() {
                            res.merge(get_constexpr_uses(&expr?)?);
                        }
                    }
                };
                res
            }
            WorkItem::Tag(idx) => {
                let mut res = Uses::single_tag(*idx);
                res.merge(get_tagtype_uses(&tag_types[*idx as usize]));
                res
            }
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

    //
    // Track relocations
    //

    let mut relocations = HashMap::<Relocation, u32>::new();

    for type_idx in &all_uses.live_types {
        // Type canonicalization be damned. Surely no self-respecting compiler would leave
        // redundant types in its output.
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
    let mut reencoder = RelocatingReencoder {
        relocations: &relocations,
    };
    for section in sections {
        match section {
            Section::Passthrough(sec) => {
                out.section(&sec);
            }

            Section::Type => {
                let mut type_section = TypeSection::new();
                let mut idx: u32 = 0;
                for rg in &rec_groups {
                    let mut sub_types: Vec<wasm_encoder::SubType> = vec![];
                    for ty in rg.types() {
                        if relocations.get(&Relocation::Type(idx)).is_some() {
                            sub_types.push(reencoder.sub_type(ty.clone())?);
                        }
                        idx += 1;
                    }
                    if sub_types.len() == 1 {
                        type_section.ty().subtype(sub_types.first().unwrap());
                    } else if sub_types.len() > 1 || rg.is_explicit_rec_group() {
                        type_section.ty().rec(sub_types)
                    }
                }
                out.section(&type_section);
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
                                    EntityType::Function(reencoder.type_index(type_idx)),
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
                        function_section.function(reencoder.type_index(func_types[idx as usize]));
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
            Section::Memory => {
                let mut memory_section = MemorySection::new();
                for idx in num_imported_memories..(memory_types.len() as u32) {
                    if relocations.get(&Relocation::Memory(idx)).is_some() {
                        let mem_type = &memory_types[idx as usize];
                        memory_section.memory(reencoder.memory_type(mem_type.clone()));
                    }
                }
                out.section(&memory_section);
            }
            Section::Global => {
                let mut global_section = GlobalSection::new();
                for (i, global) in defined_globals.iter().enumerate() {
                    let idx = num_imported_globals + i as u32;
                    if relocations.get(&Relocation::Global(idx)).is_some() {
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
                    // We don't use the reencoder here because we need to actually look up from the
                    // relocation map anyway to figure out if we should export at all. So then we
                    // might as well just write the value we find there.
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

                // Also export the explicitly-requested things so it's easy to test them in isolation.
                for idx in &args.funcs {
                    if let Some(new_idx) = relocations.get(&Relocation::Func(*idx)) {
                        export_section.export(
                            &format!("isolated_func_{}", *idx),
                            wasm_encoder::ExportKind::Func,
                            *new_idx,
                        );
                    }
                }
                for idx in &args.tables {
                    if let Some(new_idx) = relocations.get(&Relocation::Table(*idx)) {
                        export_section.export(
                            &format!("isolated_table_{}", *idx),
                            wasm_encoder::ExportKind::Table,
                            *new_idx,
                        );
                    }
                }
                for idx in &args.globals {
                    if let Some(new_idx) = relocations.get(&Relocation::Global(*idx)) {
                        export_section.export(
                            &format!("isolated_global_{}", *idx),
                            wasm_encoder::ExportKind::Global,
                            *new_idx,
                        );
                    }
                }
                for idx in &args.memories {
                    if let Some(new_idx) = relocations.get(&Relocation::Memory(*idx)) {
                        export_section.export(
                            &format!("isolated_memory_{}", *idx),
                            wasm_encoder::ExportKind::Memory,
                            *new_idx,
                        );
                    }
                }
                for idx in &args.tags {
                    if let Some(new_idx) = relocations.get(&Relocation::Tag(*idx)) {
                        export_section.export(
                            &format!("isolated_tag_{}", *idx),
                            wasm_encoder::ExportKind::Tag,
                            *new_idx,
                        );
                    }
                }

                out.section(&export_section);
            }
            Section::Start => {
                if let Some(idx) = start_idx {
                    if let Some(new_idx) = relocations.get(&Relocation::Func(idx)) {
                        out.section(&wasm_encoder::StartSection {
                            function_index: *new_idx,
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
                                wasmparser::ElementKind::Declared => ElementMode::Declared,
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
                            data: data.data.iter().map(|b| *b).collect::<Vec<u8>>(),
                        });
                    }
                }
                out.section(&data_section);
            }
            Section::DataCount => {
                out.section(&wasm_encoder::DataCountSection {
                    count: all_uses.live_datas.len() as u32,
                });
            }
            Section::Tag => {
                let mut tag_section = TagSection::new();
                for idx in num_imported_tags..(tag_types.len() as u32) {
                    if relocations.get(&Relocation::Tag(idx)).is_some() {
                        let tag_type = &tag_types[idx as usize];
                        tag_section.tag(reencoder.tag_type(tag_type.clone()));
                    }
                }
                out.section(&tag_section);
            }
        }
    }
    let out_bytes = out.finish();

    if let Some(path) = &args.out {
        fs::write(path, out_bytes).expect("unable to write file");
    } else {
        std::io::stdout()
            .write_all(&out_bytes)
            .expect("unable to write output");
    }

    // Tell the user where the new things are
    eprintln!("Success! The requested items are now located at these indices:");
    for idx in &args.types {
        if let Some(new_idx) = relocations.get(&Relocation::Type(*idx)) {
            eprintln!("  Type {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Type {} was out of range and therefore ignored.", *idx);
        }
    }
    for idx in &args.funcs {
        if let Some(new_idx) = relocations.get(&Relocation::Func(*idx)) {
            eprintln!("  Func {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Func {} was out of range and therefore ignored.", *idx);
        }
    }
    for idx in &args.tables {
        if let Some(new_idx) = relocations.get(&Relocation::Table(*idx)) {
            eprintln!("  Table {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Table {} was out of range and therefore ignored.", *idx);
        }
    }
    for idx in &args.globals {
        if let Some(new_idx) = relocations.get(&Relocation::Global(*idx)) {
            eprintln!("  Global {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Global {} was out of range and therefore ignored.", *idx);
        }
    }
    for idx in &args.memories {
        if let Some(new_idx) = relocations.get(&Relocation::Memory(*idx)) {
            eprintln!("  Memory {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Memory {} was out of range and therefore ignored.", *idx);
        }
    }
    for idx in &args.datas {
        if let Some(new_idx) = relocations.get(&Relocation::Data(*idx)) {
            eprintln!("  Data segment {} -> {}", *idx, new_idx);
        } else {
            eprintln!(
                "  Data segment {} was out of range and therefore ignored.",
                *idx
            );
        }
    }
    for idx in &args.elems {
        if let Some(new_idx) = relocations.get(&Relocation::Elem(*idx)) {
            eprintln!("  Elem segment {} -> {}", *idx, new_idx);
        } else {
            eprintln!(
                "  Elem segment {} was out of range and therefore ignored.",
                *idx
            );
        }
    }
    for idx in &args.tags {
        if let Some(new_idx) = relocations.get(&Relocation::Tag(*idx)) {
            eprintln!("  Tag {} -> {}", *idx, new_idx);
        } else {
            eprintln!("  Tag {} was out of range and therefore ignored.", *idx);
        }
    }

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
    fn raw(id: u8, bytes: &'a [u8]) -> Section<'a> {
        let foo = RawSection {
            id: id,
            data: bytes,
        };
        Self::Passthrough(foo)
    }
}
