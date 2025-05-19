use std::{fs, path::Path};

use anyhow::Result;
use wasm_encoder::{
    reencode::{Reencode, RoundtripReencoder},
    CodeSection, ConstExpr, EntityType, ExportSection, Function, FunctionSection, GlobalSection,
    ImportSection, Instruction, MemArg, MemorySection, Module, TypeSection,
};
use wasmparser::{
    Export, Global, GlobalType, Import, MemoryType, Operator, Parser, Payload::*, RecGroup,
    SubType, TableType, TagType, ValType,
};

use crate::util::*;

#[derive(clap::Parser, Debug)]
pub struct ReplayArgs {
    /// The file to read from, or "-" to read from stdin
    filename: String,

    /// Function indices to instrument, separated by commas
    #[arg(short, long, num_args = 1.., value_delimiter = ',')]
    funcs: Vec<u32>,

    #[arg(short, long)]
    out: Option<String>,
}

const INDEX_MJS: &[u8] = include_bytes!("index.mjs");

pub fn replay(args: ReplayArgs) -> Result<()> {
    let filename = args.filename;
    let out = args.out.unwrap_or("replay".to_string());

    let mut reader = get_reader(filename);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let parser = Parser::new(0);

    // The general concept here is to instrument the module in two different
    // ways: one for recording, one for replaying.
    //
    // The recording instrumentation adds special functions to the module that
    // record the full module state, and inserts calls to those functions at
    // the start of the target functions. This will slow down execution but
    // preserve state for later replay.
    //
    // The replay instrumentation adds special functions to the module that
    // initialize the module to a previously-recorded state, and that call the
    // function with previously-recorded arguments.
    //
    // We need to record "call states", which contain all the info necessary to
    // recreate the module state later--that is, initializing the module's
    // memory, tables, and globals with all the values they originally had.
    // This is pretty easy for numeric types, but actually quite difficult for
    // ref types, as we need to be able to synthesize funcrefs, GC objects, and
    // in the extreme case, arbitrary host values.

    let mut num_imported: ImportCounts = ImportCounts::default();

    let mut types: Vec<SubType> = vec![];
    let mut rec_groups: Vec<RecGroup> = vec![];
    let mut func_types: Vec<u32> = vec![];
    let mut table_types: Vec<TableType> = vec![];
    let mut memory_types: Vec<MemoryType> = vec![];
    let mut global_types: Vec<GlobalType> = vec![];
    let mut tag_types: Vec<TagType> = vec![];

    let mut imports: Vec<Import> = vec![];
    let mut exports: Vec<Export> = vec![];
    let mut defined_funcs: Vec<Func> = vec![];
    let mut defined_globals: Vec<Global> = vec![];

    let mut current_func = 0;
    let mut first_func: bool = true;

    let mut sections: Vec<Section> = vec![];

    for payload in parser.parse_all(&buf) {
        match payload? {
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
                            num_imported.functions += 1;
                            func_types.push(type_idx);
                        }
                        wasmparser::TypeRef::Table(ty) => {
                            num_imported.tables += 1;
                            table_types.push(ty);
                        }
                        wasmparser::TypeRef::Memory(ty) => {
                            num_imported.memories += 1;
                            memory_types.push(ty);
                        }
                        wasmparser::TypeRef::Global(ty) => {
                            num_imported.globals += 1;
                            global_types.push(ty);
                        }
                        wasmparser::TypeRef::Tag(ty) => {
                            num_imported.tags += 1;
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
                sections.push(Section::raw(4, &buf[r.range()]));
                for table in r {
                    let table = table?;
                    table_types.push(table.ty);
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
            StartSection { func: _, range } => {
                sections.push(Section::raw(8, &buf[range]));
            }
            ElementSection(r) => {
                sections.push(Section::raw(9, &buf[r.range()]));
            }
            DataCountSection { count: _, range } => {
                sections.push(Section::raw(12, &buf[range]));
            }
            DataSection(r) => {
                sections.push(Section::raw(11, &buf[r.range()]));
            }

            CodeSectionStart { .. } => {
                sections.push(Section::Code);
                current_func = num_imported.functions;
            }
            CodeSectionEntry(body) => {
                if first_func {
                    first_func = false
                } else {
                    current_func += 1;
                }

                let mut func = Func {
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
                sections.push(Section::raw(0, &buf[r.range()]));
            }

            _ => {}
        }
    }
    ensure_section(&mut sections, Section::Memory);
    ensure_section(&mut sections, Section::Global);

    //
    // Record/replay instrumentation
    //

    // For the most part, we just proxy the contents of the module through
    // unchanged. However, we make some modifications and additions for the
    // record and replay modes. Both are done inline to avoid duplication.
    //
    // Record mode adds:
    // - Recorder functions for each of the requested functions. Each recorder
    //   writes an entire call state into the buffer.
    // - A memory to serve as the output buffer (exported as
    //   "_replay:call_log") and a global to serve as a cursor into the output
    //   (exported as "_replay:call_log_cursor").
    //
    // Replay mode adds:
    // - Direct-call stub functions for each of the requested functions.
    // - A module init function that resets the entire module to a particular
    //   call state.
    // - A call log walker that reads the binary format without executing
    //   anything.

    fs::create_dir_all(&out).expect("unable to create output directory");
    for mode in [InstrumentationMode::Record, InstrumentationMode::Replay] {
        let mut module = Module::new();
        let mut reencoder = RoundtripReencoder {};

        let buf_memtype = wasm_encoder::MemoryType {
            minimum: 65536,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        };
        let buf_memidx = memory_types.len() as u32;
        let cur_globalidx = global_types.len() as u32;

        let recorder_base_typeidx = types.len() as u32;
        let recorder_base_funcidx = func_types.len() as u32;

        let replay_modinit_typeidx = types.len() as u32;
        let replay_modinit_funcidx = func_types.len() as u32;
        let replay_walker_typeidx = replay_modinit_typeidx + 1;
        let replay_walker_funcidx = replay_modinit_funcidx + 1;
        let replay_callstub_base_typeidx = replay_walker_typeidx + 1;
        let replay_callstub_base_funcidx = replay_walker_funcidx + 1;
        let replay_errcode_globalidx = cur_globalidx + 1;

        for section in &sections {
            match section {
                Section::Passthrough(sec) => {
                    module.section(sec);
                }

                Section::Type => {
                    let mut type_section = TypeSection::new();

                    // Pass through all existing rec groups
                    for rg in &rec_groups {
                        let mut sub_types: Vec<wasm_encoder::SubType> = vec![];
                        for ty in rg.types() {
                            sub_types.push(reencoder.sub_type(ty.clone())?);
                        }
                        if sub_types.len() == 1 {
                            type_section.ty().subtype(sub_types.first().unwrap());
                        } else if sub_types.len() > 1 || rg.is_explicit_rec_group() {
                            type_section.ty().rec(sub_types)
                        }
                    }

                    match mode {
                        InstrumentationMode::Record => {
                            // Add types for recorders. We don't worry about duplicate
                            // types; type equality/canonicalization has our back.
                            for idx_to_record in &args.funcs {
                                let func_type_idx = func_types[*idx_to_record as usize];
                                let func_type = types[func_type_idx as usize].unwrap_func();
                                type_section.ty().func_type(&wasm_encoder::FuncType::new(
                                    func_type
                                        .params()
                                        .iter()
                                        .map(|p| reencoder.val_type(*p).unwrap()),
                                    vec![],
                                ));
                            }
                        }
                        InstrumentationMode::Replay => {
                            use wasm_encoder::ValType::*;

                            // Add type for module-state initializer.
                            // Signature: [i32] -> [i32]
                            // Pass the index of a call state; it will return the cursor position
                            // for the call state's params so you can call the call stub.
                            type_section
                                .ty()
                                .func_type(&wasm_encoder::FuncType::new(vec![I32], vec![I32]));

                            // Add type for call log walker.
                            // Signature: [i32] -> [i32 i32]
                            // Pass a cursor position; it will return a func index and the next
                            // cursor position. Note that there is no indication here of when you
                            // are done with the log.
                            type_section
                                .ty()
                                .func_type(&wasm_encoder::FuncType::new(vec![I32], vec![I32, I32]));

                            // Add types for call stubs.
                            // Signature: [i32] -> <original results>
                            // Pass the index of the params within a call state. (The intent is to
                            // get this from the module initializer function.)
                            for idx_to_call in &args.funcs {
                                let func_type_idx = func_types[*idx_to_call as usize];
                                let func_type = types[func_type_idx as usize].unwrap_func();
                                type_section.ty().func_type(&wasm_encoder::FuncType::new(
                                    vec![I32],
                                    func_type
                                        .results()
                                        .iter()
                                        .map(|r| reencoder.val_type(*r).unwrap()),
                                ));
                            }
                        }
                    }

                    module.section(&type_section);
                }
                Section::Import => {
                    let mut import_section = ImportSection::new();

                    // Pass through existing imports
                    for import in &imports {
                        match import.ty {
                            wasmparser::TypeRef::Func(type_idx) => {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    EntityType::Function(reencoder.type_index(type_idx)),
                                );
                            }
                            wasmparser::TypeRef::Table(ty) => {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.table_type(ty)?,
                                );
                            }
                            wasmparser::TypeRef::Memory(ty) => {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.memory_type(ty),
                                );
                            }
                            wasmparser::TypeRef::Global(ty) => {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.global_type(ty)?,
                                );
                            }
                            wasmparser::TypeRef::Tag(ty) => {
                                import_section.import(
                                    import.module,
                                    import.name,
                                    reencoder.tag_type(ty),
                                );
                            }
                        }
                    }

                    // We do NOT add any other imports here because this will
                    // mess with the indexes of everything else in the module.
                    // Instead we must get everything else we need via refs.

                    module.section(&import_section);
                }
                Section::Function => {
                    let mut function_section = FunctionSection::new();

                    // Pass through all existing functions
                    for (i, _) in defined_funcs.iter().enumerate() {
                        let idx = num_imported.functions + i as u32;
                        function_section.function(func_types[idx as usize]);
                    }

                    match mode {
                        InstrumentationMode::Record => {
                            // Add types for recorders
                            for i in 0..args.funcs.len() {
                                function_section.function(recorder_base_typeidx + i as u32);
                            }
                        }
                        InstrumentationMode::Replay => {
                            // Add type for the module-state initializer
                            function_section.function(replay_modinit_typeidx as u32);

                            // Add type for the call log walker
                            function_section.function(replay_walker_typeidx as u32);

                            // Add types for call stubs
                            for i in 0..args.funcs.len() {
                                function_section.function(replay_callstub_base_typeidx + i as u32);
                            }
                        }
                    }

                    module.section(&function_section);
                }
                Section::Memory => {
                    let mut memory_section = MemorySection::new();

                    // Pass through all existing memories
                    for idx in num_imported.memories..(memory_types.len() as u32) {
                        let mem_type = &memory_types[idx as usize];
                        memory_section.memory(reencoder.memory_type(mem_type.clone()));
                    }

                    // Create memory for buffer (used in both modes)
                    memory_section.memory(buf_memtype);

                    module.section(&memory_section);
                }
                Section::Global => {
                    let mut global_section = GlobalSection::new();

                    // Pass through all existing globals
                    for global in &defined_globals {
                        global_section.global(
                            reencoder.global_type(global.ty)?,
                            &reencoder.const_expr(global.init_expr.clone())?,
                        );
                    }

                    // Define the global for the cursor (used in both modes)
                    global_section.global(
                        wasm_encoder::GlobalType {
                            val_type: wasm_encoder::ValType::I32,
                            mutable: true,
                            shared: false,
                        },
                        &ConstExpr::i32_const(0),
                    );

                    match mode {
                        InstrumentationMode::Record => {}
                        InstrumentationMode::Replay => {
                            // Define the global for the error code
                            global_section.global(
                                wasm_encoder::GlobalType {
                                    val_type: wasm_encoder::ValType::I32,
                                    mutable: true,
                                    shared: false,
                                },
                                &ConstExpr::i32_const(0),
                            );
                        }
                    }

                    module.section(&global_section);
                }
                Section::Export => {
                    let mut export_section = ExportSection::new();

                    // Pass through all existing exports
                    for export in &exports {
                        export_section.export(export.name, export.kind.into(), export.index);
                    }

                    // Export the buffer memory and cursor
                    export_section.export(
                        "_replay:call_log",
                        wasm_encoder::ExportKind::Memory,
                        buf_memidx,
                    );
                    export_section.export(
                        "_replay:call_log_cursor",
                        wasm_encoder::ExportKind::Global,
                        cur_globalidx,
                    );

                    match mode {
                        InstrumentationMode::Record => {}
                        InstrumentationMode::Replay => {
                            export_section.export(
                                "_replay:err_code",
                                wasm_encoder::ExportKind::Global,
                                replay_errcode_globalidx,
                            );
                            export_section.export(
                                "_replay:module_init",
                                wasm_encoder::ExportKind::Func,
                                replay_modinit_funcidx,
                            );
                            export_section.export(
                                "_replay:walk_call_log",
                                wasm_encoder::ExportKind::Func,
                                replay_walker_funcidx,
                            );
                            for (i, funcidx) in args.funcs.iter().enumerate() {
                                export_section.export(
                                    format!("_replay:callstub_{funcidx}").as_str(),
                                    wasm_encoder::ExportKind::Func,
                                    replay_callstub_base_funcidx + i as u32,
                                );
                            }
                        }
                    }

                    module.section(&export_section);
                }
                Section::Code => {
                    let mut code_section = CodeSection::new();

                    // Pass through all existing functions - BUT insert a call to
                    // the corresponding recorder if requested.
                    for (i, func) in defined_funcs.iter().enumerate() {
                        let func_idx = i as u32 + num_imported.functions;
                        let func_type = types[func_types[func_idx as usize] as usize].unwrap_func();

                        let mut new_locals: Vec<(u32, wasm_encoder::ValType)> = vec![];
                        for (n, ty) in &func.locals {
                            new_locals.push((*n, reencoder.val_type(*ty)?));
                        }
                        let mut new_func = Function::new(new_locals);

                        // Call the recorder, if requested
                        if matches!(mode, InstrumentationMode::Record) {
                            if let Some(recorder_idx) =
                                args.funcs.iter().position(|idx| *idx == func_idx)
                            {
                                for i in 0..func_type.params().len() {
                                    new_func.instruction(&Instruction::LocalGet(i as u32));
                                }
                                new_func.instruction(&Instruction::Call(
                                    recorder_base_funcidx + recorder_idx as u32,
                                ));
                            }
                        }

                        // Pass through the function body
                        for instr in &func.instructions {
                            new_func.instruction(&reencoder.instruction(instr.clone())?);
                        }

                        code_section.function(&new_func);
                    }

                    // TODO:
                    // - We are definitely not handling overflow correctly for memory64.

                    match mode {
                        InstrumentationMode::Record => {
                            // Generate recorders
                            for idx_to_record in &args.funcs {
                                use Instruction::*;

                                let func_type_idx = func_types[*idx_to_record as usize];
                                let func_type = types[func_type_idx as usize].unwrap_func();

                                let mut w = RecorderWriter {
                                    f: &mut Function::new(vec![(4, wasm_encoder::ValType::I32)]),
                                    buf: buf_memidx,
                                    cur: cur_globalidx,
                                };
                                let i32_tmps = func_type.params().len() as u32;

                                // Write func idx to begin call state
                                w.i32_imm(*idx_to_record as i32);

                                // Write memories
                                w.i32(&I32Const(memory_types.len() as i32));
                                for (i, mem_type) in memory_types.iter().enumerate() {
                                    // We now inline an RLE compressor because it has to be specialized
                                    // to a specific memory.
                                    match mem_type.index_type() {
                                        ValType::I32 => {
                                            // Write memory size in pages
                                            w.i64(&[&MemorySize(i as u32), &I64ExtendI32U]);
                                            w.rle(
                                                i as u32,
                                                &[
                                                    i32_tmps + 0,
                                                    i32_tmps + 1,
                                                    i32_tmps + 2,
                                                    i32_tmps + 3,
                                                ],
                                            );
                                        }
                                        ValType::I64 => {
                                            todo!();
                                        }
                                        _ => panic!("invalid address type"),
                                    }
                                }

                                // Write tables
                                // TODO

                                // Write mutable globals
                                w.i32(&I32Const(global_types.len() as i32));
                                for (i, global_type) in global_types.iter().enumerate() {
                                    if global_type.mutable {
                                        w.val(&global_type.content_type, &GlobalGet(i as u32))
                                    }
                                }

                                // Write params
                                w.i32(&I32Const(func_type.params().len() as i32));
                                for (i, param_type) in func_type.params().iter().enumerate() {
                                    w.val(param_type, &LocalGet(i as u32));
                                }

                                w.end();
                                code_section.function(w.f);
                            }
                        }
                        InstrumentationMode::Replay => {
                            // Generate module-state initializer
                            {
                                use Instruction::*;

                                // The function signature is [i32] -> [i32]
                                let mut r = ReplayReader {
                                    f: &mut Function::new(vec![(3, wasm_encoder::ValType::I32)]),
                                    buf: buf_memidx,
                                    cur: cur_globalidx,
                                    errcode: replay_errcode_globalidx,
                                };
                                let tmps_i32 = 1;

                                // Reset cursor to start of call state
                                r.add(&LocalGet(0));
                                r.add(&GlobalSet(cur_globalidx));

                                // Skip func idx
                                r.i32();
                                r.add(&Drop);

                                // Initialize memories
                                r.add(&Nop);
                                r.expect_i32(memory_types.len() as i32, ErrCode::WrongNumMemories);
                                for (i, mem_type) in memory_types.iter().enumerate() {
                                    match mem_type.index_type() {
                                        ValType::I32 => {
                                            // Read size in pages
                                            r.i64();
                                            r.add(&I32WrapI64);
                                            r.add(&LocalSet(tmps_i32 + 0));

                                            // Grow memory if necessary
                                            r.add(&MemorySize(i as u32));
                                            r.add(&LocalGet(tmps_i32 + 0));
                                            r.add(&I32LtU);
                                            r.add(&If(wasm_encoder::BlockType::Empty));
                                            {
                                                r.add(&MemorySize(i as u32));
                                                r.add(&LocalGet(tmps_i32 + 0));
                                                r.add(&I32Sub);
                                                r.add(&MemoryGrow(i as u32));
                                                r.add(&I32Const(0));
                                                r.add(&I32GeS);
                                                r.add(&BrIf(0));
                                                r.abort(ErrCode::FailedToGrowMemory);
                                            }
                                            r.add(&End);

                                            // RLE decompress the memory
                                            r.rle(
                                                i as u32,
                                                tmps_i32 + 0,
                                                tmps_i32 + 1,
                                                tmps_i32 + 2,
                                            );
                                        }
                                        ValType::I64 => {
                                            todo!();
                                        }
                                        _ => panic!("invalid address type"),
                                    }
                                }

                                // TODO: Read tables
                                r.add(&Nop);

                                // Initialize mutable globals
                                r.add(&Nop);
                                r.expect_i32(
                                    global_types.iter().filter(|t| t.mutable).count() as i32,
                                    ErrCode::WrongNumMutableGlobals,
                                );
                                for (i, global_type) in global_types.iter().enumerate() {
                                    if global_type.mutable {
                                        r.val(&global_type.content_type);
                                        r.add(&GlobalSet(i as u32));
                                    }
                                }

                                // Don't read params; the params will be loaded from memory at the time of the call.
                                // We keep the cursor pointing at the params.
                                r.add(&GlobalGet(cur_globalidx));

                                r.end();
                                code_section.function(r.f);
                            }

                            // Generate call log walker
                            {
                                use Instruction::*;

                                // The function signature is [i32] -> [i32 i32]
                                let mut r = ReplayReader {
                                    f: &mut Function::new(vec![(3, wasm_encoder::ValType::I32)]),
                                    buf: buf_memidx,
                                    cur: cur_globalidx,
                                    errcode: replay_errcode_globalidx,
                                };
                                let funcidx = 1;
                                let num_params = 2;
                                let tmp_i32 = 3;

                                // Reset cursor
                                r.add(&LocalGet(0));
                                r.add(&GlobalSet(cur_globalidx));

                                // Read func idx
                                r.i32();
                                r.add(&LocalSet(funcidx));

                                // Skip memories
                                r.add(&Nop);
                                r.expect_i32(memory_types.len() as i32, ErrCode::WrongNumMemories);
                                for mem_type in &memory_types {
                                    match mem_type.index_type() {
                                        ValType::I32 => {
                                            r.i64(); // length in pages
                                            r.add(&Drop);
                                            r.i32(); // length of compressed data
                                            r.bump_cur();
                                        }
                                        ValType::I64 => {
                                            todo!();
                                        }
                                        _ => panic!("invalid address type"),
                                    }
                                }

                                // TODO: Skip tables
                                r.add(&Nop);

                                // Skip mutable globals
                                r.add(&Nop);
                                r.expect_i32(
                                    global_types.iter().filter(|t| t.mutable).count() as i32,
                                    ErrCode::WrongNumMutableGlobals,
                                );
                                for global_type in &global_types {
                                    if global_type.mutable {
                                        r.val(&global_type.content_type);
                                        r.add(&Drop);
                                    }
                                }

                                // Skip params
                                r.add(&Nop);
                                r.i32();
                                r.add(&LocalSet(num_params));
                                r.add(&Block(wasm_encoder::BlockType::Empty));
                                {
                                    r.add(&Loop(wasm_encoder::BlockType::Empty));
                                    {
                                        // if (n == 0) break;
                                        r.add(&LocalGet(num_params));
                                        r.add(&I32Eqz);
                                        r.add(&BrIf(1));

                                        r.skip_val(tmp_i32);

                                        // n -= 1; continue;
                                        r.add(&LocalGet(num_params));
                                        r.add(&I32Const(1));
                                        r.add(&I32Sub);
                                        r.add(&LocalSet(num_params));
                                        r.add(&Br(0));
                                    }
                                    r.add(&End);
                                };
                                r.add(&End);

                                // Return values
                                r.add(&LocalGet(funcidx));
                                r.add(&GlobalGet(cur_globalidx));

                                r.end();
                                code_section.function(r.f);
                            }

                            // Generate call stubs
                            for idx_to_call in &args.funcs {
                                use Instruction::*;

                                let func_type_idx = func_types[*idx_to_call as usize];
                                let func_type = types[func_type_idx as usize].unwrap_func();

                                // The function signature is [i32] -> <original results>
                                let mut r = ReplayReader {
                                    f: &mut Function::new(vec![]),
                                    buf: buf_memidx,
                                    cur: cur_globalidx,
                                    errcode: replay_errcode_globalidx,
                                };

                                r.expect_i32(
                                    func_type.params().len() as i32,
                                    ErrCode::WrongNumParams,
                                );
                                for param_type in func_type.params() {
                                    r.val(param_type);
                                }
                                r.add(&Call(*idx_to_call));

                                r.end();
                                code_section.function(r.f);
                            }
                        }
                    }

                    module.section(&code_section);
                }
                _ => todo!("{:?} section", section),
            }
        }

        // Emit a custom section describing the instrumentation mode
        module.section(&wasm_encoder::CustomSection {
            name: "_replay:mode".into(),
            data: match mode {
                InstrumentationMode::Record => vec![0].into(),
                InstrumentationMode::Replay => vec![1].into(),
            },
        });

        let module_bytes = module.finish();

        fs::write(
            match mode {
                InstrumentationMode::Record => Path::new(&out).join("record.wasm"),
                InstrumentationMode::Replay => Path::new(&out).join("replay.wasm"),
            },
            module_bytes,
        )
        .expect("unable to write file");
    }

    // Write the JS instrumentation
    fs::write(Path::new(&out).join("index.mjs"), INDEX_MJS).expect("unable to write file");
    fs::write(
        Path::new(&out).join("index.js"),
        std::str::from_utf8(INDEX_MJS)?.replace("export ", ""),
    )
    .expect("unable to write file");

    Ok(())
}

