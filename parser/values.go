package parser

import (
	"github.com/bvisness/wasm-isolate/leb128"
	"github.com/bvisness/wasm-isolate/utils"
)

// Manual translation of the "Generic values" section

func _bit_2(i int, n int) bool {
	return n&(1<<i) != 0
}

func _byte(s *Stream) int {
	return _get(s)
}

func _u32(s *Stream) uint32 {
	u64, _ := utils.Must2(leb128.DecodeU64(s))
	return uint32(u64)
}

func _u64(s *Stream) uint64 {
	u64, _ := utils.Must2(leb128.DecodeU64(s))
	return u64
}

func _len32(s *Stream) int {
	pos := _pos(s)
	n := _u32(s)
	if int(n) < _len(s)-pos {
		return int(n)
	} else {
		_error_3(s, pos, "length out of bounds")
		return 0
	}
}

func _string(s *Stream) string {
	n := _len32(s)
	return _get_string_2(n, s)
}

func _list_3[T any](f func(s *Stream) T, n int, s *Stream) []T {
	res := make([]T, n)
	for i := range n {
		res[i] = f(s)
	}
	return res
}

func _opt_3[T any](f func(s *Stream) T, b bool, s *Stream) *T {
	if b {
		return _Some(f(s))
	} else {
		return nil
	}
}

func _vec_2[T any](f func(s *Stream) T, s *Stream) []T {
	n := _len32(s)
	return _list_3(f, n, s)
}
