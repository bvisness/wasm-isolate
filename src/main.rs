use std::{collections::HashSet, fs::File};

use anyhow::{bail, Result};
use clap::Parser as _;
use wasmparser::{CompositeInnerType, FuncType, HeapType, Operator, Parser, Payload::*, ValType};

/// Simple program to greet a person
#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    filename: String,

    #[arg(short, long, required = true)]
    funcs: Vec<u32>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let filename = args.filename;
    let mut reader = get_reader(filename);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let parser = Parser::new(0);

    let mut types: Vec<CompositeInnerType> = vec![];
    let mut num_imported_functions = 0;
    let mut func_types: Vec<(u32, FuncType)> = vec![];

    let mut current_func = 0;
    let mut first_func: bool = true;

    let mut defined_funcs: Vec<Func> = vec![];

    for payload in parser.parse_all(&buf) {
        match payload? {
            // Sections for WebAssembly modules
            TypeSection(r) => {
                for rg in r {
                    for t in rg?.into_types() {
                        types.push(t.composite_type.inner);
                    }
                }
            }
            ImportSection(r) => {
                for import in r {
                    match import?.ty {
                        wasmparser::TypeRef::Func(..) => {
                            num_imported_functions += 1;
                        }
                        _ => {}
                    }
                }

                // Yell at the user if they try to keep an imported function
                for func_idx in &args.funcs {
                    if *func_idx < num_imported_functions {
                        bail!("Cannot isolate func {func_idx} because it is an imported function");
                    }
                }
            }
            FunctionSection(r) => {
                for f in r {
                    let type_idx = f?;
                    if let CompositeInnerType::Func(ft) = &types[type_idx as usize] {
                        func_types.push((type_idx, ft.clone()));
                    } else {
                        bail!("ERROR: Type {} is not a func type", type_idx)
                    }
                }
            }
            TableSection(_) => { /* ... */ }
            MemorySection(_) => { /* ... */ }
            TagSection(_) => { /* ... */ }
            GlobalSection(_) => { /* ... */ }
            ExportSection(_) => { /* ... */ }
            StartSection { .. } => { /* ... */ }
            ElementSection(_) => { /* ... */ }
            DataCountSection { .. } => { /* ... */ }
            DataSection(_) => { /* ... */ }

            // Here we know how many functions we'll be receiving as
            // `CodeSectionEntry`, so we can prepare for that, and
            // afterwards we can parse and handle each function
            // individually.
            CodeSectionStart { .. } => {
                current_func = num_imported_functions;
            }
            CodeSectionEntry(body) => {
                if first_func {
                    first_func = false
                } else {
                    current_func += 1;
                }

                let (type_index, ty) = &func_types[(current_func - num_imported_functions) as usize];

                let mut func = Func {
                    original_index: current_func,
                    index_in_code_section: current_func - num_imported_functions,

                    type_index: *type_index,
                    ty: ty.clone(),

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

            _ => {}
        }
    }

    let mut func_queue: Vec<u32> = vec![];
    for func in args.funcs {
        func_queue.push(func);
    }

    let mut processed_funcs = HashSet::<u32>::new();

    while !func_queue.is_empty() {
        let func_idx = *func_queue.first().expect("defined function");
        func_queue.remove(0);
        processed_funcs.insert(func_idx);

        let func = &defined_funcs[(func_idx - num_imported_functions) as usize];
        let uses = get_func_uses(func);
        for used_func in &uses.live_funcs {
            if *used_func < num_imported_functions {
                // TODO: track imports
            } else if !processed_funcs.contains(used_func) {
                func_queue.push(*used_func);
            }
        }

        println!("{:#?}", uses);
    }

    Ok(())
}

fn get_reader(filename: String) -> Box<dyn std::io::Read> {
    if filename == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(File::open(filename).expect("Failed to open file"))
    }
}

struct Func<'a> {
    original_index: u32,
    index_in_code_section: u32,

    type_index: u32,
    ty: FuncType,

    locals: Vec<(u32, ValType)>,
    instructions: Vec<Operator<'a>>,
}

#[derive(Default, Debug)]
struct Uses {
    live_types: Vec<u32>,
    live_funcs: Vec<u32>,
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

    fn merge(&mut self, mut other: Uses) {
        self.live_types.append(&mut other.live_types);
        self.live_types.sort();
        self.live_types.dedup();

        self.live_funcs.append(&mut other.live_funcs);
        self.live_funcs.sort();
        self.live_funcs.dedup();
    }
}

fn get_func_uses(func: &Func) -> Uses {
    let mut res = Uses::default();

    res.live_types.push(func.type_index);
    res.merge(get_functype_uses(&func.ty));
    for (_, ty) in &func.locals {
        res.merge(get_valtype_uses(ty));
    }
    for instr in &func.instructions {
        res.merge(get_instr_uses(instr));
    }

    res
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

fn get_valtype_uses(ty: &ValType) -> Uses {
    match ty {
        ValType::Ref(ref_type) => get_heaptype_uses(&ref_type.heap_type()),
        _ => Uses::default(),
    }
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

fn get_instr_uses(instr: &Operator<'_>) -> Uses {
    match instr {
        Operator::Call { function_index } => Uses::single_func(*function_index),
        Operator::CallIndirect { type_index, table_index } => Uses::single_type(*type_index),
        _ => Uses::default(),
    }
}