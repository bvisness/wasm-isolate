package parser

import "fmt"

// Manual translation of the "Errors" section

type CodeError struct {
	region *Region
	msg    string
}

func _string_of_byte_1(b OInt) string {
	return fmt.Sprintf("%02x", b)
}

func _string_of_multi_1(n OInt32) string {
	return fmt.Sprintf("%d", n)
}

func _position_2(s *Stream, pos OInt) *Pos {
	return &Pos{
		file:   s.name,
		line:   -1,
		column: pos,
	}
}

func _region_3(s *Stream, left OInt, right OInt) *Region {
	return &Region{
		left:  _position_2(s, left),
		right: _position_2(s, right),
	}
}

func _error_3[T any](s *Stream, pos OInt, msg string) T {
	panic(CodeError{
		region: _region_3(s, pos, pos),
		msg:    msg,
	})
}

func _require_4[T any](b bool, s *Stream, pos OInt, msg string) T {
	if !b {
		return _error_3[T](s, pos, msg)
	}
	var zero T
	return zero
}

// Skipping guarded versions of things

func _expect_3[T any](b OInt, s *Stream, msg string) T {
	return _require_4[T](_get_1(s) == b, s, _pos_1(s)-1, msg)
}

func _illegal_3[T any](s *Stream, pos OInt, b OInt) T {
	return _error_3[T](s, pos, "illegal opcode "+_string_of_byte_1(b))
}

func _illegal2_4[T any](s *Stream, pos OInt, b OInt, n OInt32) T {
	return _error_3[T](s, pos, "illegal opcode "+_string_of_byte_1(b)+" "+_string_of_multi_1(n))
}

func _at_2[T any](f func(s *Stream) T, s *Stream) *Phrase[T] {
	left := _pos_1(s)
	x := f(s)
	right := _pos_1(s)
	return &Phrase[T]{
		at: _region_3(s, left, right),
		it: x,
	}
}

func _at_1[T any](f func(s *Stream) T) func(s *Stream) *Phrase[T] {
	return func(s *Stream) *Phrase[T] {
		return _at_2(f, s)
	}
}