struct Func<'a> {
    locals: Vec<(u32, ValType)>,
    instructions: Vec<Operator<'a>>,
}

enum InstrumentationMode {
    Record,
    Replay,
}

type Instructions<'a> = [&'a Instruction<'a>];

struct RecorderWriter<'a> {
    f: &'a mut Function,
    buf: u32,
    cur: u32,
}

impl RecorderWriter<'_> {
    fn in_buf(&self) -> MemArg {
        MemArg {
            offset: 0,
            align: 0,
            memory_index: self.buf,
        }
    }

    fn add(&mut self, ins: &Instruction) {
        self.f.instruction(ins);
    }

    fn instructions<'a>(&mut self, instructions: &Instructions) {
        for ins in instructions {
            self.f.instruction(ins);
        }
    }

    fn bump_cur<'a>(&mut self, delta: &Instructions) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.instructions(delta);
        self.f.instruction(&I32Add);
        self.f.instruction(&GlobalSet(self.cur));
    }

    fn bump_cur_imm(&mut self, n: i32) {
        use Instruction::*;
        self.bump_cur(&[&I32Const(n)]);
    }

    fn byte(&mut self, value: &Instructions) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.instructions(value);
        self.f.instruction(&I32Store8(self.in_buf()));
        self.bump_cur_imm(1);
    }

    fn byte_imm(&mut self, byte: i32) {
        use Instruction::*;
        self.byte(&[&I32Const(byte)]);
    }

    fn i16(&mut self, ins: &Instruction) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&I32Store16(self.in_buf()));
        self.bump_cur_imm(2);
    }

    fn i32(&mut self, ins: &Instruction) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&I32Store(self.in_buf()));
        self.bump_cur_imm(4);
    }

    fn i32_imm(&mut self, n: i32) {
        use Instruction::*;
        self.i32(&I32Const(n));
    }

    fn i64(&mut self, ins: &Instructions) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.instructions(ins);
        self.f.instruction(&I64Store(self.in_buf()));
        self.bump_cur_imm(8);
    }

    fn f32(&mut self, ins: &Instruction) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&F32Store(self.in_buf()));
        self.bump_cur_imm(4);
    }

    fn f64(&mut self, ins: &Instruction) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&F64Store(self.in_buf()));
        self.bump_cur_imm(8);
    }

    fn v128(&mut self, ins: &Instruction) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&V128Store(self.in_buf()));
        self.bump_cur_imm(16);
    }

    fn val(&mut self, val_type: &ValType, load: &Instruction) {
        match val_type {
            ValType::I32 => {
                self.byte_imm(0x7F);
                self.i32(load);
            }
            ValType::I64 => {
                self.byte_imm(0x7E);
                self.i64(&[load]);
            }
            ValType::F32 => {
                self.byte_imm(0x7D);
                self.f32(load);
            }
            ValType::F64 => {
                self.byte_imm(0x7C);
                self.f64(load);
            }
            ValType::V128 => {
                self.byte_imm(0x7B);
                self.v128(load);
            }
            ValType::Ref(ref_type) => todo!(),
        }
    }

    /// It is your responsibility to ensure that base has the correct address type for the given memory.
    /// Len should always be i32 because the destination buffer is i32.
    fn bytes<'a>(&mut self, mem_idx: u32, base: &Instructions, len: &Instructions) {
        use Instruction::*;
        self.f.instruction(&GlobalGet(self.cur)); // dst offset
        self.instructions(base); // src offset
        self.instructions(len); // len
        self.f.instruction(&MemoryCopy {
            src_mem: mem_idx,
            dst_mem: self.buf,
        });
        self.bump_cur(len);
    }

    // Encodes the contents of memory in RLE form as a vector of bytes (i32
    // length, then contents). TODO: i64 length?
    fn rle(&mut self, mem_idx: u32, tmp_i32s: &[u32; 4]) {
        use Instruction::*;
        let [i, mode, current_byte, current_count] = *tmp_i32s;

        let from_mem = MemArg {
            offset: 0,
            align: 0,
            memory_index: mem_idx,
        };

        // Write dummy length and prepare it to be overwritten
        self.add(&GlobalGet(self.cur));
        self.add(&GlobalGet(self.cur));
        self.i32(&I32Const(-1));

        // i = 0;
        self.add(&I32Const(0));
        self.add(&LocalSet(i));

        // mode = 0;
        self.add(&I32Const(0));
        self.add(&LocalSet(mode));

        // current_byte = 0;
        self.add(&I32Const(0));
        self.add(&LocalSet(current_byte));

        // current_count = 0;
        self.add(&I32Const(0));
        self.add(&LocalSet(current_count));

        // while (i < mem.length) ...
        self.add(&Block(wasm_encoder::BlockType::Empty));
        {
            self.add(&Loop(wasm_encoder::BlockType::Empty));
            {
                // if (i >= mem.length) break;
                self.add(&LocalGet(i));
                self.add(&MemorySize(mem_idx));
                self.add(&I32Const(65536));
                self.add(&I32Mul);
                self.add(&I32GeU);
                self.add(&BrIf(1)); // break;

                // current_byte = mem[i]
                self.add(&LocalGet(i));
                self.add(&I32Load8U(from_mem));
                self.add(&LocalSet(current_byte));

                // current_count = 1;
                self.add(&I32Const(1));
                self.add(&LocalSet(current_count));

                // i++;
                self.add(&LocalGet(i));
                self.add(&I32Const(1));
                self.add(&I32Add);
                self.add(&LocalSet(i));

                // while (i < mem.length && currentCount < max) ...
                self.add(&Block(wasm_encoder::BlockType::Empty));
                {
                    self.add(&Loop(wasm_encoder::BlockType::Empty));
                    {
                        // if (i >= mem.length || currentCount >= max) break;
                        self.add(&LocalGet(i));
                        self.add(&MemorySize(mem_idx));
                        self.add(&I32Const(65536));
                        self.add(&I32Mul);
                        self.add(&I32GeU);
                        self.add(&LocalGet(current_count));
                        self.add(&I32Const(65535));
                        self.add(&I32GeU);
                        self.add(&I32Or);
                        self.add(&BrIf(1)); // break;

                        // if ((mode === 0) xor (mem[i] === current_byte)) break;
                        // (this condition conveniently encodes exit logic for both modes)
                        self.add(&LocalGet(mode));
                        self.add(&I32Eqz);
                        self.add(&LocalGet(i));
                        self.add(&I32Load8U(from_mem));
                        self.add(&LocalGet(current_byte));
                        self.add(&I32Eq);
                        self.add(&I32Xor);
                        self.add(&BrIf(1)); // break;

                        // current_count++
                        self.add(&LocalGet(current_count));
                        self.add(&I32Const(1));
                        self.add(&I32Add);
                        self.add(&LocalSet(current_count));

                        // current_byte = mem[i]
                        self.add(&LocalGet(i));
                        self.add(&I32Load8U(from_mem));
                        self.add(&LocalSet(current_byte));

                        // i++;
                        self.add(&LocalGet(i));
                        self.add(&I32Const(1));
                        self.add(&I32Add);
                        self.add(&LocalSet(i));

                        // continue;
                        self.add(&Br(0));
                    }
                    self.add(&End);
                }
                self.add(&End);

                // if (mode === 0) ...
                self.add(&LocalGet(mode));
                self.add(&I32Eqz);
                self.add(&If(wasm_encoder::BlockType::Empty));
                {
                    // Flush run
                    self.i16(&LocalGet(current_count));
                    self.byte(&[&LocalGet(current_byte)]);
                }
                self.add(&Else);
                {
                    // Flush literals
                    self.i16(&LocalGet(current_count));
                    self.bytes(
                        mem_idx,
                        &[&LocalGet(i), &LocalGet(current_count), &I32Sub],
                        &[&LocalGet(current_count)],
                    );
                }
                self.add(&End);

                // mode = (mode + 1) % 2; // toggle
                self.add(&LocalGet(mode));
                self.add(&I32Const(1));
                self.add(&I32Add);
                self.add(&I32Const(2));
                self.add(&I32RemU);
                self.add(&LocalSet(mode));

                // continue;
                self.add(&Br(0))
            }
            self.add(&End);
        }
        self.add(&End);

        // Write final length into dummy slot
        self.add(&GlobalGet(self.cur));
        self.add(&I32Sub); // cur_old - cur_new
        self.add(&I32Const(-1));
        self.add(&I32Mul); // cur_new - cur_old
        self.add(&I32Const(4));
        self.add(&I32Sub); // cur_new - cur_old - 4
        self.add(&I32Store(self.in_buf())) // uses first global.get as its address
    }

    fn end(&mut self) {
        use Instruction::*;
        self.f.instruction(&End);
    }
}

