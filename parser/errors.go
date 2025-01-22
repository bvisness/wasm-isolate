package parser

import "fmt"

// Manual translation of the "Errors" section

type CodeError struct {
	region *Region
	msg    string
}

func _string_of_byte(b byte) string {
	return fmt.Sprintf("%02x", b)
}

func _string_of_multi(n int) string {
	return fmt.Sprintf("%d", n)
}

func _position_2(s *Stream, pos int) *Pos {
	return &Pos{
		file:   s.name,
		line:   -1,
		column: pos,
	}
}

func _region_3(s *Stream, left int, right int) *Region {
	return &Region{
		left:  _position_2(s, left),
		right: _position_2(s, right),
	}
}

func _error_3(s *Stream, pos int, msg string) Void {
	panic(CodeError{
		region: _region_3(s, pos, pos),
		msg:    msg,
	})
}

func _require_4(b bool, s *Stream, pos int, msg string) Void {
	if !b {
		_error_3(s, pos, msg)
	}
	return nil
}

// Skipping guarded versions of things

func _expect_3(b byte, s *Stream, msg string) Void {
	_require_4(_get(s) == b, s, _pos(s)-1, msg)
	return nil
}

func _illegal_3(s *Stream, pos int, b byte) Void {
	_error_3(s, pos, "illegal opcode "+_string_of_byte(b))
	return nil
}

func _illegal2_4(s *Stream, pos int, b byte, n int) Void {
	_error_3(s, pos, "illegal opcode "+_string_of_byte(b)+" "+_string_of_multi(n))
	return nil
}

func _at[T any](f func(s *Stream) T) func(s *Stream) *Phrase[T] {
	return func(s *Stream) *Phrase[T] {
		return _at_2(f, s)
	}
}

func _at_2[T any](f func(s *Stream) T, s *Stream) *Phrase[T] {
	left := _pos(s)
	x := f(s)
	right := _pos(s)
	return &Phrase[T]{
		at: _region_3(s, left, right),
		it: x,
	}
}
