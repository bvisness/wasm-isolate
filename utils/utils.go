package utils

import "fmt"

// Takes an (error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts.
func Must(err error) {
	if err != nil {
		panic(err)
	}
}

// Takes a (something, error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts.
func Must1[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

// Takes a (something, something, error) return and panics if there is an
// error. Helps avoid `if err != nil` in scripts.
func Must2[T1 any, T2 any](v1 T1, v2 T2, err error) (T1, T2) {
	if err != nil {
		panic(err)
	}
	return v1, v2
}

func Assert[T comparable](v T, msg string, args ...any) {
	var zero T
	if v == zero {
		panic(fmt.Sprintf("Assert failed: "+msg, args...))
	}
}
