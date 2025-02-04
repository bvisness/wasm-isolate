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

func _operatorAtAt_2[T any](x T, region *OSource_Region) *OSource_Phrase[T] {
	return &OSource_Phrase[T]{
		it: x,
		at: region,
	}
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

func _Int32_of_int_1(a OInt) OInt32 {
	return OInt32(a)
}

func _Int32_to_int_1(a OInt32) OInt {
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

func _I32_convert_wrap_i64_1(a OInt64) OInt32 {
	return OInt32(a)
}
func _I32_lt_u_2(a, b OInt32) bool {
	return uint32(a) < uint32(b)
}

func _I32_le_u_2(a, b OInt32) bool {
	return uint32(a) <= uint32(b)
}

func _I32_to_int_u_1(a OInt32) OInt {
	return OInt(a)
}

func _Int64_of_int_1(a OInt) OInt64 {
	return OInt64(a)
}

func _I64_convert_extend_i32_u_1(a OInt32) OInt64 {
	return OInt64(a)
}

func _Int64_to_int_1(a OInt64) OInt {
	return OInt(a)
}

func _Int64_to_int32_1(a OInt64) OInt32 {
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

func _F32_of_bits_1(bits OInt32) float32 {
	return math.Float32frombits(uint32(bits))
}

func _F64_of_bits_1(bits OInt64) float64 {
	return math.Float64frombits(uint64(bits))
}

func _operatorLSL_int_2(a, b OInt) OInt {
	return a << b
}

// Manual translation of the "Types" section

type V128 [16]byte

func _V128_of_bits_1(bits string) V128 {
	var res V128
	copy(res[:], bits)
	return res
}
