package parser

func __derefIfNotNil[T any](p *T) T {
	if p != nil {
		return *p
	}
	var zero T
	return zero
}

// Void is defined as an empty variant type, so we follow that pattern here.

type OVoidKind int

type OVoid interface {
	Kind() OVoidKind
}

type OI32_t = OInt32
type OI64_t = OInt64
type OF32_t = float32
type OF64_t = float64

func Some_1[T any](v T) *T {
	return &v
}
