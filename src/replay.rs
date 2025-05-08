use std::{fs, io::Write};

use anyhow::Result;
use wasm_encoder::{
    reencode::{Reencode, RoundtripReencoder},
    CodeSection, ConstExpr, EntityType, ExportSection, Function, FunctionSection, GlobalSection,
    ImportSection, Instruction, MemArg, Module, TypeSection,
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
                sections.push(Section::raw(5, &buf[r.range()]));
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

    //
    // Record instrumentation
    //

    let mut out = Module::new();
    let mut reencoder = RoundtripReencoder {};

    let buf_memidx = memory_types.len() as u32;
    let cur_globalidx = global_types.len() as u32;

    for section in sections {
        match section {
            Section::Passthrough(sec) => {
                out.section(&sec);
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

                // TODO: Add func types for the init/call stubs. (We may not
                // have the right signature in the module already, and we
                // definitely don't know the index.)

                out.section(&type_section);
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

                out.section(&import_section);
            }
            Section::Function => {
                let mut function_section = FunctionSection::new();

                // Pass through all existing functions
                for (i, _) in defined_funcs.iter().enumerate() {
                    let idx = num_imported.functions + i as u32;
                    function_section.function(reencoder.type_index(func_types[idx as usize]));
                }

                // Add types for recorders
                for idx_to_record in &args.funcs {
                    let func_type_idx = func_types[*idx_to_record as usize];
                    function_section.function(reencoder.type_index(func_type_idx));
                }

                out.section(&function_section);
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

                // Define the global for the cursor
                global_section.global(
                    wasm_encoder::GlobalType {
                        val_type: wasm_encoder::ValType::I32,
                        mutable: true,
                        shared: false,
                    },
                    &ConstExpr::i32_const(0),
                );

                out.section(&global_section);
            }
            Section::Export => {
                let mut export_section = ExportSection::new();

                // Pass through all existing exports
                for export in &exports {
                    export_section.export(export.name, export.kind.into(), export.index);
                }

                // TODO: Export the init and call stubs, and also every
                // function probably for ref reasons

                out.section(&export_section);
            }
            Section::Code => {
                let mut code_section = CodeSection::new();

                // Pass through all existing functions
                for func in &defined_funcs {
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

                // Generate recorders
                for idx_to_record in &args.funcs {
                    let func_type_idx = func_types[*idx_to_record as usize];
                    let func_type = types[func_type_idx as usize].unwrap_func();

                    let mut w = RecorderWriter {
                        f: &mut Function::new(vec![]),
                        buf: buf_memidx,
                        cur: cur_globalidx,
                    };

                    // Write memories
                    for (i, mem_type) in memory_types.iter().enumerate() {
                        let size = &Instruction::MemorySize(i as u32);
                        match mem_type.index_type() {
                            ValType::I32 => w.i32(size),
                            ValType::I64 => w.i64(size),
                            _ => panic!("invalid address type"),
                        }

                        w.bytes(
                            i as u32,
                            match mem_type.index_type() {
                                ValType::I32 => &Instruction::I32Const(0),
                                ValType::I64 => &Instruction::I64Const(0),
                                _ => panic!("invalid address type"),
                            },
                            size,
                        );
                    }

                    // Write tables
                    // TODO

                    // Write mutable globals
                    for (i, global_type) in global_types.iter().enumerate() {
                        if global_type.mutable {
                            w.val(&global_type.content_type, &Instruction::GlobalGet(i as u32))
                        }
                    }

                    // Write params
                    for (i, param_type) in func_type.params().iter().enumerate() {
                        w.val(param_type, &Instruction::LocalGet(i as u32));
                    }

                    w.end();
                    code_section.function(w.f);
                }

                out.section(&code_section);
            }
            _ => todo!("{:?} section", section),
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

    Ok(())
}

struct Func<'a> {
    type_idx: u32,
    locals: Vec<(u32, ValType)>,
    instructions: Vec<Operator<'a>>,
}

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

    fn bump_cur(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::I32Add);
        self.f.instruction(&Instruction::GlobalSet(self.cur));
    }

    fn bump_cur_imm(&mut self, n: i32) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(&Instruction::I32Const(n));
        self.f.instruction(&Instruction::I32Add);
        self.f.instruction(&Instruction::GlobalSet(self.cur));
    }

    fn byte_imm(&mut self, byte: i32) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(&Instruction::I32Const(byte));
        self.f.instruction(&Instruction::I32Store8(self.in_buf()));
        self.bump_cur_imm(1);
    }

    fn i32(&mut self, ins: &Instruction) {
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(ins);
        self.f.instruction(&Instruction::I32Store(self.in_buf()));
        self.bump_cur_imm(4);
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
    fn bytes(&mut self, mem_idx: u32, base: &Instruction, len: &Instruction) {
        self.f.instruction(base);
        self.f.instruction(&Instruction::GlobalGet(self.cur));
        self.f.instruction(len);
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
