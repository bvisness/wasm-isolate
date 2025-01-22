package utils

import "fmt"

// Takes an (error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts.
func Must[E comparableError](err E) {
	var zero E
	if err != zero {
		panic(err)
	}
}

// Takes a (something, error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts.
func Must1[T any, E comparableError](v T, err E) T {
	var zero E
	if err != zero {
		panic(err)
	}
	return v
}

// Takes a (something, something, error) return and panics if there is an
// error. Helps avoid `if err != nil` in scripts.
func Must2[T1 any, T2 any, E comparableError](v1 T1, v2 T2, err E) (T1, T2) {
	var zero E
	if err != zero {
		panic(err)
	}
	return v1, v2
}

func Or[T comparable](v T, vElse T) T {
	var zero T
	if v == zero {
		return vElse
	}
	return v
}

func Assert[T comparable](v T, msg string, args ...any) {
	var zero T
	if v == zero {
		panic(fmt.Sprintf("Assert failed: "+msg, args...))
	}
}

// We have this because otherwise passing a nil *SomeError through Must or
// Must1 will result in a non-nil interface value and a spurious panic.
type comparableError interface {
	comparable
	error
}
