package parser

import (
	"math"

	"golang.org/x/exp/constraints"
)

// OCaml types mapped to Go types with various extra operations mimicking
// OCaml's operations.

type OInt int     // platform-dependent signed/unsigned int
type OInt32 int32 // signed/unsigned int32
type OInt64 int64 // signed/unsigned int64

func _operatorEq_2[T comparable](a, b T) bool {
	return a == b
}

func _operatorNotEq_2[T comparable](a, b T) bool {
	return a != b
}

func _operatorGt_2[T constraints.Ordered](a, b T) bool {
	return a > b
}

func _operatorLt_2[T constraints.Ordered](a, b T) bool {
	return a < b
}

func _operatorGte_2[T constraints.Ordered](a, b T) bool {
	return a >= b
}

func _bool_operatorOr_2(a, b bool) bool {
	return a || b
}

func _int_operatorPlus_2(a, b OInt) OInt {
	return a + b
}

func _int_operatorMinus_2(a, b OInt) OInt {
	return a - b
}

func _int_operatorland_2(a, b OInt) OInt {
	return a & b
}

func _int_operatorlsl_2(a, b OInt) OInt {
	return a << b
}

func _Int32_of_int(a OInt) OInt32 {
	return OInt32(a)
}

func _Int32_to_int(a OInt32) OInt {
	return OInt(a)
}

func _Int32_add_2(a, b OInt32) OInt32 {
	return a + b
}

func _Int32_shift_left_2(a, b OInt32) OInt32 {
	return a << b
}

func _Int32_logand_2(a, b OInt32) OInt32 {
	return a & b
}

func _I32_convert_wrap_i64(a OInt64) OInt32 {
	return OInt32(a)
}
func _I32_lt_u_2(a, b OInt32) bool {
	return uint32(a) < uint32(b)
}

func _I32_le_u_2(a, b OInt32) bool {
	return uint32(a) <= uint32(b)
}

func _I32_to_int_u(a OInt32) OInt {
	return OInt(a)
}

func _Int64_of_int(a OInt) OInt64 {
	return OInt64(a)
}

func _I64_convert_extend_i32_u(a OInt32) OInt64 {
	return OInt64(a)
}

func _Int64_to_int(a OInt64) OInt {
	return OInt(a)
}

func _Int64_to_int32(a OInt64) OInt32 {
	return OInt32(a)
}

func _Int64_add_2(a, b OInt64) OInt64 {
	return a + b
}

func _Int64_shift_left_2(a, b OInt64) OInt64 {
	return a << b
}

func _Int64_logor_2(a, b OInt64) OInt64 {
	return a | b
}

func _Int64_logxor_2(a, b OInt64) OInt64 {
	return a ^ b
}

func _F32_of_bits(bits OInt32) float32 {
	return math.Float32frombits(uint32(bits))
}

func _F64_of_bits(bits OInt64) float64 {
	return math.Float64frombits(uint64(bits))
}

func _operatorLSL_int_2(a, b OInt) OInt {
	return a << b
}

// Manual translation of the "Types" section

type Void any

func _var(s *Stream) OInt32 {
	return _u32(s)
}

type Instruction interface{}
