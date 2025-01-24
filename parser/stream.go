package parser

import (
	"errors"
	"io"

	"github.com/bvisness/wasm-isolate/utils"
)

// Manual translation of the "Decoding stream" section

type Stream struct {
	name  string
	bytes string
	pos   int
}

var _ io.Reader = &Stream{}

func (s *Stream) Read(p []byte) (n int, err error) {
	utils.Assert(len(p) == 1, "can only read one byte at a time from Stream")
	if _eos(s) {
		return 0, io.EOF
	}
	p[0] = byte(_byte(s))
	return 1, nil
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

func _reset_2(s *Stream, pos int) {
	s.pos = pos
}

func _check_2(n int, s *Stream) {
	if _pos(s)+n > _len(s) {
		panic(EOS)
	}
}

func _skip_2(n int, s *Stream) {
	if n < 0 {
		panic(EOS)
	} else {
		_check_2(n, s)
		s.pos = s.pos + n
	}
}

func _read(s *Stream) int {
	return int(s.bytes[s.pos])
}

func _Some[T any](v T) *T {
	return &v
}

func _peek(s *Stream) *int {
	if _eos(s) {
		return nil
	} else {
		return _Some(_read(s))
	}
}

func _get(s *Stream) int {
	_check_2(1, s)
	b := _read(s)
	_skip_2(1, s)
	return b
}

func _get_string_2(n int, s *Stream) string {
	i := _pos(s)
	_skip_2(n, s)
	return s.bytes[i : i+n]
}