enum ErrCode {
    WrongNumMemories = 1,
    WrongNumTables = 2,
    WrongNumMutableGlobals = 3,
    WrongNumParams = 4,

    WrongValType = 10,
    UnknownValType = 11,

    FailedToGrowMemory = 20,
}

struct ReplayReader<'a> {
    f: &'a mut Function,
    buf: u32,
    cur: u32,
    errcode: u32,
}

impl ReplayReader<'_> {
    fn in_buf(&self) -> MemArg {
        MemArg {
            offset: 0,
            align: 0,
            memory_index: self.buf,
        }
    }

    fn add(&mut self, ins: &Instruction) {
        self.f.instruction(ins);
    }

    fn instructions<'a>(&mut self, instructions: &Instructions) {
        for ins in instructions {
            self.f.instruction(ins);
        }
    }

    fn abort(&mut self, errcode: ErrCode) {
        use Instruction::*;
        self.add(&I32Const(errcode as i32));
        self.add(&GlobalSet(self.errcode));
        self.add(&Unreachable);
    }

    fn bump_cur(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.add(&I32Add);
        self.add(&GlobalSet(self.cur));
    }

    fn bump_cur_imm(&mut self, n: i32) {
        use Instruction::*;
        self.add(&I32Const(n));
        self.bump_cur();
    }

    fn byte(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(1);
        self.add(&I32Load8U(self.in_buf()));
    }

    fn expect_byte(&mut self, n: u8, errcode: ErrCode) {
        use Instruction::*;
        self.add(&Block(wasm_encoder::BlockType::Empty));
        {
            self.add(&GlobalGet(self.cur));
            self.add(&I32Load8U(self.in_buf()));
            self.add(&I32Const(n as i32));
            self.add(&I32Eq);
            self.add(&BrIf(0));
            self.abort(errcode);
        }
        self.add(&End);
        self.bump_cur_imm(1);
    }

    fn i32(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(4);
        self.add(&I32Load(self.in_buf()));
    }

    fn expect_i32(&mut self, n: i32, errcode: ErrCode) {
        use Instruction::*;
        self.add(&Block(wasm_encoder::BlockType::Empty));
        {
            self.add(&GlobalGet(self.cur));
            self.add(&I32Load(self.in_buf()));
            self.add(&I32Const(n));
            self.add(&I32Eq);
            self.add(&BrIf(0));
            self.abort(errcode);
        }
        self.add(&End);
        self.bump_cur_imm(4);
    }

    fn i64(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(8);
        self.add(&I64Load(self.in_buf()));
    }

    fn f32(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(4);
        self.add(&F32Load(self.in_buf()));
    }

    fn f64(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(8);
        self.add(&F64Load(self.in_buf()));
    }

    fn v128(&mut self) {
        use Instruction::*;
        self.add(&GlobalGet(self.cur));
        self.bump_cur_imm(16);
        self.add(&V128Load(self.in_buf()));
    }

    fn val(&mut self, val_type: &ValType) {
        match val_type {
            ValType::I32 => {
                self.expect_byte(0x7F, ErrCode::WrongValType);
                self.i32();
            }
            ValType::I64 => {
                self.expect_byte(0x7E, ErrCode::WrongValType);
                self.i64();
            }
            ValType::F32 => {
                self.expect_byte(0x7D, ErrCode::WrongValType);
                self.f32();
            }
            ValType::F64 => {
                self.expect_byte(0x7C, ErrCode::WrongValType);
                self.f64();
            }
            ValType::V128 => {
                self.expect_byte(0x7B, ErrCode::WrongValType);
                self.v128();
            }
            ValType::Ref(ref_type) => todo!(),
        }
    }

    fn skip_val(&mut self, tmp_i32: u32) {
        use Instruction::*;

        self.byte();
        self.add(&LocalSet(tmp_i32));

        self.add(&Block(wasm_encoder::BlockType::Empty));
        {
            self.add(&Block(wasm_encoder::BlockType::Empty));
            {
                self.add(&Block(wasm_encoder::BlockType::Empty));
                {
                    self.add(&Block(wasm_encoder::BlockType::Empty));
                    {
                        // Skip i32
                        self.add(&LocalGet(tmp_i32));
                        self.add(&I32Const(0x7F));
                        self.add(&I32Eq);
                        self.add(&BrIf(0));

                        // Skip i64
                        self.add(&LocalGet(tmp_i32));
                        self.add(&I32Const(0x7E));
                        self.add(&I32Eq);
                        self.add(&BrIf(1));

                        // Skip f32
                        self.add(&LocalGet(tmp_i32));
                        self.add(&I32Const(0x7D));
                        self.add(&I32Eq);
                        self.add(&BrIf(0));

                        // Skip f64
                        self.add(&LocalGet(tmp_i32));
                        self.add(&I32Const(0x7C));
                        self.add(&I32Eq);
                        self.add(&BrIf(1));

                        // Skip v128
                        self.add(&LocalGet(tmp_i32));
                        self.add(&I32Const(0x7B));
                        self.add(&I32Eq);
                        self.add(&BrIf(2));

                        // Unknown val type
                        self.abort(ErrCode::UnknownValType);
                    }
                    self.add(&End);

                    self.bump_cur_imm(4);
                    self.add(&Br(2));
                }
                self.add(&End);

                self.bump_cur_imm(8);
                self.add(&Br(1));
            }
            self.add(&End);

            self.bump_cur_imm(16);
            self.add(&Br(0));
        }
        self.add(&End);
    }

    /// It is your responsibility to ensure that base has the correct address type for the given memory.
    /// Len should always be i32 because the source buffer is i32.
    fn bytes<'a>(&mut self, mem_idx: u32, base: &Instruction, len: &Instructions) {
        use Instruction::*;
        self.add(base); // dst offset
        self.add(&GlobalGet(self.cur)); // src offset
        self.instructions(len); // len
        self.add(&MemoryCopy {
            src_mem: self.buf,
            dst_mem: mem_idx,
        });
        self.instructions(len);
        self.bump_cur();
    }

    fn rle(&mut self, mem_idx: u32, tmp_i32_1: u32, tmp_i32_2: u32, tmp_i32_3: u32) {
        use Instruction::*;
        let i = tmp_i32_1;
        let end = tmp_i32_2;
        let run_length = tmp_i32_3;

        // i = 0;
        self.add(&I32Const(0));
        self.add(&LocalSet(i));

        // end = len + cur (after len);
        self.i32();
        self.add(&GlobalGet(self.cur));
        self.add(&I32Add);
        self.add(&LocalSet(end));

        // while (cur < end)
        self.add(&Loop(wasm_encoder::BlockType::Empty));
        {
            // memory.fill(addr=i, val=run_byte, len=run_length)
            self.add(&LocalGet(i));
            self.byte();
            self.i32();
            self.add(&LocalTee(run_length));
            self.add(&MemoryFill(mem_idx));

            // i += run_length
            self.add(&LocalGet(i));
            self.add(&LocalGet(run_length));
            self.add(&I32Add);
            self.add(&LocalSet(i));

            self.add(&GlobalGet(self.cur));
            self.add(&LocalGet(end));
            self.add(&I32LtU);
            self.add(&BrIf(0));
        }
        self.add(&End);
    }

    fn end(&mut self) {
        use Instruction::*;
        self.add(&End);
    }
}
