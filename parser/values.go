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
