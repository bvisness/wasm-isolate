#pragma endian little

#include <std/mem.pat>

struct Vec<T> {
  u32 count;
  T items[count];
};

struct Memory {
  u64 sizePages;
  Vec<u8> contents;
};

struct Table {
  // TODO
};

enum ValType : u8 {
  I32 = 0x7F,
  I64 = 0x7E,
  F32 = 0x7D,
  F64 = 0x7C,

  V128 = 0x7B,

  // TODO: Refs
};

struct ValCtor {
  ValType type;
  match (type) {
    (ValType::I32): s32 value;
    (ValType::I64): s64 value;
    (ValType::F32): float value;
    (ValType::F64): double value;

    (ValType::V128): u8 value[16];
  }
};

// ref_ctor = null_ctor
//          | funcref_ctor
//          | externref_ctor
//          | struct_ctor
//          | array_ctor
//          | i31_ctor
// null_ctor = 0x00
// funcref_ctor = 0x70 idx<func>
// externref_ctor = TODO
// struct_ctor = TODO
// array_ctor = TODO
// i31_ctor = TODO

struct CallState {
  u32 funcIdx;
  Vec<Memory> memories;
  //Vec<Table> tables;
  Vec<ValCtor> mutableGlobals;
  Vec<ValCtor> params;
};

CallState calls[while($ < std::mem::size())] @ 0x00;
