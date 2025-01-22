package parser

import "errors"

// Manual translation of the "Decoding stream" section

type Stream struct {
	name  string
	bytes string
	pos   int
}

var EOS = errors.New("EOS")

func _len(s *Stream) int {
	return len(s.bytes)
}

func _pos(s *Stream) int {
	return s.pos
}

func _eos(s *Stream) bool {
	return _pos(s) == _len(s)
}

func _reset(s *Stream, pos int) {
	s.pos = pos
}

func _check(n int, s *Stream) {
	if _pos(s)+n > _len(s) {
		panic(EOS)
	}
}

func _skip(n int, s *Stream) {
	if n < 0 {
		panic(EOS)
	} else {
		_check(n, s)
		s.pos = s.pos + n
	}
}

func _read(s *Stream) byte {
	return s.bytes[s.pos]
}

func _peek(s *Stream) (byte, bool) {
	if _eos(s) {
		return 0, false
	} else {
		return _read(s), true
	}
}

func _get(s *Stream) byte {
	_check(1, s)
	b := _read(s)
	_skip(1, s)
	return b
}

func _get_string(n int, s *Stream) string {
	i := _pos(s)
	_skip(n, s)
	return s.bytes[i : i+n]
}
