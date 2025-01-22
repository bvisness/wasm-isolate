package utils

import "fmt"

// Takes an (error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts. Use sparingly in real code.
func Must(err error) {
	if err != nil {
		panic(err)
	}
}

// Takes a (something, error) return and panics if there is an error.
// Helps avoid `if err != nil` in scripts. Use sparingly in real code.
func Must1[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

func Assert[T comparable](v T, msg string, args ...any) {
	var zero T
	if v == zero {
		panic(fmt.Sprintf("Assert failed: "+msg, args...))
	}
}
