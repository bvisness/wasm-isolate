use std::{fs, io::Write, path::Path};

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

    fs::create_dir_all(&out).expect("unable to create output directory");
    for mode in [InstrumentationMode::Record, InstrumentationMode::Replay] {
        let mut module = Module::new();
        let mut reencoder = RoundtripReencoder {};

        let recorder_base_typeidx = types.len() as u32;
        let recorder_base_funcidx = func_types.len() as u32;
        let buf_memidx = memory_types.len() as u32;
        let cur_globalidx = global_types.len() as u32;

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
                            // TODO: Add func types for the init/call stubs. (We may not
                            // have the right signature in the module already, and we
                            // definitely don't know the index.)
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

                    // TODO: Add imports for instrumentation.

                    module.section(&import_section);
                }
                Section::Function => {
                    let mut function_section = FunctionSection::new();

                    // Pass through all existing functions
                    for (i, _) in defined_funcs.iter().enumerate() {
                        let idx = num_imported.functions + i as u32;
                        function_section.function(reencoder.type_index(func_types[idx as usize]));
                    }

                    match mode {
                        InstrumentationMode::Record => {
                            // Add types for recorders
                            for i in 0..args.funcs.len() {
                                function_section.function(
                                    reencoder.type_index(recorder_base_typeidx + i as u32),
                                );
                            }
                        }
                        InstrumentationMode::Replay => {}
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

                    match mode {
                        InstrumentationMode::Record => {
                            // Create memory for buffer
                            memory_section.memory(wasm_encoder::MemoryType {
                                minimum: 128,
                                maximum: Some(128),
                                memory64: false,
                                shared: false,
                                page_size_log2: None,
                            });
                        }
                        InstrumentationMode::Replay => {}
                    }

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

                    match mode {
                        InstrumentationMode::Record => {
                            // Define the global for the cursor
                            global_section.global(
                                wasm_encoder::GlobalType {
                                    val_type: wasm_encoder::ValType::I32,
                                    mutable: true,
                                    shared: false,
                                },
                                &ConstExpr::i32_const(0),
                            );
                        }
                        InstrumentationMode::Replay => {}
                    }

                    module.section(&global_section);
                }
                Section::Export => {
                    let mut export_section = ExportSection::new();

                    // Pass through all existing exports
                    for export in &exports {
                        export_section.export(export.name, export.kind.into(), export.index);
                    }

                    match mode {
                        InstrumentationMode::Record => {
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
                        }
                        InstrumentationMode::Replay => {
                            // TODO: Export the init and call stubs, and also every
                            // function probably for ref reasons
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

                    match mode {
                        InstrumentationMode::Record => {
                            // Generate recorders
                            for idx_to_record in &args.funcs {
                                let func_type_idx = func_types[*idx_to_record as usize];
                                let func_type = types[func_type_idx as usize].unwrap_func();

                                let mut w = RecorderWriter {
                                    f: &mut Function::new(vec![]),
                                    buf: buf_memidx,
                                    cur: cur_globalidx,
                                };

                                // Write func idx to begin call state
                                w.i32_imm(*idx_to_record as i32);

                                // Write memories
                                for (i, mem_type) in memory_types.iter().enumerate() {
                                    match mem_type.index_type() {
                                        ValType::I32 => {
                                            w.i32(&Instruction::MemorySize(i as u32));
                                            w.bytes(
                                                i as u32,
                                                &Instruction::I32Const(0),
                                                &[
                                                    &Instruction::MemorySize(i as u32),
                                                    &Instruction::I32Const(65536),
                                                    &Instruction::I32Mul,
                                                ],
                                            );
                                        }
                                        ValType::I64 => {
                                            w.i64(&Instruction::MemorySize(i as u32));
                                            w.bytes(
                                                i as u32,
                                                &Instruction::I64Const(0),
                                                &[
                                                    &Instruction::MemorySize(i as u32),
                                                    &Instruction::I64Const(65536),
                                                    &Instruction::I64Mul,
                                                    &Instruction::I32WrapI64,
                                                ],
                                            );
                                        }
                                        _ => panic!("invalid address type"),
                                    }
                                }

                                // Write tables
                                // TODO

                                // Write mutable globals
                                for (i, global_type) in global_types.iter().enumerate() {
                                    if global_type.mutable {
                                        w.val(
                                            &global_type.content_type,
                                            &Instruction::GlobalGet(i as u32),
                                        )
                                    }
                                }

                                // Write params
                                for (i, param_type) in func_type.params().iter().enumerate() {
                                    w.val(param_type, &Instruction::LocalGet(i as u32));
                                }

                                w.end();
                                code_section.function(w.f);
                            }
                        }
                        InstrumentationMode::Replay => {}
                    }

                    module.section(&code_section);
                }
                _ => todo!("{:?} section", section),
            }
        }
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

    Ok(())
}

struct Func<'a> {
    type_idx: u32,
    locals: Vec<(u32, ValType)>,
    instructions: Vec<Operator<'a>>,
}

enum InstrumentationMode {
    Record,
    Replay,
}

struct RecorderWriter<'a> {
    f: &'a mut Function,
    buf: u32,
    cur: u32,
}

type Instructions<'a> = [&'a Instruction<'a>];

impl RecorderWriter<'_> {
    fn in_buf(&self) -> MemArg {
        MemArg {
            offset: 0,
            align: 0,
            memory_index: self.buf,
        }
    }

    fn instructions<'a>(&mut self, instructions: &Instructions) {
        for ins in instructions {
            self.f.instruction(ins);
        }
    }

    fn bump_cur<'a>(&mut self, delta: &Instructions) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.instructions(delta);
        self.f.instruction(&Instruction::I32Add);
        self.f.instruction(&Instruction::GlobalSet(self.cur));
    }

    fn bump_cur_imm(&mut self, n: i32) {
        self.bump_cur(&[&Instruction::I32Const(n)]);
    }

    fn byte(&mut self, value: &Instructions) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.instructions(value);
        self.f.instruction(&Instruction::I32Store8(self.in_buf()));
        self.bump_cur_imm(1);
    }

    fn byte_imm(&mut self, byte: i32) {
        self.byte(&[&Instruction::I32Const(byte)]);
    }

    fn i32(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::I32Store(self.in_buf()));
        self.bump_cur_imm(4);
    }

    fn i32_imm(&mut self, n: i32) {
        self.i32(&Instruction::I32Const(n));
    }

    fn i64(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::I64Store(self.in_buf()));
        self.bump_cur_imm(8);
    }

    fn f32(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::F32Store(self.in_buf()));
        self.bump_cur_imm(4);
    }

    fn f64(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::F64Store(self.in_buf()));
        self.bump_cur_imm(8);
    }

    fn v128(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::V128Store(self.in_buf()));
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
                self.i64(load);
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
    fn bytes<'a>(&mut self, mem_idx: u32, base: &Instruction, len: &Instructions) {
        self.f.instruction(&Instruction::GlobalGet(self.cur)); // dst offset
        self.f.instruction(base); // src offset
        self.instructions(len); // len
        self.f.instruction(&Instruction::MemoryCopy {
            src_mem: mem_idx,
            dst_mem: self.buf,
        });
        self.bump_cur(len);
    }

    fn end(&mut self) {
        self.f.instruction(&Instruction::End);
    }
}
