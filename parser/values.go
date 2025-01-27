package parser

// Manual translation of the "Generic values" section

func _list_3[T any](f func(s *Stream) T, n OInt, s *Stream) []T {
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

func _either_2[T any](fs []func(s *Stream) T, s *Stream) T {
	if len(fs) == 0 {
		panic("`either` called with no options")
	}
	if len(fs) == 1 {
		return fs[0](s)
	}

	pos := _pos(s)
	res, exception := func() (res T, exc any) {
		defer func() {
			if r := recover(); r != nil {
				exc = r
			}
		}()
		res = fs[0](s)
		return
	}()
	if exception == nil {
		return res
	} else if _, ok := exception.(CodeError); ok {
		_reset_2(s, pos)
		return _either_2(fs[1:], s)
	} else {
		panic(exception)
	}
}
