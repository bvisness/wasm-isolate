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
	pos   OInt
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

func _len(s *Stream) OInt {
	return OInt(len(s.bytes))
}

func _pos(s *Stream) OInt {
	return s.pos
}

func _eos(s *Stream) bool {
	return _pos(s) == _len(s)
}

func _reset_2(s *Stream, pos OInt) {
	s.pos = pos
}

func _check_2(n OInt, s *Stream) {
	if _pos(s)+n > _len(s) {
		panic(EOS)
	}
}

func _skip_2(n OInt, s *Stream) {
	if n < 0 {
		panic(EOS)
	} else {
		_check_2(n, s)
		s.pos = s.pos + n
	}
}

func _read(s *Stream) OInt {
	return OInt(s.bytes[s.pos])
}

func _Some[T any](v T) *T {
	return &v
}

func _peek(s *Stream) *OInt {
	if _eos(s) {
		return nil
	} else {
		return _Some(_read(s))
	}
}

func _get(s *Stream) OInt {
	_check_2(1, s)
	b := _read(s)
	_skip_2(1, s)
	return b
}

func _get_string_2(n OInt, s *Stream) string {
	i := _pos(s)
	_skip_2(n, s)
	return s.bytes[i : i+n]
}
