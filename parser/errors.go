package parser

import "fmt"

// Manual translation of the "Errors" section

type Position struct {
	file   string
	line   int
	column int
}

type Region struct {
	left  *Position
	right *Position
}

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

func _position(s *Stream, pos int) *Position {
	return &Position{
		file:   s.name,
		line:   -1,
		column: pos,
	}
}

func _region(s *Stream, left int, right int) *Region {
	return &Region{
		left:  _position(s, left),
		right: _position(s, right),
	}
}

func _error(s *Stream, pos int, msg string) {
	panic(CodeError{
		region: _region(s, pos, pos),
		msg:    msg,
	})
}

func _require(b bool, s *Stream, pos int, msg string) {
	if !b {
		_error(s, pos, msg)
	}
}

// Skipping guarded versions of things

func _expect(b byte, s *Stream, msg string) {
	_require(_get(s) == b, s, _pos(s)-1, msg)
}

func _illegal(s *Stream, pos int, b byte) {
	_error(s, pos, "illegal opcode "+_string_of_byte(b))
}

func _illegal2(s *Stream, pos int, b byte, n int) {
	_error(s, pos, "illegal opcode "+_string_of_byte(b)+" "+_string_of_multi(n))
}

// Skipping _at (partial application)
